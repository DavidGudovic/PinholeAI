#!/usr/bin/env node
// Collect Tauri bundles into release files with stable names + SHA256SUMS.txt.
//
//   node scripts/package.mjs --platform windows|linux [--out release-assets] [--target-dir DIR]
//                            [--portable-dir DIR]
//
// Windows → Pinhole-<ver>-windows-x64-setup.exe   (NSIS installer from `tauri build --bundles nsis`)
//           Pinhole-<ver>-windows-x64-portable.zip (Pinhole/Pinhole.exe + config/ + Data/README.txt)
// Linux   → Pinhole-<ver>-linux-x86_64.AppImage, Pinhole-<ver>-linux-amd64.deb
// Both    → SHA256SUMS.txt (sha256sum format) and a table in $GITHUB_STEP_SUMMARY.
//
// --portable-dir DIR also leaves the unzipped portable folder in DIR (CI uploads it as
// its own artifact so a human can download a ready-to-run zip from the run page).
// No dependencies (zip writer below).

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const argv = process.argv.slice(2);
const arg = (name, def) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : def;
};

const platform = arg("--platform", process.platform === "win32" ? "windows" : "linux");
const outDir = path.resolve(arg("--out", "release-assets"));
const portableDir = argv.includes("--portable-dir") ? path.resolve(arg("--portable-dir")) : null;

const conf = JSON.parse(fs.readFileSync(path.join(ROOT, "src-tauri", "tauri.conf.json"), "utf8"));
const version = conf.version;
const product = conf.productName || "Pinhole";

function die(msg) {
  console.error(`package: ${msg}`);
  process.exit(1);
}

function findTargetDir() {
  const candidates = [arg("--target-dir"), process.env.CARGO_TARGET_DIR, path.join(ROOT, "target"), path.join(ROOT, "src-tauri", "target")].filter(Boolean);
  for (const c of candidates) if (fs.existsSync(path.join(c, "release", "bundle"))) return path.resolve(c);
  die(`no release/bundle folder in ${candidates.join(", ")} — run \`npx tauri build\` first`);
}

function newest(dir, re) {
  if (!fs.existsSync(dir)) return null;
  const files = fs.readdirSync(dir).filter((f) => re.test(f)).map((f) => path.join(dir, f));
  files.sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs);
  return files[0] || null;
}

function cargoBinName() {
  const toml = fs.readFileSync(path.join(ROOT, "src-tauri", "Cargo.toml"), "utf8");
  const pkg = toml.split(/^\[/m).find((s) => s.startsWith("package]")) || "";
  const bin = toml.match(/^\[\[bin\]\][^[]*?name\s*=\s*"([^"]+)"/m);
  return bin ? bin[1] : (pkg.match(/^name\s*=\s*"([^"]+)"/m) || [])[1];
}

function copyDir(src, dst) {
  fs.mkdirSync(dst, { recursive: true });
  for (const e of fs.readdirSync(src, { withFileTypes: true })) {
    const s = path.join(src, e.name), d = path.join(dst, e.name);
    if (e.isDirectory()) copyDir(s, d);
    else if (e.isFile()) fs.copyFileSync(s, d);
  }
}

// ---------------------------------------------------------------- zip writer
const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
function crc32(buf) {
  if (typeof zlib.crc32 === "function") return zlib.crc32(buf) >>> 0;
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** Write a zip of `dir` (entries prefixed with `prefix/`). Deterministic timestamps. */
function writeZip(dir, prefix, outFile) {
  const entries = [];
  const walk = (abs, rel) => {
    const items = fs.readdirSync(abs, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name));
    for (const e of items) {
      const a = path.join(abs, e.name), r = `${rel}/${e.name}`;
      if (e.isDirectory()) { entries.push({ name: `${r}/`, dir: true }); walk(a, r); }
      else if (e.isFile()) entries.push({ name: r, file: a });
    }
  };
  entries.push({ name: `${prefix}/`, dir: true });
  walk(dir, prefix);

  // 2000-01-01 00:00 in DOS format (reproducible)
  const dosTime = 0, dosDate = ((2000 - 1980) << 9) | (1 << 5) | 1;
  const chunks = [];
  const central = [];
  let offset = 0;
  for (const e of entries) {
    const nameBuf = Buffer.from(e.name, "utf8");
    let data = Buffer.alloc(0), comp = data, method = 0, crc = 0;
    if (!e.dir) {
      data = fs.readFileSync(e.file);
      crc = crc32(data);
      const deflated = zlib.deflateRawSync(data, { level: 9 });
      if (deflated.length < data.length) { comp = deflated; method = 8; } else comp = data;
    }
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4);
    local.writeUInt16LE(0x0800, 6); // UTF-8 names
    local.writeUInt16LE(method, 8);
    local.writeUInt16LE(dosTime, 10);
    local.writeUInt16LE(dosDate, 12);
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(comp.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    local.writeUInt16LE(0, 28);
    chunks.push(local, nameBuf, comp);

    const cen = Buffer.alloc(46);
    cen.writeUInt32LE(0x02014b50, 0);
    cen.writeUInt16LE(20, 4); // made by: MS-DOS/FAT, v2.0
    cen.writeUInt16LE(20, 6);
    cen.writeUInt16LE(0x0800, 8);
    cen.writeUInt16LE(method, 10);
    cen.writeUInt16LE(dosTime, 12);
    cen.writeUInt16LE(dosDate, 14);
    cen.writeUInt32LE(crc, 16);
    cen.writeUInt32LE(comp.length, 20);
    cen.writeUInt32LE(data.length, 24);
    cen.writeUInt16LE(nameBuf.length, 28);
    cen.writeUInt16LE(0, 30);
    cen.writeUInt16LE(0, 32);
    cen.writeUInt16LE(0, 34);
    cen.writeUInt16LE(0, 36);
    cen.writeUInt32LE(e.dir ? 0x10 : 0x20, 38); // DOS dir / archive attribute
    cen.writeUInt32LE(offset, 42);
    central.push(cen, nameBuf);
    offset += local.length + nameBuf.length + comp.length;
    if (offset > 0xffffffff) die("portable zip exceeds 4 GB (zip64 not implemented)");
  }
  const cenSize = central.reduce((n, b) => n + b.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(entries.length, 8);
  end.writeUInt16LE(entries.length, 10);
  end.writeUInt32LE(cenSize, 12);
  end.writeUInt32LE(offset, 16);
  fs.writeFileSync(outFile, Buffer.concat([...chunks, ...central, end]));
}

// ---------------------------------------------------------------- portable texts
const DATA_README = `Pinhole ${version} — portable mode
=====================================

This "Data" folder sits next to Pinhole.exe, so Pinhole runs in PORTABLE MODE:
models, the downloaded engine, settings, styles, presets and the images you
save all live in this folder. Pinhole does not use %LOCALAPPDATA%\\Pinhole.

- Move or copy the whole "Pinhole" folder (Pinhole.exe + config + Data) to take
  everything with you, e.g. to a USB drive. Keep the folder somewhere you can
  write to (not "C:\\Program Files").
- Delete or rename this Data folder to switch to installed mode
  (%LOCALAPPDATA%\\Pinhole\\Data).
- Models are big: make sure the drive has room (5–25 GB per model).

Privacy: Pinhole never stores your prompts. Generated images stay in memory
until you click Save (saved images go to Data\\outputs). No telemetry.
Note: the Microsoft Edge WebView2 runtime that draws Pinhole's window keeps its
own small browser cache under %LOCALAPPDATA%\\${conf.identifier} (no prompts or
images are written there by Pinhole).

Requirements
- Windows 10 or 11, 64-bit.
- Microsoft Edge WebView2 Runtime. Windows 11 already has it. On Windows 10, if
  Pinhole does not open, install the "Evergreen Standalone Installer" from
  https://developer.microsoft.com/microsoft-edge/webview2/ (or use the
  Pinhole setup .exe instead, which installs WebView2 for you).
- A GPU is strongly recommended (NVIDIA: CUDA build; AMD/Intel: Vulkan build).
  Without one Pinhole falls back to the CPU engine, which is very slow.
`;

const TOP_README = `Pinhole ${version} — simple, private, offline AI image generator
===================================================================

Start: double-click Pinhole.exe.

On first run Pinhole detects your GPU, downloads the matching image engine
(stable-diffusion.cpp) once, and suggests the best models for your GPU —
each one is a one-click download. After that it works fully offline.

This is the portable build: everything is kept in the "Data" folder next to
Pinhole.exe (see Data\\README.txt). The "config" folder holds Pinhole's
built-in model registry — don't delete it.

Licenses: LICENSE.txt (Pinhole, MIT) and THIRD_PARTY_LICENSES.txt.
`;

// ---------------------------------------------------------------- main
function sha256(file) {
  const h = crypto.createHash("sha256");
  const fd = fs.openSync(file, "r");
  const buf = Buffer.alloc(1 << 20);
  let n;
  while ((n = fs.readSync(fd, buf, 0, buf.length, null)) > 0) h.update(buf.subarray(0, n));
  fs.closeSync(fd);
  return h.digest("hex");
}

const target = findTargetDir();
const bundle = path.join(target, "release", "bundle");
// Only ever delete files this script produces (never a whole user-supplied folder).
fs.mkdirSync(outDir, { recursive: true });
for (const f of fs.readdirSync(outDir)) {
  if (/^(SHA256SUMS\.txt|.+-\d+\.\d+\.\d+.*-(windows|linux)-.+)$/.test(f) && f.startsWith(product)) fs.rmSync(path.join(outDir, f), { force: true });
  else if (f === "SHA256SUMS.txt") fs.rmSync(path.join(outDir, f), { force: true });
}
const produced = [];
const put = (src, name) => {
  const dst = path.join(outDir, name);
  fs.copyFileSync(src, dst);
  produced.push(dst);
  console.log(`  ${path.relative(ROOT, src)} → ${name}`);
};

console.log(`package: ${product} ${version} (${platform}) from ${path.relative(ROOT, target) || target}`);

if (platform === "windows") {
  const setup = newest(path.join(bundle, "nsis"), /\.exe$/i);
  if (!setup) die(`no NSIS installer in ${path.join(bundle, "nsis")}`);
  put(setup, `${product}-${version}-windows-x64-setup.exe`);

  const names = [conf.mainBinaryName, cargoBinName(), product, product.toLowerCase()].filter(Boolean);
  const exe = names.map((n) => path.join(target, "release", `${n}.exe`)).find((p) => fs.existsSync(p));
  if (!exe) die(`app binary not found in ${path.join(target, "release")} (tried ${names.map((n) => n + ".exe").join(", ")})`);

  const stageRoot = path.join(target, "portable-stage");
  const stage = path.join(stageRoot, product);
  fs.rmSync(stageRoot, { recursive: true, force: true });
  fs.mkdirSync(path.join(stage, "Data"), { recursive: true });
  fs.copyFileSync(exe, path.join(stage, `${product}.exe`));
  copyDir(path.join(ROOT, "config"), path.join(stage, "config"));
  // MSVC runtime DLLs for the engines (collected by the bundle workflow).
  const vcrt = path.join(ROOT, "src-tauri", "vcrt");
  if (fs.existsSync(vcrt)) copyDir(vcrt, path.join(stage, "vcrt"));
  else console.warn("package: src-tauri/vcrt missing — engines will need the VC++ runtime installed");
  fs.writeFileSync(path.join(stage, "Data", "README.txt"), DATA_README.replace(/\n/g, "\r\n"));
  fs.writeFileSync(path.join(stage, "README.txt"), TOP_README.replace(/\n/g, "\r\n"));
  fs.copyFileSync(path.join(ROOT, "LICENSE"), path.join(stage, "LICENSE.txt"));
  if (fs.existsSync(path.join(ROOT, "THIRD_PARTY_LICENSES"))) fs.copyFileSync(path.join(ROOT, "THIRD_PARTY_LICENSES"), path.join(stage, "THIRD_PARTY_LICENSES.txt"));
  else console.warn("package: THIRD_PARTY_LICENSES missing — not included in the portable zip");

  const zipName = `${product}-${version}-windows-x64-portable.zip`;
  writeZip(stage, product, path.join(outDir, zipName));
  produced.push(path.join(outDir, zipName));
  console.log(`  ${path.relative(ROOT, exe)} + config/ + Data/README.txt → ${zipName}`);
  if (portableDir) {
    const existing = fs.existsSync(portableDir) ? fs.readdirSync(portableDir) : [];
    if (existing.length && !existing.includes(`${product}.exe`)) die(`--portable-dir ${portableDir} is not empty and is not a previous portable folder; refusing to overwrite`);
    if (existing.length) fs.rmSync(portableDir, { recursive: true, force: true });
    copyDir(stage, portableDir);
    console.log(`  portable folder → ${portableDir}`);
  }
} else if (platform === "linux") {
  const appimage = newest(path.join(bundle, "appimage"), /\.AppImage$/);
  const deb = newest(path.join(bundle, "deb"), /\.deb$/);
  if (!appimage && !deb) die(`no AppImage or .deb in ${bundle}`);
  if (appimage) {
    put(appimage, `${product}-${version}-linux-x86_64.AppImage`);
    fs.chmodSync(path.join(outDir, `${product}-${version}-linux-x86_64.AppImage`), 0o755);
  } else console.warn("package: no AppImage produced");
  if (deb) put(deb, `${product}-${version}-linux-amd64.deb`);
  else console.warn("package: no .deb produced");
} else {
  die(`unknown --platform ${platform}`);
}

const sums = produced.map((f) => `${sha256(f)}  ${path.basename(f)}`);
fs.writeFileSync(path.join(outDir, "SHA256SUMS.txt"), sums.join("\n") + "\n");
console.log("\nSHA256SUMS.txt\n" + sums.join("\n"));

if (process.env.GITHUB_STEP_SUMMARY) {
  const rows = produced.map((f, i) => `| \`${path.basename(f)}\` | ${(fs.statSync(f).size / 1048576).toFixed(1)} MB | \`${sums[i].split(" ")[0]}\` |`);
  fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, [
    `### ${product} ${version} — ${platform}`, "",
    "Download from **Artifacts** at the bottom of this run's Summary page.", "",
    "| file | size | sha256 |", "|---|---|---|", ...rows, "",
  ].join("\n"));
}
