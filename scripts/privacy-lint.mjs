#!/usr/bin/env node
// Pinhole privacy lint — the static check required by CLAUDE.md ("Privacy tests")
// and docs/ARCHITECTURE.md §5. Node ≥ 18, no dependencies.
//
//   node scripts/privacy-lint.mjs              lint the repo (exit 1 on findings)
//   node scripts/privacy-lint.mjs --self-test  prove every rule fires on fixtures
//   node scripts/privacy-lint.mjs --root DIR   lint another checkout
//
// Rules (ids in brackets):
//   (a) [rust-log-prompt]   logging/printing macro in non-test Rust code whose
//                           arguments mention a prompt-ish identifier
//       [rust-dbg]          any dbg!() in non-test Rust code
//       [rust-error-prompt] CoreError constructors / .with_details() fed a prompt-ish identifier
//   (b) [forbidden-crate]   `log`/`tracing`/telemetry/updater crates in any Cargo.toml
//       [forbidden-npm]     telemetry/updater/log packages in package.json
//   (c) [prompt-type-write] a function that references a prompt-bearing type AND
//                           calls a file-writing API (best-effort heuristic)
//   (d) [console]           console.log/info/debug/warn/trace/dir/table in src/**
//       [console-error-prompt] console.error(...) with a prompt-ish identifier
//   (e) [storage-prompt]    localStorage/sessionStorage/indexedDB near prompt-ish identifiers
//       [autofill]          prompt-bearing <input> without autoComplete="off" (WebView2 autofill)
//   (f) [remote-asset]      http(s) assets in index.html / CSS / JSX attributes
//       [ts-network]        fetch("http…"), WebSocket, EventSource, sendBeacon, XMLHttpRequest in src/**
//   (g) [csp]               tauri.conf.json CSP allows remote origins / updater configured
//   (h) [bind-all]          "0.0.0.0" in non-test Rust code (engines bind 127.0.0.1 only)
//       [embed-metadata]    engine code sends img_gen without `embed_image_metadata: false`
//
// Suppress a single finding with a comment on the same line or the line above:
//   // privacy-lint: allow <reason>          (Rust / TS)
//   {/* privacy-lint: allow <reason> */}     (JSX)
// A reason is required.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

// ----------------------------------------------------------------------------
// Configuration — extend these lists as the code grows.
// ----------------------------------------------------------------------------

/** Rust types that carry prompt / negative-prompt / style text. */
export const PROMPT_TYPES = [
  "GenerateRequest",
  "FineTune",
  "FinalPrompt",
  "FinalPromptPreview",
  "ImgGenBody",
  "ImgGenRequest",
  "SdImgGenRequest",
  "SdGenRequest",
  "CaptionRequest",
  "PastedGenerationData",
];

/** Identifier parts (split on `_` and camelCase) that count as prompt-ish. */
const PROMPT_WORDS = new Set([
  "prompt", "prompts", "negative", "negatives", "positive",
  "caption", "captions", "instruction", "instructions", "pasted",
]);

/** Identifier/string parts that make browser storage suspicious. */
const STORAGE_WORDS = new Set([
  "prompt", "prompts", "negative", "negatives", "pasted", "paste", "caption", "captions", "instruction",
]);

/** Bare Rust macros that print / log / put values into panic output. */
const RUST_PRINT_MACROS = new Set([
  "println", "eprintln", "print", "eprint", "dbg",
  "info", "warn", "error", "debug", "trace", "event",
  "panic", "unreachable", "todo", "unimplemented",
  "assert_eq", "assert_ne", "debug_assert_eq", "debug_assert_ne",
]);

/** Paths whose macros are always logging macros (`log::info!`, `tracing::warn!`). */
const RUST_LOG_PATHS = /(^|::)(log|tracing|env_logger|slog)::$/;

/** Calls that write files. */
const RUST_WRITE_APIS = [
  /\bfs::write\s*\(/,
  /\bFile::create(_new)?\s*\(/,
  /\bFile::options\s*\(/,
  /\bOpenOptions::new\s*\(/,
  /\bto_writer(_pretty)?\s*\(/,
  /\bwrite_atomic\s*\(/,
  /\bNamedTempFile\b/,
  /\bpersist\s*\(/,
];

/** Rust crates that must never be direct dependencies. */
const FORBIDDEN_CRATES = [
  /^log$/, /^tracing(-.*)?$/, /^env_logger$/, /^pretty_env_logger$/, /^simplelog$/, /^fern$/,
  /^log4rs$/, /^slog(-.*)?$/, /^flexi_logger$/, /^tauri-plugin-log$/, /^sentry(-.*)?$/,
  /^opentelemetry(-.*)?$/, /^tauri-plugin-updater$/, /^tauri-plugin-aptabase$/,
  /^tauri-plugin-analytics$/, /^posthog(-.*)?$/, /^segment$/, /^minidump(-.*)?$/, /^crash-handler$/,
];

/** npm packages that must never be dependencies. */
const FORBIDDEN_NPM = [
  /^@tauri-apps\/plugin-updater$/, /^@tauri-apps\/plugin-log$/, /^@sentry\//, /^sentry/,
  /^posthog/, /^mixpanel/, /^@vercel\/analytics$/, /^@segment\//, /^analytics$/, /^amplitude/,
  /^@amplitude\//, /^react-ga/, /^@datadog\//, /^logrocket/, /^@bugsnag\//, /^bugsnag/,
  /^@google-analytics\//, /^ga-4-react$/, /^web-vitals$/, /^@aptabase\//,
];

const ALLOW_RE = /privacy-lint:\s*allow\s+\S/;

// ----------------------------------------------------------------------------
// Small helpers
// ----------------------------------------------------------------------------

const isIdentChar = (c) => c !== undefined && /[A-Za-z0-9_]/.test(c);

/** Split an identifier into lowercase words: `negativePrompt` → [negative, prompt]. */
export function identWords(ident) {
  return ident
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1_$2")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean);
}

function promptishIdents(text, words = PROMPT_WORDS) {
  const hits = new Set();
  for (const m of text.matchAll(/[A-Za-z_][A-Za-z0-9_]*/g)) {
    if (identWords(m[0]).some((w) => words.has(w))) hits.add(m[0]);
  }
  return [...hits];
}

function lineStarts(src) {
  const starts = [0];
  for (let i = 0; i < src.length; i++) if (src[i] === "\n") starts.push(i + 1);
  return starts;
}

function lineOf(starts, offset) {
  let lo = 0, hi = starts.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (starts[mid] <= offset) lo = mid; else hi = mid - 1;
  }
  return lo + 1; // 1-based
}

function blankRange(arr, a, b) {
  for (let k = a; k < b && k < arr.length; k++) if (arr[k] !== "\n" && arr[k] !== "\r") arr[k] = " ";
}

/** Given `open` index of ( [ or {, return index just past its matching close (masked code). */
function matchBracket(code, open) {
  const pairs = { "(": ")", "[": "]", "{": "}" };
  const stack = [pairs[code[open]]];
  let i = open + 1;
  while (i < code.length && stack.length) {
    const c = code[i];
    if (c === "(" || c === "[" || c === "{") stack.push(pairs[c]);
    else if (c === ")" || c === "]" || c === "}") stack.pop();
    i++;
  }
  return i;
}

// ----------------------------------------------------------------------------
// Rust masking: blank comments and string contents (offsets/newlines preserved)
// ----------------------------------------------------------------------------

export function maskRust(src) {
  const out = src.split("");
  const strings = [];
  const n = src.length;
  let i = 0;
  while (i < n) {
    const c = src[i], d = src[i + 1];
    if (c === "/" && d === "/") {
      let j = src.indexOf("\n", i);
      if (j < 0) j = n;
      blankRange(out, i, j);
      i = j;
      continue;
    }
    if (c === "/" && d === "*") {
      let depth = 1, j = i + 2;
      while (j < n && depth > 0) {
        if (src[j] === "/" && src[j + 1] === "*") { depth++; j += 2; }
        else if (src[j] === "*" && src[j + 1] === "/") { depth--; j += 2; }
        else j++;
      }
      blankRange(out, i, j);
      i = j;
      continue;
    }
    if ((c === "r" || (c === "b" && d === "r")) && !isIdentChar(src[i - 1])) {
      let j = c === "b" ? i + 2 : i + 1;
      let hashes = 0;
      while (src[j] === "#") { hashes++; j++; }
      if (src[j] === '"' && (hashes > 0 || j === (c === "b" ? i + 2 : i + 1))) {
        const close = '"' + "#".repeat(hashes);
        let end = src.indexOf(close, j + 1);
        if (end < 0) end = n;
        strings.push({ start: j + 1, end, text: src.slice(j + 1, end) });
        blankRange(out, j + 1, end);
        i = end + close.length;
        continue;
      }
    }
    if (c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== '"') j += src[j] === "\\" ? 2 : 1;
      strings.push({ start: i + 1, end: j, text: src.slice(i + 1, j) });
      blankRange(out, i + 1, j);
      i = j + 1;
      continue;
    }
    if (c === "'") {
      if (d === "\\") {
        let j = i + 2;
        while (j < n && src[j] !== "'" && src[j] !== "\n") j++;
        blankRange(out, i + 1, j);
        i = j + 1;
        continue;
      }
      const cp = src.codePointAt(i + 1);
      const len = cp !== undefined && cp > 0xffff ? 2 : 1;
      if (src[i + 1 + len] === "'") {
        blankRange(out, i + 1, i + 1 + len);
        i += 2 + len;
        continue;
      }
      i++; // lifetime
      continue;
    }
    i++;
  }
  return { code: out.join(""), strings };
}

/** Blank `#[cfg(test)]` / `#[test]` / test-util items so rules only see shipped code. */
export function stripRustTestCode(src, code) {
  if (/#!\[\s*cfg\s*\(\s*(test|any\s*\(\s*test|feature\s*=\s*"test-util")/.test(src)) return code.replace(/[^\n]/g, " ");
  const out = code.split("");
  const attr = /#\[\s*(cfg\s*\(|test\s*\]|tokio::test|rstest)/g;
  let m;
  while ((m = attr.exec(code))) {
    const start = m.index;
    const open = code.indexOf("[", start);
    const attrEnd = matchBracket(code, open);
    const raw = src.slice(start, attrEnd);
    const isTest =
      /#\[\s*(test|tokio::test|rstest)\b/.test(raw) ||
      ((/[(,\s]test\s*[),]/.test(raw) || /feature\s*=\s*"test-util"/.test(raw)) && !/not\s*\(\s*test/.test(raw));
    if (!isTest) continue;
    // Skip further attributes, then find the item's end.
    let j = attrEnd;
    for (;;) {
      while (j < code.length && /\s/.test(code[j])) j++;
      if (code[j] === "#" && code[j + 1] === "[") { j = matchBracket(code, j + 1); continue; }
      break;
    }
    let depth = 0, end = code.length;
    for (let k = j; k < code.length; k++) {
      const c = code[k];
      if (c === "(" || c === "[") depth++;
      else if (c === ")" || c === "]") depth--;
      else if (depth === 0 && c === ";") { end = k + 1; break; }
      else if (depth === 0 && c === "{") { end = matchBracket(code, k); break; }
    }
    blankRange(out, start, end);
    attr.lastIndex = end;
  }
  return out.join("");
}

// ----------------------------------------------------------------------------
// TS/JS masking (comments + string contents; template `${}` stays as code)
// ----------------------------------------------------------------------------

export function maskTs(src) {
  const code = src.split("");      // comments + strings blanked
  const withStr = src.split("");   // comments blanked only
  const n = src.length;
  const tplStack = []; // brace depth at which a template expression started
  let braceDepth = 0;
  let i = 0;
  let lastSig = ""; // last significant (non-space) char, for regex detection
  const startTemplate = (from) => {
    // scan template text from `from` (just after ` or }) until ` or ${
    let j = from;
    while (j < n) {
      if (src[j] === "\\") { j += 2; continue; }
      if (src[j] === "`") { blankRange(code, from, j); return { end: j + 1, expr: false }; }
      if (src[j] === "$" && src[j + 1] === "{") { blankRange(code, from, j); return { end: j + 2, expr: true }; }
      j++;
    }
    blankRange(code, from, n);
    return { end: n, expr: false };
  };
  while (i < n) {
    const c = src[i], d = src[i + 1];
    if (c === "/" && d === "/") {
      let j = src.indexOf("\n", i);
      if (j < 0) j = n;
      blankRange(code, i, j); blankRange(withStr, i, j);
      i = j; continue;
    }
    if (c === "/" && d === "*") {
      let j = src.indexOf("*/", i + 2);
      j = j < 0 ? n : j + 2;
      blankRange(code, i, j); blankRange(withStr, i, j);
      i = j; continue;
    }
    if (c === "/" && (lastSig === "" || "(,=:[!&|?{};+-*%<>~^".includes(lastSig))) {
      // regex literal (heuristic): up to an unescaped / on the same line
      let j = i + 1, inClass = false, ok = false;
      while (j < n && src[j] !== "\n") {
        if (src[j] === "\\") { j += 2; continue; }
        if (src[j] === "[") inClass = true;
        else if (src[j] === "]") inClass = false;
        else if (src[j] === "/" && !inClass) { ok = true; break; }
        j++;
      }
      if (ok) {
        blankRange(code, i + 1, j);
        i = j + 1; lastSig = "/"; continue;
      }
    }
    if (c === "'" || c === '"') {
      let j = i + 1;
      while (j < n && src[j] !== c && src[j] !== "\n") j += src[j] === "\\" ? 2 : 1;
      if (src[j] === c) {
        blankRange(code, i + 1, j);
        i = j + 1; lastSig = c; continue;
      }
      i++; continue; // unterminated on this line: JSX text apostrophe etc.
    }
    if (c === "`") {
      const r = startTemplate(i + 1);
      if (r.expr) tplStack.push(braceDepth);
      i = r.end; lastSig = "`"; continue;
    }
    if (c === "{") braceDepth++;
    if (c === "}") {
      if (tplStack.length && tplStack[tplStack.length - 1] === braceDepth) {
        tplStack.pop();
        const r = startTemplate(i + 1);
        if (r.expr) tplStack.push(braceDepth);
        i = r.end; lastSig = "`"; continue;
      }
      braceDepth--;
    }
    if (!/\s/.test(c)) lastSig = c;
    if (/[A-Za-z0-9_$]/.test(c)) {
      // consume identifier; `return /re/` is rare enough to ignore
      let j = i;
      while (j < n && /[A-Za-z0-9_$]/.test(src[j])) j++;
      lastSig = "a"; i = j; continue;
    }
    i++;
  }
  return { code: code.join(""), withStr: withStr.join("") };
}

// ----------------------------------------------------------------------------
// Findings
// ----------------------------------------------------------------------------

class Findings {
  constructor() { this.items = []; }
  add(file, src, starts, offset, rule, message, extraAllowOffsets = []) {
    const lines = src.split("\n");
    const candidates = [offset, ...extraAllowOffsets].map((o) => lineOf(starts, o));
    for (const ln of candidates) {
      if (ALLOW_RE.test(lines[ln - 1] || "") || ALLOW_RE.test(lines[ln - 2] || "")) return;
    }
    const line = candidates[0];
    if (this.items.some((it) => it.file === file && it.line === line && it.rule === rule)) return;
    this.items.push({ file, line, rule, message, snippet: (lines[line - 1] || "").trim().slice(0, 160) });
  }
  addPlain(file, line, rule, message, snippet = "") {
    this.items.push({ file, line, rule, message, snippet });
  }
}

// ----------------------------------------------------------------------------
// Rust rules
// ----------------------------------------------------------------------------

const RUST_SCOPE = /^src-tauri\/(src|crates\/[^/]+\/src)\//;
const RUST_TEST_PATH = /(^|\/)(tests?|benches|examples|testutil|test_util)(\/|$)|(^|\/)(tests?|testutil|test_util|testing|[a-z0-9_]+_tests?)\.rs$/;

export function lintRust(file, src, F) {
  const masked = maskRust(src);
  const code = stripRustTestCode(src, masked.code);
  const strings = masked.strings.filter((s) => code.slice(s.start - 1, s.start) !== " " || code[s.end] === '"');
  const starts = lineStarts(src);
  const stringsIn = (a, b) => masked.strings.filter((s) => s.start >= a && s.end <= b && /[^ \n]/.test(code.slice(a, b)));

  // (a) logging / printing macros
  const macroRe = /(?<![A-Za-z0-9_:])((?:[A-Za-z_][A-Za-z0-9_]*::)*)([A-Za-z_][A-Za-z0-9_]*)!\s*([([{])/g;
  let m;
  while ((m = macroRe.exec(code))) {
    const pathPrefix = m[1], name = m[2];
    if (name === "macro_rules") continue;
    const isLogPath = RUST_LOG_PATHS.test(pathPrefix);
    const open = m.index + m[0].length - 1;
    const close = matchBracket(code, open);
    // write!/writeln! count only when aimed at stdout/stderr
    const isStdWrite = !pathPrefix && (name === "write" || name === "writeln") && /\b(stderr|stdout)\b/.test(code.slice(open, close));
    if (!isLogPath && !isStdWrite && (pathPrefix || !RUST_PRINT_MACROS.has(name))) continue;
    if (name === "dbg" && !pathPrefix) {
      F.add(file, src, starts, m.index, "rust-dbg", "dbg!() is not allowed in shipped code (it prints values to stderr).");
      continue;
    }
    const args = code.slice(open + 1, close - 1);
    const caps = [];
    for (const s of stringsIn(open, close)) {
      for (const c of s.text.matchAll(/(?<!\{)\{([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)/g)) caps.push(c[1]);
    }
    const hits = promptishIdents(args + " " + caps.join(" "));
    if (hits.length) {
      F.add(file, src, starts, m.index, "rust-log-prompt",
        `${pathPrefix}${name}!() receives prompt-ish value(s) ${hits.map((h) => `\`${h}\``).join(", ")} — prompts must never reach logs, stderr or panic output.`);
    }
  }

  // (a2) CoreError built from prompt-ish values
  const errRe = /(\bCoreError::(?:new|invalid|internal|not_found)|\.with_details)\s*\(/g;
  while ((m = errRe.exec(code))) {
    const open = m.index + m[0].length - 1;
    const close = matchBracket(code, open);
    const args = code.slice(open + 1, close - 1);
    const caps = [];
    for (const s of stringsIn(open, close)) {
      for (const c of s.text.matchAll(/(?<!\{)\{([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*)/g)) caps.push(c[1]);
    }
    const hits = promptishIdents(args + " " + caps.join(" "));
    if (hits.length) {
      F.add(file, src, starts, m.index, "rust-error-prompt",
        `${m[1]}() receives prompt-ish value(s) ${hits.map((h) => `\`${h}\``).join(", ")} — errors are shown/copied and must never contain prompt text.`);
    }
  }

  // (c) prompt-bearing type + file-writing API in the same function
  const typeRes = PROMPT_TYPES.map((t) => [t, new RegExp(`\\b${t}\\b`)]);
  const impls = [];
  const implRe = /\bimpl\b/g;
  while ((m = implRe.exec(code))) {
    let k = m.index + 4, depth = 0;
    while (k < code.length) {
      const c = code[k];
      if (c === "<") depth++;
      else if (c === ">") depth--;
      else if (c === ";" && depth <= 0) { k = -1; break; }
      else if (c === "{" && depth <= 0) break;
      k++;
    }
    if (k < 0 || k >= code.length) continue;
    const header = code.slice(m.index, k);
    const types = typeRes.filter(([, re]) => re.test(header)).map(([t]) => t);
    if (types.length) impls.push({ start: k, end: matchBracket(code, k), types });
  }
  const fnRe = /\bfn\s+([A-Za-z_][A-Za-z0-9_]*)/g;
  while ((m = fnRe.exec(code))) {
    let k = m.index, depth = 0, body = -1;
    for (; k < code.length; k++) {
      const c = code[k];
      if (c === "(" || c === "[") depth++;
      else if (c === ")" || c === "]") depth--;
      else if (depth === 0 && c === ";") break;
      else if (depth === 0 && c === "{") { body = k; break; }
    }
    if (body < 0) continue;
    const end = matchBracket(code, body);
    const text = code.slice(m.index, end);
    const types = new Set(typeRes.filter(([, re]) => re.test(text)).map(([t]) => t));
    for (const im of impls) if (m.index > im.start && m.index < im.end) im.types.forEach((t) => types.add(t));
    if (!types.size) continue;
    const writes = [];
    for (const re of RUST_WRITE_APIS) {
      const g = new RegExp(re.source, "g");
      let w;
      while ((w = g.exec(text))) writes.push({ api: w[0].replace(/\s*\($/, ""), offset: m.index + w.index });
    }
    if (!writes.length) continue;
    F.add(file, src, starts, m.index, "prompt-type-write",
      `fn ${m[1]}() references prompt-bearing type(s) ${[...types].join(", ")} and writes files (${[...new Set(writes.map((w) => w.api))].join(", ")}). ` +
      `Prompt data may only be serialized into the loopback engine request. Split the function or add "// privacy-lint: allow <reason>".`,
      writes.map((w) => w.offset));
  }

  // (h) bind-all
  for (const s of strings) {
    if (/\b0\.0\.0\.0\b/.test(s.text) && code[s.start - 1] === '"') {
      F.add(file, src, starts, s.start, "bind-all", 'Found "0.0.0.0": engines must bind to 127.0.0.1 only (CLAUDE.md privacy rule 6).');
    }
  }
  return code;
}

function lintEngineMetadata(files, F) {
  const engine = files.filter((f) => f.rel.startsWith("src-tauri/crates/pinhole-engine/src/") && f.rel.endsWith(".rs") && !RUST_TEST_PATH.test(f.rel.slice("src-tauri/crates/pinhole-engine/src/".length)));
  let sendsImgGen = null;
  let mentionsFalse = false;
  let sawTrue = false;
  for (const f of engine) {
    const masked = maskRust(f.content);
    const code = stripRustTestCode(f.content, masked.code);
    const live = masked.strings.filter((s) => /[^ \n]/.test(code.slice(Math.max(0, s.start - 1), s.start)));
    if (!sendsImgGen && live.some((s) => /img_gen/.test(s.text))) sendsImgGen = f;
    const starts = lineStarts(f.content);
    // `embed_image_metadata: true` / `= true` / json!({"embed_image_metadata": true})
    const re = /embed_image_metadata"?\s*[:=]\s*true\b/g;
    let m;
    const unmaskedLive = f.content.split("").map((ch, idx) => (code[idx] === " " && /\S/.test(ch) && !live.some((s) => idx >= s.start && idx < s.end) ? " " : ch)).join("");
    while ((m = re.exec(unmaskedLive))) {
      sawTrue = true;
      F.add(f.rel, f.content, starts, m.index, "embed-metadata", "embed_image_metadata must always be false (sd-server defaults to true and would bake the prompt into the PNG).");
    }
    if (/embed_image_metadata"?\s*[:=]\s*false\b/.test(unmaskedLive) || /embed_image_metadata\s*:\s*bool/.test(code)) mentionsFalse = true;
  }
  if (sendsImgGen && !mentionsFalse && !sawTrue) {
    F.addPlain(sendsImgGen.rel, 1, "embed-metadata", 'pinhole-engine talks to /sdcpp/v1/img_gen but never sets "embed_image_metadata": false (CLAUDE.md privacy rule 2).');
  }
}

// ----------------------------------------------------------------------------
// Manifest rules
// ----------------------------------------------------------------------------

export function lintCargoToml(file, src, F) {
  let section = "";
  const lines = src.split("\n");
  lines.forEach((raw, idx) => {
    const line = raw.replace(/#.*$/, "").trim();
    const h = line.match(/^\[([^\]]+)\]$/);
    if (h) {
      section = h[1].trim();
      const t = section.match(/^(?:workspace\.)?(?:target\.[^.]+(?:\.[^.]+)*\.)?(?:dependencies|build-dependencies)\.([A-Za-z0-9_-]+)$/);
      if (t && FORBIDDEN_CRATES.some((re) => re.test(t[1])) && !ALLOW_RE.test(raw)) {
        F.addPlain(file, idx + 1, "forbidden-crate", `Crate \`${t[1]}\` is not allowed (no logging/tracing/telemetry/updater crates — CLAUDE.md privacy rules 1 and 4).`, raw.trim());
      }
      return;
    }
    const isDeps = /^(workspace\.)?(dependencies|build-dependencies)$/.test(section) || /^target\..*\.(dependencies|build-dependencies)$/.test(section);
    if (!isDeps) return;
    const d = line.match(/^([A-Za-z0-9_-]+)\s*=/);
    if (!d) return;
    const pkg = (line.match(/package\s*=\s*"([^"]+)"/) || [])[1];
    for (const name of [d[1], pkg].filter(Boolean)) {
      if (FORBIDDEN_CRATES.some((re) => re.test(name)) && !ALLOW_RE.test(raw)) {
        F.addPlain(file, idx + 1, "forbidden-crate", `Crate \`${name}\` is not allowed (no logging/tracing/telemetry/updater crates — CLAUDE.md privacy rules 1 and 4).`, raw.trim());
      }
    }
  });
}

export function lintPackageJson(file, src, F) {
  let pkg;
  try { pkg = JSON.parse(src); } catch { return; }
  const lines = src.split("\n");
  for (const key of ["dependencies", "devDependencies", "optionalDependencies", "peerDependencies"]) {
    for (const name of Object.keys(pkg[key] || {})) {
      if (FORBIDDEN_NPM.some((re) => re.test(name))) {
        const ln = lines.findIndex((l) => l.includes(`"${name}"`)) + 1;
        F.addPlain(file, ln, "forbidden-npm", `Package \`${name}\` is not allowed (no telemetry, analytics, crash reporting or update checks — CLAUDE.md privacy rule 4).`, (lines[ln - 1] || "").trim());
      }
    }
  }
}

export function lintTauriConf(file, src, F) {
  let conf;
  try { conf = JSON.parse(src); } catch (e) { F.addPlain(file, 1, "csp", `tauri.conf.json is not valid JSON: ${e.message}`); return; }
  const lines = src.split("\n");
  const lineOfText = (t) => Math.max(1, lines.findIndex((l) => l.includes(t)) + 1);
  const cspRaw = conf?.app?.security?.csp;
  const cspList = typeof cspRaw === "string" ? [cspRaw] : cspRaw && typeof cspRaw === "object" ? Object.entries(cspRaw).map(([k, v]) => `${k} ${Array.isArray(v) ? v.join(" ") : v}`) : [];
  if (!cspList.length) F.addPlain(file, lineOfText('"security"'), "csp", "No CSP configured: the WebView must be locked to 'self', ipc: and blob:/data: (CLAUDE.md privacy rule 5).");
  for (const csp of cspList) {
    for (const tok of csp.split(/[\s;]+/)) {
      if (!tok) continue;
      const remote = /^(https?|wss?):\/\/(?!ipc\.localhost(?:[:/]|$))/i.test(tok) || /^(https?|wss?):$/i.test(tok) || tok === "*" || /^\*\./.test(tok);
      if (remote) F.addPlain(file, lineOfText("csp"), "csp", `CSP allows remote source \`${tok}\`: the WebView makes no network calls of its own (CLAUDE.md privacy rule 5).`);
    }
  }
  if (conf?.plugins?.updater || conf?.bundle?.createUpdaterArtifacts) {
    F.addPlain(file, lineOfText("updater"), "csp", "Updater configured: no update checks allowed (CLAUDE.md privacy rule 4).");
  }
}

// ----------------------------------------------------------------------------
// Frontend rules
// ----------------------------------------------------------------------------

const TS_TEST_PATH = /(\.(test|spec)\.[cm]?[jt]sx?$)|(^|\/)(__tests__|__mocks__|test|tests)\//;
const TS_SETUP_PATH = /(^|\/)(setupTests|test-setup|vitest\.setup)\.[cm]?[jt]sx?$/;

export function lintTs(file, src, F) {
  const { code, withStr } = maskTs(src);
  const starts = lineStarts(src);
  let m;

  // (d) console.*
  const conRe = /\bconsole\s*\.\s*(log|info|debug|warn|trace|dir|dirxml|table|group|groupCollapsed|count|time|timeLog|assert|error)\s*\(/g;
  while ((m = conRe.exec(code))) {
    if (m[1] !== "error") {
      F.add(file, src, starts, m.index, "console", `console.${m[1]}() is not allowed in src/ (it can leak prompts to the WebView console / logs). Use console.error with a CoreError code only.`);
      continue;
    }
    const open = m.index + m[0].length - 1;
    const close = matchBracket(code, open);
    const hits = promptishIdents(code.slice(open + 1, close - 1));
    if (hits.length) {
      F.add(file, src, starts, m.index, "console-error-prompt", `console.error() receives prompt-ish value(s) ${hits.map((h) => `\`${h}\``).join(", ")} — only log a CoreError code.`);
    }
  }

  // (e) browser storage near prompt-ish identifiers: the storage line itself,
  // the call's arguments, and the declarations of identifiers passed to it
  // (`const draft = {prompt}; localStorage.setItem(k, JSON.stringify(draft))`).
  const stRe = /\b(localStorage|sessionStorage|indexedDB)\b/g;
  const wsLines = withStr.split("\n");
  while ((m = stRe.exec(code))) {
    const ln = lineOf(starts, m.index);
    const parts = [wsLines[ln - 1] || ""];
    const call = /^\s*(?:\.\s*[A-Za-z_$][\w$]*\s*)?([(\[])/.exec(code.slice(m.index + m[1].length));
    if (call) {
      const open = m.index + m[1].length + call[0].length - 1;
      const close = matchBracket(code, open);
      parts.push(withStr.slice(open, close));
      for (const id of new Set(code.slice(open, close).match(/[A-Za-z_$][\w$]*/g) || [])) {
        const declRe = new RegExp(`\\b(?:const|let|var)\\s+${id.replace(/\$/g, "\\$")}\\b`, "g");
        let dm, last = null;
        const from = starts[Math.max(0, ln - 16)];
        declRe.lastIndex = from;
        while ((dm = declRe.exec(code)) && dm.index < m.index) last = dm;
        if (last) {
          const semi = code.indexOf(";", last.index);
          const nl = code.indexOf("\n", last.index);
          const brace = /=\s*[\[{(]/.exec(code.slice(last.index, nl < 0 ? undefined : nl));
          const end = brace ? matchBracket(code, last.index + brace.index + brace[0].length - 1) : semi >= 0 && (nl < 0 || semi < nl) ? semi : nl;
          parts.push(withStr.slice(last.index, end));
        }
      }
    }
    const hits = promptishIdents(parts.join("\n"), STORAGE_WORDS);
    if (hits.length) {
      F.add(file, src, starts, m.index, "storage-prompt", `${m[1]} used near prompt-ish ${hits.map((h) => `\`${h}\``).join(", ")} — prompts must never be persisted in the WebView (CLAUDE.md privacy rule 1).`);
    }
  }

  // [autofill] WebView2/Chromium form autofill can save <input> values to its
  // on-disk profile: prompt-bearing inputs must opt out.
  const inputRe = /<input\b/g;
  while ((m = inputRe.exec(code))) {
    let k = m.index + 6, depth = 0;
    for (; k < code.length; k++) {
      const c = code[k];
      if (c === "{") depth++;
      else if (c === "}") depth--;
      else if (c === ">" && depth <= 0) break;
    }
    const tag = withStr.slice(m.index, k + 1);
    if (/\btype\s*=\s*["'{]?\s*["']?(checkbox|radio|range|file|hidden|button|submit|color)\b/.test(tag)) continue;
    const hits = promptishIdents(tag, STORAGE_WORDS);
    if (hits.length && !/autoComplete\s*=\s*(\{\s*)?["'`]off["'`]/i.test(tag)) {
      F.add(file, src, starts, m.index, "autofill", `<input> bound to prompt-ish ${hits.map((h) => `\`${h}\``).join(", ")} without autoComplete="off" — WebView2 autofill may store it on disk. Use a <textarea> or add autoComplete="off".`);
    }
  }

  // (f) remote assets in JSX attributes / inline styles
  const attrRe = /\b(src|href|poster|srcSet|srcset|data|action|formAction)\s*=\s*\{?\s*["'`]\s*(https?:)?\/\//g;
  while ((m = attrRe.exec(withStr))) {
    F.add(file, src, starts, m.index, "remote-asset", `JSX ${m[1]}= points to a remote URL — the WebView must not load or navigate to remote content. Fetch bytes via Rust (blob:) or open links with the opener plugin.`);
  }
  const urlRe = /url\(\s*["'`]?\s*(https?:)?\/\//g;
  while ((m = urlRe.exec(withStr))) {
    F.add(file, src, starts, m.index, "remote-asset", "CSS url() points to a remote URL — bundle the asset locally (no CDNs, no remote fonts).");
  }

  // [ts-network] direct network APIs
  const netRe = /\b(fetch\s*\(\s*["'`]\s*(?:https?:|wss?:)?\/\/|new\s+WebSocket\b|new\s+EventSource\b|sendBeacon\s*\(|new\s+XMLHttpRequest\b|importScripts\s*\(\s*["'`]https?:)/g;
  while ((m = netRe.exec(withStr))) {
    F.add(file, src, starts, m.index, "ts-network", `\`${m[1].replace(/\s+/g, " ").slice(0, 40)}\` — the WebView makes no network calls; everything goes through the Rust HttpClient (CLAUDE.md privacy rule 5).`);
  }
}

export function lintHtmlCss(file, src, F) {
  const starts = lineStarts(src);
  // strip comments but keep offsets
  const noComments = src.replace(/<!--[\s\S]*?-->|\/\*[\s\S]*?\*\//g, (s) => s.replace(/[^\n]/g, " "));
  const res = [
    [/<link\b[^>]*\bhref\s*=\s*["']?\s*(https?:)?\/\//gi, "<link href> to a remote URL"],
    [/<script\b[^>]*\bsrc\s*=\s*["']?\s*(https?:)?\/\//gi, "<script src> from a remote URL"],
    [/<(img|iframe|video|audio|source|embed|object)\b[^>]*\b(src|data)\s*=\s*["']?\s*(https?:)?\/\//gi, "remote media URL"],
    [/url\(\s*["']?\s*(https?:)?\/\//gi, "CSS url() to a remote URL"],
    [/@import\s+(url\()?\s*["']?\s*(https?:)?\/\//gi, "@import of a remote stylesheet"],
  ];
  for (const [re, what] of res) {
    let m;
    while ((m = re.exec(noComments))) {
      F.add(file, src, starts, m.index, "remote-asset", `${what} — no CDN assets or remote fonts (CLAUDE.md privacy rule 4). Bundle it locally.`);
    }
  }
}

// ----------------------------------------------------------------------------
// Driver
// ----------------------------------------------------------------------------

export function lintFiles(files) {
  const F = new Findings();
  for (const f of files) {
    const rel = f.rel.replace(/\\/g, "/");
    if (rel.endsWith(".rs") && RUST_SCOPE.test(rel)) {
      const inner = rel.replace(RUST_SCOPE, "");
      if (!RUST_TEST_PATH.test(inner)) lintRust(rel, f.content, F);
    } else if (/(^|\/)Cargo\.toml$/.test(rel)) {
      lintCargoToml(rel, f.content, F);
    } else if (rel === "package.json") {
      lintPackageJson(rel, f.content, F);
    } else if (/(^|\/)tauri(\.[a-z]+)?\.conf\.json$/.test(rel)) {
      lintTauriConf(rel, f.content, F);
    } else if (/^src\/.*\.[cm]?[jt]sx?$/.test(rel)) {
      if (!TS_TEST_PATH.test(rel.slice(4)) && !TS_SETUP_PATH.test(rel)) lintTs(rel, f.content, F);
    } else if (rel === "index.html" || /^(src|public)\/.*\.(html?|css)$/.test(rel)) {
      lintHtmlCss(rel, f.content, F);
    }
  }
  lintEngineMetadata(files.map((f) => ({ ...f, rel: f.rel.replace(/\\/g, "/") })), F);
  return F.items;
}

const SKIP_DIRS = new Set(["node_modules", "target", ".git", "dist", "gen", ".smoke-cache", "Data"]);

function walk(root, dir, acc) {
  let entries;
  try { entries = fs.readdirSync(dir, { withFileTypes: true }); } catch { return; }
  for (const e of entries) {
    const abs = path.join(dir, e.name);
    const rel = path.relative(root, abs).split(path.sep).join("/");
    if (e.isDirectory()) {
      if (SKIP_DIRS.has(e.name)) continue;
      walk(root, abs, acc);
    } else if (e.isFile() && /\.(rs|toml|json|[cm]?[jt]sx?|html?|css)$/.test(e.name)) {
      if (e.name.endsWith(".json") && !/^(package\.json|tauri(\.[a-z]+)?\.conf\.json)$/.test(e.name)) continue;
      acc.push({ rel, content: fs.readFileSync(abs, "utf8") });
    }
  }
}

export function lintTree(root) {
  const files = [];
  for (const top of ["src-tauri", "src", "public", "tests"]) walk(root, path.join(root, top), files);
  for (const single of ["index.html", "package.json", "Cargo.toml"]) {
    const abs = path.join(root, single);
    if (fs.existsSync(abs)) files.push({ rel: single, content: fs.readFileSync(abs, "utf8") });
  }
  // tests/ is walked only for its Cargo.toml (test code may use sentinels freely)
  return { files: files.length, findings: lintFiles(files.filter((f) => !f.rel.startsWith("tests/") || f.rel === "tests/Cargo.toml")) };
}

function report(findings) {
  if (!findings.length) return;
  const byRule = {};
  for (const f of findings) (byRule[f.rule] ||= []).push(f);
  for (const f of findings) {
    console.error(`${f.file}:${f.line}: [${f.rule}] ${f.message}`);
    if (f.snippet) console.error(`    > ${f.snippet}`);
  }
  console.error(`\nprivacy-lint: ${findings.length} finding(s): ${Object.entries(byRule).map(([r, l]) => `${r}×${l.length}`).join(", ")}`);
  console.error('Fix them, or (only if it is genuinely safe) add "// privacy-lint: allow <reason>" on the line.');
}

// ----------------------------------------------------------------------------
// Self-test fixtures: every rule must fire on "bad" and stay quiet on "good".
// ----------------------------------------------------------------------------

const FIXTURES = [
  // ---- (a) Rust logging
  { rel: "src-tauri/crates/pinhole-core/src/bad_log.rs", expect: ["rust-log-prompt", "rust-log-prompt", "rust-log-prompt", "rust-log-prompt", "rust-log-prompt", "rust-log-prompt", "rust-dbg"], content: `
pub fn run(req: &GenerateRequestLike) {
    println!("sending {}", req.prompt);
    eprintln!("neg={negative_prompt}");
    log::info!("final: {:?}", final_prompt);
    tracing::debug!(?req.negative, "x");
    panic!("bad {}", req.prompt);
    let _ = dbg!(1 + 1);
    writeln!(std::io::stderr(), "{}", final_prompt.prompt).ok();
    writeln!(out_file, "{}", seed).ok();
}
` },
  { rel: "src-tauri/crates/pinhole-core/src/good_log.rs", expect: [], content: `
//! Docs may mention the prompt freely: println!("{}", prompt) in a comment is fine.
/// Returns the prompt length. eprintln!("{prompt}") in docs is fine too.
pub fn run(req: &Req) {
    println!("engine started on port {}", port);          // no prompt identifiers
    eprintln!("Type a prompt first");                       // literal text only
    let s = "println!(\\"{}\\", prompt)";                  // inside a string
    let r = r#"dbg!(prompt)"#;
    todo!("store agent");
    eprintln!("{}", req.prompt.len()); // privacy-lint: allow length only, no text
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() { println!("{}", prompt); dbg!(prompt); }
}
#[cfg(feature = "test-util")]
pub fn helper() { println!("{}", prompt); }
` },
  { rel: "src-tauri/crates/pinhole-engine/src/testutil.rs", expect: [], content: `pub fn x() { println!("{}", prompt); dbg!(1); }` },
  // ---- (a2) CoreError
  { rel: "src-tauri/crates/pinhole-core/src/bad_err.rs", expect: ["rust-error-prompt", "rust-error-prompt"], content: `
fn f(req: &R) -> CoreError {
    let e = CoreError::invalid(format!("bad prompt: {}", req.prompt));
    CoreError::new("engine_failed", "x").with_details(final_prompt.negative.clone().unwrap())
}
fn ok() -> CoreError { CoreError::invalid("Type a prompt first") }
` },
  // ---- (b) forbidden crates / npm
  { rel: "src-tauri/crates/pinhole-x/Cargo.toml", expect: ["forbidden-crate", "forbidden-crate", "forbidden-crate", "forbidden-crate"], content: `
[package]
name = "x"
[dependencies]
serde = "1"
log = "0.4"
tracing-subscriber = { version = "0.3" }
mylog = { package = "log", version = "0.4" }
[target.'cfg(windows)'.dependencies]
sentry = "0.34"
[dev-dependencies]
tracing = "0.1"
` },
  { rel: "package.json", expect: ["forbidden-npm", "forbidden-npm"], content: `{
  "dependencies": { "react": "^19", "@sentry/react": "^8" },
  "devDependencies": { "@tauri-apps/plugin-updater": "^2", "vite": "^8" }
}` },
  // ---- (c) prompt type + file writes
  { rel: "src-tauri/crates/pinhole-core/src/bad_write.rs", expect: ["prompt-type-write", "prompt-type-write", "prompt-type-write"], content: `
pub fn remember(req: &GenerateRequest, dir: &Path) {
    let s = serde_json::to_string(req).unwrap();
    std::fs::write(dir.join("last.json"), s).unwrap();
}
pub fn dump(p: &FinalPrompt) -> Result<()> {
    let f = File::create("x.yaml")?;
    serde_yaml::to_writer(f, p)?;
    Ok(())
}
impl ImgGenBody {
    pub fn save(&self) { pinhole_store::write_atomic(Path::new("b"), b"x").unwrap(); }
}
` },
  { rel: "src-tauri/crates/pinhole-core/src/good_write.rs", expect: [], content: `
pub fn save_image(core: &AppCore, id: &str) -> CoreResult<SavedImage> {
    std::fs::write(path, bytes)?; // no prompt types here
    Ok(saved)
}
pub async fn generate(core: &AppCore, req: GenerateRequest) -> CoreResult<()> {
    let body = ImgGenBody::from(&req);
    core.local.post_json(&base, "/sdcpp/v1/img_gen", &body)?;
    Ok(())
}
impl From<&FineTune> for PresetFineTune { fn from(f: &FineTune) -> Self { todo!() } }
pub fn allowed(req: &GenerateRequest) {
    // privacy-lint: allow writes only the seed, never the request
    std::fs::write("seed.txt", req.seed.to_string()).unwrap();
}
` },
  // ---- (h) bind-all + embed metadata
  { rel: "src-tauri/crates/pinhole-engine/src/process.rs", expect: ["bind-all"], content: `
pub fn args() -> Vec<&'static str> { vec!["--listen-ip", "0.0.0.0"] }
// "0.0.0.0" in a comment is fine
` },
  { rel: "src-tauri/crates/pinhole-engine/src/sdapi.rs", expect: ["embed-metadata"], content: `
pub fn body(p: &str) -> serde_json::Value {
    let url = "/sdcpp/v1/img_gen";
    serde_json::json!({ "prompt": p, "embed_image_metadata": true })
}
` },
  // ---- (d)(e)(f) frontend
  { rel: "src/tabs/create/Bad.tsx", expect: ["console", "console", "console-error-prompt", "storage-prompt", "storage-prompt", "remote-asset", "remote-asset", "ts-network", "ts-network", "autofill"], content: `
export function Bad({ prompt }: { prompt: string }) {
  console.log("render");
  console.warn(\`x\`);
  console.error("failed", prompt);
  localStorage.setItem("lastPrompt", "x");
  const draft = { negativePrompt: neg };
  sessionStorage.setItem("d", JSON.stringify(draft));
  fetch("https://civitai.com/api/v1/models");
  const ws = new WebSocket(url);
  const field = <input value={negativePrompt} onChange={(e) => setNeg(e.target.value)} />;
  return <div style={{ backgroundImage: "url(https://cdn.example.com/a.png)" }}>
    <img src="https://cdn.example.com/x.png" /></div>;
}
` },
  { rel: "src/tabs/create/Good.tsx", expect: [], content: `
// console.log("prompt") in a comment is fine
/* localStorage.setItem("prompt", prompt) */
export function Good({ prompt }: { prompt: string }) {
  const theme = localStorage.getItem("theme");
  try { run(); } catch (e) { console.error("generate failed", (e as CoreError).code); }
  const text = "console.log(prompt)";
  const re = /["']/g;
  const t = \`Don't \${count} "worry"\`;
  const blobUrl = URL.createObjectURL(blob);
  const r = fetch(blobUrl);
  const a = <input value={negativePrompt} autoComplete="off" onChange={(e) => { if (x > 1) set(e.target.value); }} />;
  const b = <input type="checkbox" checked={usePromptPrefix} />;
  const c = <input value={seed} />;
  return <p>Don't worry, we never store it. <img src={blobUrl} /></p>;
}
` },
  { rel: "src/lib/paste/parse.test.ts", expect: [], content: `console.log(prompt); localStorage.setItem("prompt", prompt);` },
  { rel: "index.html", expect: ["remote-asset", "remote-asset"], content: `<!doctype html><html><head>
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Inter">
<script src="https://cdn.example.com/x.js"></script>
<!-- <script src="https://commented.out/x.js"></script> -->
<script type="module" src="/src/main.tsx"></script></head></html>` },
  { rel: "src/index.css", expect: ["remote-asset", "remote-asset"], content: `@import url("https://fonts.googleapis.com/css2?family=Inter");
@import "tailwindcss";
.a { background: url(https://cdn.example.com/bg.png); }
.b { background: url(/local.png); }
/* url(https://in-a-comment.example.com) */` },
  // ---- (g) CSP
  { rel: "src-tauri/tauri.conf.json", expect: ["csp", "csp"], content: JSON.stringify({ app: { security: { csp: "default-src 'self'; connect-src ipc: http://ipc.localhost https://civitai.com; img-src *" } }, plugins: {} }, null, 2) },
];

const GOOD_CONF = { rel: "src-tauri/tauri.conf.json", content: JSON.stringify({ app: { security: { csp: "default-src 'self'; img-src 'self' blob: data:; connect-src ipc: http://ipc.localhost" } } }) };

function selfTest() {
  let failed = 0;
  const check = (name, got, want) => {
    const g = [...got].sort().join(","), w = [...want].sort().join(",");
    if (g !== w) { failed++; console.error(`✗ ${name}\n    expected [${w}]\n    got      [${g}]`); }
    else console.log(`✓ ${name} → [${g || "clean"}]`);
  };
  for (const fx of FIXTURES) {
    const found = lintFiles([fx]).filter((f) => f.file === fx.rel);
    check(fx.rel, found.map((f) => f.rule), fx.expect);
    if (found.length && fx.expect.length === 0) for (const f of found) console.error(`    ${f.file}:${f.line} [${f.rule}] ${f.message}`);
  }
  check("good tauri.conf.json", lintFiles([GOOD_CONF]).map((f) => f.rule), []);
  // engine that sends img_gen without the flag at all
  check("engine without embed_image_metadata", lintFiles([{ rel: "src-tauri/crates/pinhole-engine/src/api.rs", content: 'const P: &str = "/sdcpp/v1/img_gen";' }]).map((f) => f.rule), ["embed-metadata"]);
  check("engine with embed_image_metadata: false", lintFiles([{ rel: "src-tauri/crates/pinhole-engine/src/api.rs", content: 'const P: &str = "/sdcpp/v1/img_gen";\nstruct B { embed_image_metadata: bool }\nfn b() -> B { B { embed_image_metadata: false } }' }]).map((f) => f.rule), []);
  // identWords sanity
  const iw = identWords("negativePromptText").join(",");
  if (iw !== "negative,prompt,text") { failed++; console.error(`✗ identWords → ${iw}`); } else console.log("✓ identWords");
  if (failed) { console.error(`\nprivacy-lint self-test: ${failed} failure(s)`); process.exit(1); }
  console.log("\nprivacy-lint self-test: all rules fire on bad fixtures and stay quiet on good ones.");
}

// ----------------------------------------------------------------------------

const isMain = process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) {
  const args = process.argv.slice(2);
  if (args.includes("--self-test")) {
    selfTest();
  } else {
    const rootIdx = args.indexOf("--root");
    const root = rootIdx >= 0 ? path.resolve(args[rootIdx + 1]) : path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
    const { files, findings } = lintTree(root);
    if (findings.length) {
      report(findings);
      process.exit(1);
    }
    console.log(`privacy-lint: OK (${files} files checked)`);
  }
}
