#!/usr/bin/env node
// Verify every pinned URL / hash in config/models.yaml and config/engine.yaml
// against the real servers, WITHOUT downloading multi-GB files where possible,
// and gather the facts needed to fill the remaining TODOs.
//
//   node scripts/verify-pins.mjs [--config-dir config] [--out pins-report.json]
//                                [--only urls,civitai,releases] [--dry-run]
//
// Needs the `yaml` package: either resolvable normally, or installed into the
// directory named by $PINHOLE_TOOLS_DIR (`npm i --prefix "$PINHOLE_TOOLS_DIR" yaml@2`).
// Optional env: GITHUB_TOKEN (GitHub REST API), CIVITAI_API_KEY.
// Never sends a Hugging Face token: pins must be downloadable anonymously,
// exactly like Pinhole itself downloads them.
//
// How each URL is checked:
//   huggingface.co/<repo>/resolve/<rev>/<path>  → POST /api/models/<repo>/paths-info/<rev>
//       (lfs.oid = SHA-256, size); fallback: HEAD with X-Linked-Etag / X-Linked-Size;
//       also reports the commit behind <rev> (pin it!) and whether the repo is gated.
//   github.com/<o>/<r>/releases/download/<tag>/<name> → GET /repos/<o>/<r>/releases/tags/<tag>
//       (asset exists, size, `digest: sha256:…`); no digest → streamed download + hash.
//   anything else → streamed download + hash.
// Plus: CivitAI BaseModel enum check, CivitAI candidates for TODO `recommended`
// entries, and the latest engine releases with the assets Pinhole needs.
//
// Output: Markdown on stdout and in $GITHUB_STEP_SUMMARY; JSON in --out.
// Exit 1 only on hard failures (missing file/asset, gated/unauthorized, 404,
// SHA-256 mismatch against a non-TODO pin). TODOs are reported, not failed.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const UA = "pinhole-verify-pins (+https://github.com/)";

// Engine assets Pinhole needs, per upstream repo. Extend when engine.yaml grows.
const ENGINE_ASSETS = {
  "leejet/stable-diffusion.cpp": {
    windows_cpu: /^sd-.*-bin-win-cpu-x64\.zip$/,
    windows_cuda: /^sd-.*-bin-win-cuda12-x64\.zip$/,
    windows_cudart: /^cudart-sd-bin-win-cu12-x64\.zip$/,
    windows_vulkan: /^sd-.*-bin-win-vulkan-x64\.zip$/,
    linux_cpu: /^sd-.*-bin-Linux-Ubuntu-[\d.]+-x86_64\.zip$/,
    linux_vulkan: /^sd-.*-bin-Linux-Ubuntu-[\d.]+-x86_64-vulkan\.zip$/,
  },
  // Pinhole's fork: upstream code + the sd-server lock-down patch (engine/sd-cpp/).
  "DavidGudovic/stable-diffusion.cpp": {
    windows_cpu: /^sd-.*-bin-win-cpu-x64\.zip$/,
    windows_cuda: /^sd-.*-bin-win-cuda12-x64\.zip$/,
    windows_cudart: /^cudart-sd-bin-win-cu12-x64\.zip$/,
    windows_vulkan: /^sd-.*-bin-win-vulkan-x64\.zip$/,
    linux_cpu: /^sd-.*-bin-Linux-Ubuntu-[\d.]+-x86_64-cpu\.zip$/,
    linux_cuda: /^sd-.*-bin-Linux-Ubuntu-[\d.]+-x86_64-cuda12\.zip$/,
    linux_cudart: /^cudart-sd-bin-Linux-Ubuntu-[\d.]+-x86_64-cu12\.zip$/,
    linux_vulkan: /^sd-.*-bin-Linux-Ubuntu-[\d.]+-x86_64-vulkan\.zip$/,
  },
  "ggml-org/llama.cpp": {
    windows_cpu: /^llama-.*-bin-win-cpu-x64\.zip$/,
    windows_cuda: /^llama-.*-bin-win-cuda-?12[\d.]*-x64\.zip$/,
    windows_cudart: /^cudart-llama-bin-win-cuda-?12[\d.]*-x64\.zip$/,
    windows_vulkan: /^llama-.*-bin-win-vulkan-x64\.zip$/,
    linux_cpu: /^llama-.*-bin-ubuntu-x64\.(zip|tar\.gz)$/,
    linux_vulkan: /^llama-.*-bin-ubuntu-vulkan-x64\.(zip|tar\.gz)$/,
  },
};

// CivitAI queries for `recommended` entries whose ids are still TODO.
const CIVITAI_ROLE_QUERIES = {
  realistic: [{ tag: "photorealistic" }, { query: "realistic" }],
  anime: [{}],
};

// ----------------------------------------------------------------------------
// args / utils
// ----------------------------------------------------------------------------

const argv = process.argv.slice(2);
const argVal = (name, def) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : def;
};
const CONFIG_DIR = path.resolve(argVal("--config-dir", path.join(ROOT, "config")));
const OUT = path.resolve(argVal("--out", "pins-report.json"));
const ONLY = new Set(argVal("--only", "urls,civitai,releases").split(","));
const DRY = argv.includes("--dry-run");

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const isTodo = (v) => v === undefined || v === null || v === "" || /^todo$/i.test(String(v).trim());
const isSha = (v) => typeof v === "string" && /^[0-9a-f]{64}$/i.test(v.trim());
const mb = (bytes) => (bytes == null ? "?" : (bytes / 1048576).toFixed(bytes < 10 * 1048576 ? 2 : 0));
const short = (h) => (h ? `${h.slice(0, 12)}…` : "");
const md = (s) => String(s ?? "").replace(/\|/g, "\\|").replace(/\n/g, " ");

async function loadYamlLib() {
  try {
    return await import("yaml");
  } catch {
    const dir = process.env.PINHOLE_TOOLS_DIR;
    if (dir) {
      const req = createRequire(path.join(path.resolve(dir), "noop.js"));
      return await import(pathToFileURL(req.resolve("yaml")).href);
    }
    console.error("The `yaml` package is required: npm i --prefix \"$PINHOLE_TOOLS_DIR\" yaml@2 (and set PINHOLE_TOOLS_DIR)");
    process.exit(2);
  }
}

async function http(url, opts = {}, { retries = 3, timeoutMs = 60_000 } = {}) {
  const headers = { "User-Agent": UA, ...(opts.headers || {}) };
  for (let attempt = 0; ; attempt++) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    try {
      const res = await fetch(url, { ...opts, headers, signal: ctrl.signal });
      if ((res.status >= 500 || res.status === 429) && attempt < retries) {
        await res.body?.cancel?.();
        const wait = Number(res.headers.get("retry-after")) * 1000 || 2000 * 2 ** attempt;
        await sleep(Math.min(wait, 30_000));
        continue;
      }
      return res;
    } catch (e) {
      if (attempt < retries) { await sleep(2000 * 2 ** attempt); continue; }
      throw e;
    } finally {
      clearTimeout(timer);
    }
  }
}

function ghHeaders() {
  const h = { Accept: "application/vnd.github+json", "X-GitHub-Api-Version": "2022-11-28" };
  if (process.env.GITHUB_TOKEN) h.Authorization = `Bearer ${process.env.GITHUB_TOKEN}`;
  return h;
}

function civitaiHeaders() {
  return process.env.CIVITAI_API_KEY ? { Authorization: `Bearer ${process.env.CIVITAI_API_KEY}` } : {};
}

async function getJson(url, headers = {}) {
  const res = await http(url, { headers: { Accept: "application/json", ...headers } });
  const text = await res.text();
  let json = null;
  try { json = JSON.parse(text); } catch { /* not json */ }
  return { status: res.status, ok: res.ok, json, text: json ? "" : text.slice(0, 300), headers: res.headers };
}

/** Streamed download + SHA-256 (nothing written to disk). */
async function downloadHash(url, headers = {}) {
  const res = await http(url, { headers, redirect: "follow" }, { retries: 2, timeoutMs: 3 * 3600_000 });
  if (!res.ok) { await res.body?.cancel?.(); return { status: res.status }; }
  const hash = crypto.createHash("sha256");
  let size = 0;
  for await (const chunk of res.body) { hash.update(chunk); size += chunk.length; }
  return { status: res.status, sha256: hash.digest("hex"), size };
}

// ----------------------------------------------------------------------------
// Collect pins from YAML
// ----------------------------------------------------------------------------

/** Walk YAML data; every object with a `url` (or `*_url`) string is a pin. */
function collectPins(data, file, vars = {}) {
  const pins = [];
  const walk = (node, where, scopeVars) => {
    if (Array.isArray(node)) { node.forEach((v, i) => walk(v, `${where}[${i}]`, scopeVars)); return; }
    if (!node || typeof node !== "object") return;
    const localVars = { ...scopeVars };
    for (const k of ["version", "tag", "release"]) if (typeof node[k] === "string" && !isTodo(node[k])) localVars[k] = node[k];
    for (const [k, v] of Object.entries(node)) {
      if ((k === "url" || k.endsWith("_url")) && typeof v === "string") {
        let url = v.trim();
        url = url.replace(/\$?\{(version|tag|release|short)\}/g, (m, name) => {
          if (name === "short" && localVars.version) return localVars.version.split("-").pop();
          return localVars[name] ?? m;
        });
        pins.push({
          file, where: k === "url" ? where : `${where}.${k}`, url: isTodo(url) ? null : url,
          sha256: k === "url" ? node.sha256 : node[k.replace(/_url$/, "_sha256")],
          size_mb: node.size_mb, name: node.file,
          template: /\{[a-z_]+\}/.test(url),
        });
      }
    }
    for (const [k, v] of Object.entries(node)) if (v && typeof v === "object") walk(v, where ? `${where}.${k}` : k, localVars);
  };
  walk(data, "", vars);
  return pins;
}

function collectBaseModels(models) {
  const out = new Map(); // string → [where]
  const walk = (node, where) => {
    if (Array.isArray(node)) { node.forEach((v, i) => walk(v, `${where}[${i}]`)); return; }
    if (!node || typeof node !== "object") return;
    for (const [k, v] of Object.entries(node)) {
      if (k === "civitai_base_models" && Array.isArray(v)) {
        for (const s of v) if (typeof s === "string") out.set(s, [...(out.get(s) || []), where]);
      } else if (v && typeof v === "object") walk(v, where ? `${where}.${k}` : k);
    }
  };
  walk(models, "");
  return out;
}

function collectRepos(engine) {
  const repos = [];
  const walk = (node, where) => {
    if (!node || typeof node !== "object") return;
    for (const [k, v] of Object.entries(node)) {
      if (k === "repo" && typeof v === "string") {
        const m = v.match(/github\.com\/([^/]+\/[^/#?]+?)(?:\.git)?\/?$/);
        if (m) repos.push({ repo: m[1], where, version: node.version });
      } else if (v && typeof v === "object") walk(v, where ? `${where}.${k}` : k);
    }
  };
  walk(engine, "");
  return repos;
}

// ----------------------------------------------------------------------------
// URL checks
// ----------------------------------------------------------------------------

const HF_RE = /^https:\/\/huggingface\.co\/(?:(datasets|spaces)\/)?([^/]+\/[^/]+)\/resolve\/([^/]+)\/([^?#]+)/;
const GH_RE = /^https:\/\/github\.com\/([^/]+)\/([^/]+)\/releases\/(?:download\/([^/]+)|latest\/download)\/([^?#]+)$/;

async function checkHuggingFace(entries) {
  // group by repo@rev
  const groups = new Map();
  for (const e of entries) {
    const m = e.url.match(HF_RE);
    const kind = m[1] || "models";
    const repo = m[2];
    const rev = decodeURIComponent(m[3]);
    const p = m[4].split("/").map(decodeURIComponent).join("/");
    const key = `${kind}|${repo}|${rev}`;
    if (!groups.has(key)) groups.set(key, { kind, repo, rev, items: [] });
    groups.get(key).items.push({ e, path: p });
  }
  const repoInfo = [];
  for (const g of groups.values()) {
    const api = `https://huggingface.co/api/${g.kind}/${g.repo}`;
    // revision → commit + gated
    let commit = null, gated = null;
    const rv = await getJson(`${api}/revision/${encodeURIComponent(g.rev)}`);
    if (rv.ok && rv.json) { commit = rv.json.sha || null; gated = rv.json.gated ?? null; }
    repoInfo.push({ repo: g.repo, rev: g.rev, commit, gated, status: rv.status });
    // paths-info (JSON body, as huggingface.js does)
    let infos = null;
    try {
      const res = await http(`${api}/paths-info/${encodeURIComponent(g.rev)}`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: JSON.stringify({ paths: g.items.map((i) => i.path), expand: false }),
      });
      if (res.ok) infos = await res.json();
      else await res.body?.cancel?.();
    } catch { /* fall back below */ }
    for (const { e, path: p } of g.items) {
      const r = { source: "huggingface", commit, gated };
      const info = Array.isArray(infos) ? infos.find((x) => x.path === p) : null;
      if (info) {
        r.status = 200;
        r.size = info.lfs?.size ?? info.size;
        r.sha256 = info.lfs?.oid ?? null;
        if (!r.sha256) r.note = "not an LFS file (hash via download)";
      } else {
        // HEAD fallback: X-Linked-Etag carries the LFS SHA-256
        const res = await http(e.url, { method: "HEAD", redirect: "manual" });
        r.status = res.status;
        const linked = res.headers.get("x-linked-etag");
        const size = res.headers.get("x-linked-size") || res.headers.get("content-length");
        if (res.status < 400) {
          r.status = 200;
          r.size = size ? Number(size) : null;
          r.sha256 = linked ? linked.replace(/^W\//, "").replace(/"/g, "") : null;
          if (!isSha(r.sha256)) { r.sha256 = null; r.note = "no LFS hash in headers (hash via download)"; }
        }
      }
      if (r.status === 200 && !r.sha256) {
        const d = await downloadHash(e.url);
        r.status = d.status; r.sha256 = d.sha256; r.size = d.size ?? r.size;
      }
      if (gated && gated !== false) {
        r.hard = true;
        r.note = `repo is gated (${gated}) — users cannot download it anonymously`;
      }
      if (r.status === 401 || r.status === 403) { r.hard = true; r.note = r.note || "unauthorized — gated or private repo"; }
      if (r.status === 404) { r.hard = true; r.note = "file not found at this revision"; }
      e.result = r;
    }
  }
  return repoInfo;
}

async function checkGithubAssets(entries) {
  const releases = new Map();
  for (const e of entries) {
    const m = e.url.match(GH_RE);
    const [, owner, repo, tag, nameEnc] = m;
    const name = decodeURIComponent(nameEnc);
    const key = `${owner}/${repo}@${tag || "latest"}`;
    if (!releases.has(key)) {
      const url = tag ? `https://api.github.com/repos/${owner}/${repo}/releases/tags/${encodeURIComponent(tag)}` : `https://api.github.com/repos/${owner}/${repo}/releases/latest`;
      releases.set(key, await getJson(url, ghHeaders()));
    }
    const rel = releases.get(key);
    const r = { source: "github-release", status: rel.status };
    if (!tag) r.warn = "uses /releases/latest — pin a tag";
    if (!rel.ok) {
      r.hard = rel.status === 404;
      r.note = rel.status === 404 ? `release tag ${tag} not found` : `GitHub API HTTP ${rel.status}${rel.json?.message ? `: ${rel.json.message}` : ""}`;
    } else {
      const asset = (rel.json.assets || []).find((a) => a.name === name);
      if (!asset) {
        r.status = 404; r.hard = true;
        r.note = `asset not in release ${rel.json.tag_name}`;
      } else {
        r.status = 200;
        r.size = asset.size;
        if (asset.digest && /^sha256:/i.test(asset.digest)) r.sha256 = asset.digest.slice(7).toLowerCase();
        else {
          const d = await downloadHash(asset.browser_download_url);
          r.status = d.status; r.sha256 = d.sha256; r.size = d.size ?? r.size;
          r.note = "no API digest — hashed by download";
        }
      }
    }
    e.result = r;
  }
}

async function checkOther(entries) {
  for (const e of entries) {
    try {
      const d = await downloadHash(e.url);
      e.result = { source: "download", status: d.status, sha256: d.sha256, size: d.size };
      if (d.status === 404 || d.status === 410) { e.result.hard = true; e.result.note = "not found"; }
      else if (d.status >= 400) { e.result.note = `HTTP ${d.status}`; e.result.hard = d.status === 401 || d.status === 403; }
    } catch (err) {
      e.result = { source: "download", status: 0, note: `network error: ${err.message}` };
    }
  }
}

function finalize(e) {
  const r = e.result || { status: 0, note: "not checked" };
  const exp = isSha(e.sha256) ? e.sha256.trim().toLowerCase() : null;
  r.expected_sha256 = exp;
  r.expected_size_mb = e.size_mb ?? null;
  if (exp && r.sha256 && exp !== r.sha256.toLowerCase()) {
    r.hard = true;
    r.note = `SHA-256 MISMATCH (pinned ${short(exp)}, server ${short(r.sha256)})`;
  }
  if (!isTodo(e.sha256) && !exp) r.warn = `sha256 value is neither TODO nor 64 hex chars: ${e.sha256}`;
  if (r.size && e.size_mb && Math.abs(r.size / 1048576 - e.size_mb) / e.size_mb > 0.1) {
    r.warn = [r.warn, `size_mb ${e.size_mb} ≠ actual ${mb(r.size)} MB`].filter(Boolean).join("; ");
  }
  r.ok = r.status === 200 && !r.hard && !!r.sha256;
  r.todo = !exp;
  e.result = r;
}

// ----------------------------------------------------------------------------
// CivitAI
// ----------------------------------------------------------------------------

async function civitaiEnums(baseModels) {
  const res = await getJson("https://civitai.com/api/v1/enums", civitaiHeaders());
  const out = { status: res.status, enum: null, unknown: [], known: [] };
  if (!res.ok || !res.json) return out;
  const key = Object.keys(res.json).find((k) => /^basemodels?$/i.test(k));
  const list = key ? res.json[key] : null;
  if (!Array.isArray(list)) { out.note = `no BaseModel key in response (keys: ${Object.keys(res.json).join(", ")})`; return out; }
  out.enum = list;
  const norm = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, "");
  for (const [s, where] of baseModels) {
    if (list.includes(s)) { out.known.push(s); continue; }
    const n = norm(s);
    const tokens = s.toLowerCase().split(/[^a-z0-9]+/).filter((t) => t.length > 1);
    const suggestions = list.filter((x) => norm(x) === n || norm(x).includes(n) || n.includes(norm(x)) || tokens.some((t) => x.toLowerCase().includes(t))).slice(0, 12);
    out.unknown.push({ value: s, where, suggestions });
  }
  return out;
}

function pickVersion(model, baseModel) {
  const versions = model.modelVersions || [];
  return versions.find((v) => (!baseModel || v.baseModel === baseModel) && !isEarlyAccess(v)) || versions[0];
}

function isEarlyAccess(v) {
  if (v.availability && v.availability !== "Public") return true;
  if (v.earlyAccessEndsAt && new Date(v.earlyAccessEndsAt) > new Date()) return true;
  return false;
}

function describeModel(model, baseModel) {
  const v = pickVersion(model, baseModel) || {};
  const files = v.files || [];
  const f = files.find((x) => x.primary) || files[0] || {};
  const format = f.metadata?.format || (f.name?.endsWith(".safetensors") ? "SafeTensor" : f.name?.split(".").pop());
  const commercial = Array.isArray(model.allowCommercialUse) ? model.allowCommercialUse.join(",") : String(model.allowCommercialUse ?? "");
  const d = {
    modelId: model.id, name: model.name, nsfw: !!model.nsfw,
    versionId: v.id, versionName: v.name, baseModel: v.baseModel, earlyAccess: isEarlyAccess(v),
    file: f.name, format, fp: f.metadata?.fp, sizeMB: f.sizeKB ? Math.round(f.sizeKB / 1024) : null,
    sha256: f.hashes?.SHA256?.toLowerCase() || null, pickle: f.pickleScanResult, virus: f.virusScanResult,
    thumbsUp: model.stats?.thumbsUpCount, downloads: model.stats?.downloadCount, commercial,
    downloadUrl: f.downloadUrl,
  };
  // The API sometimes returns models whose versions are for other base models: require a match.
  d.baseMatch = !baseModel || d.baseModel === baseModel;
  d.eligible = d.baseMatch && format === "SafeTensor" && d.pickle === "Success" && d.virus === "Success" && !d.earlyAccess && !d.nsfw;
  return d;
}

/** Verify recommended entries that already pin CivitAI ids (GET /api/v1/model-versions/<id>). */
async function civitaiPinned(models) {
  const out = [];
  const families = models.families || {};
  const basesFor = (fam) => {
    let f = families[fam];
    for (let i = 0; f && i < 5; i++) {
      if (Array.isArray(f.civitai_base_models) && f.civitai_base_models.length) return f.civitai_base_models;
      f = f.inherits ? families[f.inherits] : null;
    }
    return [];
  };
  for (const [role, list] of Object.entries(models.recommended || {})) {
    for (const [idx, cand] of (list || []).entries()) {
      if (cand?.source !== "civitai" || isTodo(cand.civitai_version_id)) continue;
      const res = await getJson(`https://civitai.com/api/v1/model-versions/${cand.civitai_version_id}`, civitaiHeaders());
      const e = { role, index: idx, family: cand.family, title: cand.title, yamlSizeMB: cand.size_mb ?? null, modelId: cand.civitai_model_id, versionId: cand.civitai_version_id, status: res.status, problems: [], warnings: [] };
      if (!res.ok || !res.json) {
        e.hard = res.status === 404 || res.status === 410;
        e.problems.push(`HTTP ${res.status}`);
      } else {
        const v = res.json;
        const files = v.files || [];
        const f = files.find((x) => x.primary) || files[0] || {};
        e.versionName = v.name; e.modelName = v.model?.name; e.baseModel = v.baseModel;
        e.file = f.name; e.format = f.metadata?.format; e.sizeMB = f.sizeKB ? Math.round(f.sizeKB / 1024) : null;
        e.sha256 = f.hashes?.SHA256?.toLowerCase() || null; e.pickle = f.pickleScanResult; e.virus = f.virusScanResult;
        e.earlyAccess = isEarlyAccess(v); e.nsfw = !!v.model?.nsfw;
        if (!isTodo(cand.civitai_model_id) && Number(v.modelId) !== Number(cand.civitai_model_id)) { e.hard = true; e.problems.push(`version belongs to model ${v.modelId}, not ${cand.civitai_model_id}`); }
        if (!["SafeTensor", "GGUF"].includes(e.format)) { e.hard = true; e.problems.push(`primary file format ${e.format} (only SafeTensor/GGUF allowed)`); }
        if (e.pickle !== "Success" || e.virus !== "Success") { e.hard = true; e.problems.push(`scans pickle=${e.pickle} virus=${e.virus}`); }
        if (e.earlyAccess) { e.hard = true; e.problems.push("version is early access (paid)"); }
        const bases = basesFor(cand.family);
        if (bases.length && !bases.includes(v.baseModel)) e.warnings.push(`baseModel "${v.baseModel}" not in family ${cand.family} civitai_base_models [${bases.join(", ")}]`);
        if (cand.size_mb && e.sizeMB && Math.abs(e.sizeMB - cand.size_mb) / cand.size_mb > 0.1) e.warnings.push(`size_mb ${cand.size_mb} ≠ actual ${e.sizeMB}`);
        if (e.nsfw) e.warnings.push("model is flagged NSFW");
      }
      out.push(e);
      await sleep(300);
    }
  }
  return out;
}

async function civitaiCandidates(models) {
  const out = [];
  const families = models.families || {};
  const baseFor = (fam) => {
    let f = families[fam];
    for (let i = 0; f && i < 5; i++) {
      if (Array.isArray(f.civitai_base_models) && f.civitai_base_models.length) return f.civitai_base_models[0];
      f = f.inherits ? families[f.inherits] : null;
    }
    return null;
  };
  for (const [role, list] of Object.entries(models.recommended || {})) {
    for (const [idx, cand] of (list || []).entries()) {
      if (cand?.source !== "civitai") continue;
      if (!isTodo(cand.civitai_model_id) && !isTodo(cand.civitai_version_id)) continue;
      const baseModel = baseFor(cand.family) || "SDXL 1.0";
      const queries = CIVITAI_ROLE_QUERIES[role] || [{}];
      const seen = new Map();
      const errors = [];
      for (const extra of queries) {
        for (const sort of ["Highest Rated", "Most Downloaded"]) {
          const qs = new URLSearchParams({ types: "Checkpoint", sort, period: "AllTime", limit: "5", nsfw: "false", primaryFileOnly: "true" });
          qs.append("baseModels", baseModel);
          for (const [k, v] of Object.entries(extra)) qs.set(k, v);
          const url = `https://civitai.com/api/v1/models?${qs}`;
          const res = await getJson(url, civitaiHeaders());
          if (!res.ok || !res.json) { errors.push(`${sort} ${JSON.stringify(extra)} → HTTP ${res.status} ${res.text || ""}`); continue; }
          for (const m of res.json.items || []) if (!seen.has(m.id)) seen.set(m.id, describeModel(m, baseModel));
          await sleep(500);
        }
      }
      out.push({ role, index: idx, family: cand.family, baseModel, candidates: [...seen.values()].slice(0, 10), errors });
    }
  }
  return out;
}

// ----------------------------------------------------------------------------
// Engine releases
// ----------------------------------------------------------------------------

async function engineReleases(repos) {
  const out = [];
  const seen = new Set();
  for (const { repo, where, version } of repos) {
    if (seen.has(repo)) continue;
    seen.add(repo);
    const res = await getJson(`https://api.github.com/repos/${repo}/releases?per_page=10`, ghHeaders());
    const want = ENGINE_ASSETS[repo] || {};
    const entry = { repo, where, pinned: version, status: res.status, releases: [], best: null, note: res.ok ? null : res.json?.message || res.text };
    if (res.ok && Array.isArray(res.json)) {
      for (const rel of res.json) {
        const assets = rel.assets || [];
        const found = {};
        for (const [kind, re] of Object.entries(want)) {
          const a = assets.find((x) => re.test(x.name));
          found[kind] = a ? { name: a.name, size: a.size, sha256: a.digest?.startsWith("sha256:") ? a.digest.slice(7) : null, url: a.browser_download_url } : null;
        }
        const r = { tag: rel.tag_name, published: rel.published_at, prerelease: rel.prerelease, draft: rel.draft, found, allAssets: assets.map((a) => a.name) };
        r.complete = Object.keys(want).length > 0 && Object.values(found).every(Boolean);
        entry.releases.push(r);
      }
      entry.best = entry.releases.find((r) => r.complete && !r.draft) || null;
    }
    out.push(entry);
  }
  return out;
}

// ----------------------------------------------------------------------------
// Report
// ----------------------------------------------------------------------------

function renderMarkdown(rep) {
  const L = [];
  const p = (s = "") => L.push(s);
  p("# Pinhole pin verification");
  p();
  p(`Generated ${rep.generated_at} · config: \`${path.relative(ROOT, CONFIG_DIR) || "."}\``);
  p();
  const urls = rep.entries;
  if (urls) {
    const hard = urls.filter((e) => e.result?.hard);
    const ok = urls.filter((e) => e.result?.ok && !e.result?.todo);
    const todoFill = urls.filter((e) => e.result?.ok && e.result?.todo);
    const noUrl = rep.todo_urls;
    p("## Summary");
    p();
    p(`- **${ok.length}** pinned hashes verified`);
    p(`- **${todoFill.length}** files reachable with hash still \`TODO\` (paste-ready values below)`);
    p(`- **${hard.length}** hard failures${hard.length ? " ❌" : ""}`);
    p(`- **${noUrl.length}** pins with \`url: TODO\``);
    p(`- **${urls.filter((e) => e.result?.warn).length}** warnings`);
    p();
    if (hard.length) {
      p("### ❌ Hard failures");
      p();
      for (const e of hard) p(`- \`${e.file}\` → \`${e.where}\`: ${e.result.note || `HTTP ${e.result.status}`} — ${e.url}`);
      p();
    }
    p("## URLs");
    p();
    p("| | where | status | size MB (yaml → actual) | sha256 (server) | pinned | note |");
    p("|---|---|---|---|---|---|---|");
    for (const e of urls) {
      const r = e.result || {};
      const icon = r.hard ? "❌" : r.ok ? (r.todo ? "🟡" : "✅") : "⚠️";
      p(`| ${icon} | \`${md(e.file)}:${md(e.where)}\` | ${r.status ?? ""} | ${e.size_mb ?? "?"} → ${mb(r.size)} | \`${r.sha256 || ""}\` | ${r.expected_sha256 ? "`" + short(r.expected_sha256) + "`" : "TODO"} | ${md([r.note, r.warn].filter(Boolean).join("; "))} |`);
    }
    p();
    if (noUrl.length) {
      p("### Pins with `url: TODO`");
      p();
      for (const e of noUrl) p(`- \`${e.file}:${e.where}\``);
      p();
    }
    if (todoFill.length) {
      p("### Paste-ready values for TODO hashes");
      p();
      p("```yaml");
      for (const e of todoFill) p(`# ${e.file} → ${e.where}\nurl: ${e.url}\nsha256: ${e.result.sha256}\nsize_mb: ${Math.max(1, Math.round(e.result.size / 1048576))}`);
      p("```");
      p();
    }
    if (rep.hf_repos?.length) {
      p("### Hugging Face revisions (pin `resolve/<commit>/…` instead of `main`)");
      p();
      p("| repo | rev | commit | gated |");
      p("|---|---|---|---|");
      for (const r of rep.hf_repos) p(`| ${r.repo} | ${r.rev} | \`${r.commit || `? (HTTP ${r.status})`}\` | ${r.gated === false ? "no" : r.gated ?? "?"} |`);
      p();
    }
  }
  if (rep.civitai_enums) {
    const c = rep.civitai_enums;
    p("## CivitAI `baseModel` strings");
    p();
    if (!c.enum) p(`⚠️ Could not read https://civitai.com/api/v1/enums (HTTP ${c.status}) ${c.note || ""}`);
    else {
      p(`${c.known.length} known: ${c.known.map((s) => `\`${s}\``).join(", ")}`);
      p();
      if (c.unknown.length) {
        p("**Unknown to the live API (fix `civitai_base_models`):**");
        p();
        for (const u of c.unknown) p(`- \`${u.value}\` (in ${u.where.join(", ")}) → candidates: ${u.suggestions.map((s) => `\`${s}\``).join(", ") || "none"}`);
      } else p("All `civitai_base_models` strings exist in the live enum ✅");
      p();
      p("<details><summary>Full live BaseModel enum</summary>");
      p();
      p(c.enum.map((s) => `\`${s}\``).join(", "));
      p();
      p("</details>");
    }
    p();
  }
  if (rep.civitai_pinned?.length) {
    p("## CivitAI `recommended` pins");
    p();
    p("| | role | model / version (ids) | base | file | fmt | MB (yaml → actual) | sha256 | scans | problems / warnings |");
    p("|---|---|---|---|---|---|---|---|---|---|");
    for (const e of rep.civitai_pinned) {
      p(`| ${e.hard ? "❌" : e.warnings.length || e.problems.length ? "⚠️" : "✅"} | ${e.role}[${e.index}] | ${md(e.modelName ?? e.title)} / ${md(e.versionName ?? "")} (${e.modelId}/${e.versionId}) | ${md(e.baseModel ?? "")} | ${md(e.file ?? "")} | ${e.format ?? ""} | ${e.yamlSizeMB ?? "?"} → ${e.sizeMB ?? "?"} | \`${e.sha256 || ""}\` | ${e.pickle ?? "?"}/${e.virus ?? "?"} | ${md([...e.problems, ...e.warnings].join("; "))} |`);
    }
    p();
  }
  if (rep.civitai_candidates) {
    p("## CivitAI candidates for TODO `recommended` entries");
    p();
    if (!rep.civitai_candidates.length) { p("None — every CivitAI `recommended` entry already has ids (verified above)."); p(); }
    for (const g of rep.civitai_candidates) {
      p(`### ${g.role}[${g.index}] — family \`${g.family}\`, baseModel \`${g.baseModel}\``);
      p();
      if (g.errors.length) p(`⚠️ ${g.errors.join("; ")}`);
      p("| ok | model (id) | version (id) | base | file | fmt/fp | MB | sha256 | scans | 👍 / ⬇ | commercial |");
      p("|---|---|---|---|---|---|---|---|---|---|---|");
      for (const d of g.candidates) {
        p(`| ${d.eligible ? "✅" : "—"} | ${md(d.name)} (${d.modelId}) | ${md(d.versionName)} (${d.versionId}) | ${md(d.baseModel)}${d.earlyAccess ? " ⏳EA" : ""} | ${md(d.file)} | ${d.format}/${d.fp ?? ""} | ${d.sizeMB ?? "?"} | \`${d.sha256 || ""}\` | ${d.pickle}/${d.virus} | ${d.thumbsUp ?? "?"} / ${d.downloads ?? "?"} | ${md(d.commercial)} |`);
      }
      const top = g.candidates.find((d) => d.eligible);
      if (top) {
        p();
        p("```yaml");
        p(`- { family: ${g.family}, source: civitai, civitai_model_id: ${top.modelId}, civitai_version_id: ${top.versionId} }  # ${top.name} — ${top.versionName}, ${top.sizeMB} MB, sha256 ${top.sha256}`);
        p("```");
      }
      p();
    }
  }
  if (rep.engine_releases) {
    p("## Engine releases (latest 10)");
    p();
    for (const e of rep.engine_releases) {
      const kinds = Object.keys(ENGINE_ASSETS[e.repo] || {});
      p(`### ${e.repo} (pinned: \`${e.pinned ?? "?"}\`)`);
      p();
      if (e.status !== 200) { p(`⚠️ GitHub API HTTP ${e.status}: ${e.note || ""}`); p(); continue; }
      p(`| tag | published | ${kinds.join(" | ")} |`);
      p(`|---|---|${kinds.map(() => "---").join("|")}|`);
      for (const r of e.releases) p(`| ${r.complete ? "**" + r.tag + "**" : r.tag}${r.prerelease ? " (pre)" : ""} | ${(r.published || "").slice(0, 10)} | ${kinds.map((k) => (r.found[k] ? "✓" : "✗")).join(" | ")} |`);
      p();
      if (e.best) {
        p(`Newest release with every needed asset: **${e.best.tag}**`);
        p();
        p("```yaml");
        p(`version: ${e.best.tag}`);
        for (const [k, a] of Object.entries(e.best.found)) p(`${k}: { url: ${a.url}, sha256: ${a.sha256 || "TODO"}, size_mb: ${Math.max(1, Math.round(a.size / 1048576))} }`);
        p("```");
      } else p("⚠️ None of the latest 10 releases has every needed asset — check the regexes in ENGINE_ASSETS against the asset list below.");
      p();
      if (e.releases[0]) {
        p(`<details><summary>All assets of ${e.releases[0].tag}</summary>`);
        p();
        for (const a of e.releases[0].allAssets) p(`- ${a}`);
        p();
        p("</details>");
        p();
      }
    }
  }
  return L.join("\n");
}

// ----------------------------------------------------------------------------
// main
// ----------------------------------------------------------------------------

async function main() {
  const YAML = await loadYamlLib();
  const readYaml = (name) => {
    const p = path.join(CONFIG_DIR, name);
    if (!fs.existsSync(p)) return null;
    return YAML.parse(fs.readFileSync(p, "utf8"), { merge: true, maxAliasCount: -1 });
  };
  const models = readYaml("models.yaml") || {};
  const engine = readYaml("engine.yaml") || {};
  const pins = [...collectPins(models, "models.yaml"), ...collectPins(engine, "engine.yaml")];

  // dedupe by URL, keep every location
  const byUrl = new Map();
  const todoUrls = [];
  for (const p of pins) {
    if (!p.url) { todoUrls.push(p); continue; }
    if (!byUrl.has(p.url)) byUrl.set(p.url, { ...p, whereAll: [] });
    const e = byUrl.get(p.url);
    e.whereAll.push(`${p.file}:${p.where}`);
    if (isSha(p.sha256) && !isSha(e.sha256)) e.sha256 = p.sha256;
    if (isSha(p.sha256) && isSha(e.sha256) && p.sha256.toLowerCase() !== e.sha256.toLowerCase()) e.conflict = true;
  }
  const entries = [...byUrl.values()];
  const hf = entries.filter((e) => HF_RE.test(e.url));
  const gh = entries.filter((e) => GH_RE.test(e.url));
  const other = entries.filter((e) => !HF_RE.test(e.url) && !GH_RE.test(e.url));

  if (DRY) {
    console.log(`# verify-pins dry run (${entries.length} URLs, ${todoUrls.length} url: TODO)\n`);
    for (const [label, list] of [["huggingface", hf], ["github-release", gh], ["download+hash", other]]) {
      console.log(`## ${label} (${list.length})`);
      for (const e of list) console.log(`- ${e.whereAll.join(", ")} → ${e.url}  sha256=${isSha(e.sha256) ? short(e.sha256) : "TODO"}${e.template ? "  (unresolved template)" : ""}`);
      console.log();
    }
    console.log("## url: TODO");
    for (const e of todoUrls) console.log(`- ${e.file}:${e.where}`);
    console.log(`\nbaseModels: ${[...collectBaseModels(models).keys()].join(", ")}`);
    console.log(`engine repos: ${collectRepos(engine).map((r) => r.repo).join(", ")}`);
    return 0;
  }

  const rep = { generated_at: new Date().toISOString() };
  if (ONLY.has("urls")) {
    for (const e of entries.filter((e) => e.template)) e.result = { status: 0, note: "unresolved {placeholder} in URL", hard: false };
    rep.hf_repos = await checkHuggingFace(hf.filter((e) => !e.template));
    await checkGithubAssets(gh.filter((e) => !e.template));
    await checkOther(other.filter((e) => !e.template));
    for (const e of entries) {
      finalize(e);
      if (e.conflict) { e.result.hard = true; e.result.note = "different sha256 pinned for the same URL in several places"; }
      e.where = e.whereAll.join(", ");
      e.file = e.whereAll.length > 1 ? "*" : e.file;
      if (e.file !== "*") e.where = e.where.replace(`${e.file}:`, "");
    }
    rep.entries = entries;
    rep.todo_urls = todoUrls;
  }
  if (ONLY.has("civitai")) {
    rep.civitai_enums = await civitaiEnums(collectBaseModels(models));
    rep.civitai_pinned = await civitaiPinned(models);
    rep.civitai_candidates = await civitaiCandidates(models);
  }
  if (ONLY.has("releases")) rep.engine_releases = await engineReleases(collectRepos(engine));

  const markdown = renderMarkdown(rep);
  console.log(markdown);
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, markdown + "\n");

  const json = {
    generated_at: rep.generated_at,
    urls: Object.fromEntries((rep.entries || []).map((e) => [e.url, {
      ok: !!e.result.ok, status: e.result.status ?? 0, sha256: e.result.sha256 || null, size: e.result.size ?? null,
      expected_sha256: e.result.expected_sha256, expected_size_mb: e.result.expected_size_mb, todo: !!e.result.todo,
      hard_failure: !!e.result.hard, source: e.result.source, where: e.whereAll, note: e.result.note || null,
      warning: e.result.warn || null, commit: e.result.commit || null,
    }])),
    todo_urls: (rep.todo_urls || []).map((e) => `${e.file}:${e.where}`),
    hf_repos: rep.hf_repos || [],
    civitai_enums: rep.civitai_enums || null,
    civitai_pinned: rep.civitai_pinned || null,
    civitai_candidates: rep.civitai_candidates || null,
    engine_releases: rep.engine_releases || null,
  };
  fs.writeFileSync(OUT, JSON.stringify(json, null, 2));
  console.log(`\nWrote ${OUT}`);

  const hard = (rep.entries || []).filter((e) => e.result?.hard);
  const hardCivitai = (rep.civitai_pinned || []).filter((e) => e.hard);
  for (const e of hardCivitai) console.error(`  - recommended.${e.role}[${e.index}] CivitAI ${e.modelId}/${e.versionId}: ${e.problems.join("; ")}`);
  if (hard.length || hardCivitai.length) {
    console.error(`\n${hard.length} hard failure(s):`);
    for (const e of hard) console.error(`  - ${e.whereAll.join(", ")}: ${e.result.note || `HTTP ${e.result.status}`} (${e.url})`);
    return 1;
  }
  return 0;
}

main().then((code) => process.exit(code), (err) => { console.error(err); process.exit(2); });
