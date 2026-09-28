#!/usr/bin/env node
// Real-app end-to-end test: drives the built Pinhole binary (real Rust core, real
// Tauri IPC, real WebKitGTK) through tauri-driver + WebKitWebDriver. Linux only.
// See tests/e2e/README.md for setup. No repo dependencies are added: the WebDriver
// client is installed on demand into a cache folder outside node_modules.
//
//   xvfb-run -a -s "-screen 0 1440x960x24" node tests/e2e/run.mjs
//
// Env:
//   PINHOLE_APP          app binary (default target/debug/pinhole; build it with
//                        `cargo build -p pinhole --features tauri/custom-protocol`)
//   PINHOLE_E2E_OUT      screenshots + report (default target/e2e)
//   PINHOLE_E2E_DATA     Data folder to use (default: fresh temp folder; kept if set)
//   PINHOLE_E2E_ENGINE   "1" (default) downloads the real CPU engine from GitHub; "0" skips it
//   PINHOLE_E2E_ONLY     regex: run only matching steps (state from skipped steps is missing!)
//   PINHOLE_E2E_DEPS     where to npm-install selenium-webdriver (default <tmp>/pinhole-e2e-deps)
//   TAURI_DRIVER         tauri-driver binary (default: on PATH / ~/.cargo/bin)
//
// PRIVACY: the sentinel prompt typed below must never reach Data/ — the last step
// scans every file for it.

import { spawn, execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import zlib from "node:zlib";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..");
const APP = path.resolve(process.env.PINHOLE_APP || path.join(repo, "target", "debug", "pinhole"));
const OUT = path.resolve(process.env.PINHOLE_E2E_OUT || path.join(repo, "target", "e2e"));
const KEEP_DATA = !!process.env.PINHOLE_E2E_DATA;
const DATA = path.resolve(process.env.PINHOLE_E2E_DATA || fs.mkdtempSync(path.join(os.tmpdir(), "pinhole-e2e-")) + "/Data");
const WITH_ENGINE = (process.env.PINHOLE_E2E_ENGINE ?? "1") !== "0";
const ONLY = process.env.PINHOLE_E2E_ONLY ? new RegExp(process.env.PINHOLE_E2E_ONLY) : null;
const DEPS = path.resolve(process.env.PINHOLE_E2E_DEPS || path.join(os.tmpdir(), "pinhole-e2e-deps"));
const PORT = Number(process.env.PINHOLE_E2E_PORT || 4444);

const SENTINEL = "PINHOLE_SENTINEL_7f3a";
const PROMPT = `a lighthouse on a cliff at dusk ${SENTINEL}`;
// What CivitAI's "Copy generation data" button produces (A1111 format).
const CIVITAI_TEXT = [
  `masterpiece, portrait of a knight in ornate armor, ${SENTINEL}_PASTE, dramatic lighting`,
  "Negative prompt: lowres, blurry, watermark",
  'Steps: 28, Sampler: DPM++ 2M Karras, CFG scale: 6.5, Seed: 1234567, Size: 512x768, Clip skip: 2, Model hash: 6ce0161689, Model: v1-5-pruned-emaonly, Denoising strength: 0.4, Hires upscale: 1.5, Hires upscaler: 4x-UltraSharp, Lora hashes: "detail_tweaker: e3f9c2b1a8d7", Civitai resources: [{"type":"checkpoint","modelVersionId":128713,"modelName":"DreamShaper","modelVersionName":"8"},{"type":"lora","weight":0.6,"modelVersionId":62833,"modelName":"Detail Tweaker LoRA","modelVersionName":"v1.0"}], Version: v1.9.4',
].join("\n");

fs.mkdirSync(OUT, { recursive: true });
fs.mkdirSync(DATA, { recursive: true });

// ------------------------------------------------------------------ deps

function loadSelenium() {
  const req = createRequire(path.join(DEPS, "package.json"));
  try {
    return req("selenium-webdriver");
  } catch {
    fs.mkdirSync(DEPS, { recursive: true });
    if (!fs.existsSync(path.join(DEPS, "package.json"))) fs.writeFileSync(path.join(DEPS, "package.json"), '{"private":true}\n');
    console.log(`e2e: installing selenium-webdriver into ${DEPS}`);
    execFileSync("npm", ["i", "--no-save", "--no-audit", "--no-fund", "selenium-webdriver@4"], { cwd: DEPS, stdio: "inherit" });
    return req("selenium-webdriver");
  }
}
const { Builder, By, Key, until } = loadSelenium();

// ------------------------------------------------------------------ tiny harness

const results = [];
let driver;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function step(name, fn) {
  if (ONLY && !ONLY.test(name)) return;
  const t0 = Date.now();
  const notes = [];
  try {
    await fn((n) => notes.push(n));
    results.push({ name, ok: true, ms: Date.now() - t0, notes });
    console.log(`PASS ${name} (${((Date.now() - t0) / 1000).toFixed(1)}s)${notes.length ? "\n     " + notes.join("\n     ") : ""}`);
  } catch (e) {
    results.push({ name, ok: false, ms: Date.now() - t0, notes, error: String(e?.stack || e) });
    console.log(`FAIL ${name}: ${e?.message || e}${notes.length ? "\n     " + notes.join("\n     ") : ""}`);
    await shot(`FAIL-${name}`).catch(() => undefined);
    await closeOverlays().catch(() => undefined);
  }
}

function assert(cond, msg) {
  if (!cond) throw new Error(msg);
}

async function shot(name) {
  await settle().catch(() => undefined);
  const png = await driver.takeScreenshot();
  const file = path.join(OUT, `${name.replace(/[^\w.-]+/g, "_")}.png`);
  fs.writeFileSync(file, Buffer.from(png, "base64"));
  return file;
}

const lit = (s) => (s.includes("'") ? `concat('${s.split("'").join(`', "'", '`)}')` : `'${s}'`);
const xButton = (text) => `//button[contains(normalize-space(.), ${lit(text)}) or @aria-label=${lit(text)}]`;

async function bodyText() {
  return driver.executeScript("return document.body.innerText");
}

async function waitText(text, timeout = 15000) {
  const re = text instanceof RegExp ? text : null;
  await driver.wait(async () => {
    const t = await bodyText();
    return re ? re.test(t) : t.includes(text);
  }, timeout, `text not shown: ${text}`);
}

async function waitNoText(text, timeout = 15000) {
  await driver.wait(async () => !(await bodyText()).includes(text), timeout, `text still shown: ${text}`);
}

/** First visible element matching the XPath. */
async function visible(xpath, timeout = 10000) {
  let found;
  await driver.wait(async () => {
    for (const el of await driver.findElements(By.xpath(xpath))) {
      if (await el.isDisplayed().catch(() => false)) {
        found = el;
        return true;
      }
    }
    return false;
  }, timeout, `not visible: ${xpath}`);
  return found;
}

async function click(xpath, timeout) {
  const el = await visible(xpath, timeout);
  await driver.wait(until.elementIsEnabled(el), timeout ?? 10000, `disabled: ${xpath}`);
  await driver.executeScript("arguments[0].scrollIntoView({block:'center'})", el);
  try {
    await el.click();
  } catch {
    await driver.executeScript("arguments[0].click()", el);
  }
  return el;
}
const clickButton = (text, timeout) => click(xButton(text), timeout);

async function closeOverlays() {
  for (let i = 0; i < 3; i++) {
    const open = await driver.executeScript("return !!document.querySelector('[role=dialog]')");
    if (!open) return;
    await driver.actions().sendKeys(Key.ESCAPE).perform();
    await sleep(300);
  }
}

/** Real IPC call from the page (same bridge the UI uses). */
async function invoke(cmd, args = {}) {
  const r = await driver.executeAsyncScript(
    `const [cmd, args, done] = arguments;
     window.__TAURI_INTERNALS__.invoke(cmd, args).then((v) => done({ ok: true, v }), (e) => done({ ok: false, e }));`,
    cmd,
    args,
  );
  if (!r.ok) throw Object.assign(new Error(`${cmd}: ${JSON.stringify(r.e)}`), { core: r.e });
  return r.v;
}

const hasXdotool = (() => {
  try {
    execFileSync("xdotool", ["version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
})();

/**
 * Answer a native GTK file chooser (tauri-plugin-dialog) by typing `file` into it.
 * WebDriver can't reach native dialogs and Tauri's IPC internals are read-only, so
 * this uses xdotool on the Xvfb display. Returns false when xdotool is missing.
 */
async function answerNativeOpenDialog(title, file, timeout = 10000) {
  if (!hasXdotool) return false;
  const x = (...a) => execFileSync("xdotool", a).toString().trim();
  let win = "";
  const t0 = Date.now();
  while (!win && Date.now() - t0 < timeout) {
    await sleep(250);
    try {
      win = x("search", "--name", title).split("\n")[0];
    } catch {
      /* not open yet */
    }
  }
  if (!win) throw new Error(`native dialog "${title}" did not open`);
  x("windowfocus", "--sync", win);
  await sleep(300);
  x("key", "--window", win, "ctrl+l");
  await sleep(300);
  x("type", "--window", win, "--delay", "5", file);
  await sleep(300);
  x("key", "--window", win, "Return");
  return true;
}

/** Let React commit and WebKit paint before a screenshot. */
async function settle(ms = 250) {
  await driver.executeAsyncScript("const done = arguments[0]; requestAnimationFrame(() => requestAnimationFrame(() => done()))");
  await sleep(ms);
}

async function selectValue(css, value) {
  // Clicking <option> is intercepted in WebKitWebDriver; set it the way a user's pick does.
  await driver.executeScript(
    `const [css, v] = arguments; const el = document.querySelector(css);
     el.value = v; el.dispatchEvent(new Event('change', { bubbles: true }));`,
    css,
    value,
  );
}

async function setFileInput(file, scope = "") {
  const input = await driver.findElement(By.css(`${scope} input[type=file]:not([disabled])`.trim()));
  // Pure-JS fallback when the driver refuses hidden inputs.
  try {
    await input.sendKeys(file);
  } catch {
    const b64 = fs.readFileSync(file).toString("base64");
    await driver.executeScript(
      `const [input, b64, name] = arguments; const bin = atob(b64); const u8 = new Uint8Array(bin.length);
       for (let i = 0; i < bin.length; i++) u8[i] = bin.charCodeAt(i);
       const dt = new DataTransfer(); dt.items.add(new File([u8], name, { type: 'image/png' }));
       input.files = dt.files; input.dispatchEvent(new Event('change', { bubbles: true }));`,
      input,
      b64,
      path.basename(file),
    );
  }
}

async function openTab(label) {
  await click(`//nav[@role='tablist']//button[@role='tab' and contains(normalize-space(.), ${lit(label)})]`);
  await sleep(300);
}

async function openSettings() {
  await click(`//button[@aria-label='Settings']`);
  await waitText("Offline mode");
  await sleep(500);
}

async function closeSettings() {
  await click(`//div[@role='dialog']//button[@aria-label='Close']`);
  await driver.wait(async () => !(await driver.executeScript("return !!document.querySelector('[role=dialog]')")), 5000);
}

function readSettingsYaml() {
  const f = path.join(DATA, "config", "settings.yaml");
  return fs.existsSync(f) ? fs.readFileSync(f, "utf8") : "";
}
const yamlValue = (yaml, key) => (yaml.match(new RegExp(`^${key}:\\s*(.*)$`, "m")) || [])[1]?.trim();

async function waitSetting(key, want, timeout = 5000) {
  await driver.wait(async () => yamlValue(readSettingsYaml(), key) === want, timeout, `settings.yaml ${key} != ${want} (is ${yamlValue(readSettingsYaml(), key)})`);
}

async function toggleByLabel(label) {
  return click(`//div[div[normalize-space(.)=${lit(label)}]]/following-sibling::div//button[@role='switch'] | //label[.//span[normalize-space(.)=${lit(label)}]]/button[@role='switch']`);
}

async function switchState(label) {
  const el = await visible(`//label[.//span[normalize-space(.)=${lit(label)}]]/button[@role='switch']`);
  return (await el.getAttribute("aria-checked")) === "true";
}

// ------------------------------------------------------------------ fixtures

/** A PNG with no chunks but IHDR/IDAT/IEND (solid gradient), for Edit/Describe imports. */
function writeTestPng(file, w = 96, h = 64) {
  const crcTable = Array.from({ length: 256 }, (_, n) => {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    return c >>> 0;
  });
  const crc = (buf) => {
    let c = 0xffffffff;
    for (const b of buf) c = crcTable[(c ^ b) & 0xff] ^ (c >>> 8);
    return (c ^ 0xffffffff) >>> 0;
  };
  const chunk = (type, data) => {
    const len = Buffer.alloc(4);
    len.writeUInt32BE(data.length);
    const td = Buffer.concat([Buffer.from(type), data]);
    const c = Buffer.alloc(4);
    c.writeUInt32BE(crc(td));
    return Buffer.concat([len, td, c]);
  };
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(w, 0);
  ihdr.writeUInt32BE(h, 4);
  ihdr[8] = 8;
  ihdr[9] = 2; // RGB
  const raw = Buffer.alloc((w * 3 + 1) * h);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) {
      const o = y * (w * 3 + 1) + 1 + x * 3;
      raw[o] = (x * 255) / w;
      raw[o + 1] = (y * 255) / h;
      raw[o + 2] = 160;
    }
  const png = Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk("IHDR", ihdr),
    chunk("IDAT", zlib.deflateSync(raw)),
    chunk("IEND", Buffer.alloc(0)),
  ]);
  fs.writeFileSync(file, png);
}

/**
 * A header-only "SD 1.5" safetensors file: the tensor names the registry's detect
 * rules look for (UNet + CLIP-L token embedding with ne0 = 768) and tiny data.
 * Enough for "Add a file I already have" → detection → installed.json; the engine
 * cannot actually load it (that error path is tested too).
 */
function writeFakeSd15(file) {
  const tensors = {
    "model.diffusion_model.input_blocks.0.0.weight": { dtype: "F16", shape: [1, 4] },
    "cond_stage_model.transformer.text_model.embeddings.token_embedding.weight": { dtype: "F16", shape: [1, 768] },
    "first_stage_model.decoder.conv_in.weight": { dtype: "F16", shape: [1, 4] },
  };
  let off = 0;
  const header = {};
  for (const [k, v] of Object.entries(tensors)) {
    const n = v.shape.reduce((a, b) => a * b, 1) * 2;
    header[k] = { ...v, data_offsets: [off, off + n] };
    off += n;
  }
  let json = Buffer.from(JSON.stringify(header));
  const pad = (8 - (json.length % 8)) % 8;
  json = Buffer.concat([json, Buffer.alloc(pad, 0x20)]);
  const len = Buffer.alloc(8);
  len.writeBigUInt64LE(BigInt(json.length));
  fs.writeFileSync(file, Buffer.concat([len, json, Buffer.alloc(off)]));
}

/** Every file under `dir` containing `needle` (raw bytes, so PNG chunks are covered). */
function scanFor(dir, needle, skip = () => false) {
  const hits = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (skip(p)) continue;
      if (e.isDirectory()) walk(p);
      else if (e.isFile() && fs.statSync(p).size < 512 * 1024 * 1024) {
        const buf = fs.readFileSync(p);
        if (buf.includes(needle) || p.includes(needle)) hits.push(p);
      }
    }
  };
  if (fs.existsSync(dir)) walk(dir);
  return hits;
}

// ------------------------------------------------------------------ run

const fixtures = path.join(path.dirname(DATA), "fixtures");
fs.mkdirSync(fixtures, { recursive: true });
const PNG = path.join(fixtures, "photo.png");
const FAKE_MODEL = path.join(fixtures, "my-sd15-model.safetensors");
writeTestPng(PNG);
writeFakeSd15(FAKE_MODEL);

function findTauriDriver() {
  if (process.env.TAURI_DRIVER) return process.env.TAURI_DRIVER;
  const cargoBin = path.join(os.homedir(), ".cargo", "bin", "tauri-driver");
  return fs.existsSync(cargoBin) ? cargoBin : "tauri-driver";
}

assert(fs.existsSync(APP), `app binary not found: ${APP} (cargo build -p pinhole --features tauri/custom-protocol)`);
console.log(`e2e: app ${APP}\ne2e: Data ${DATA}\ne2e: screenshots ${OUT}`);

// The app's own stdout/stderr (inherited through tauri-driver → WebKitWebDriver) go to
// app.log, which the privacy step scans for the sentinel too.
const APP_LOG = path.join(OUT, "app.log");
const logFd = fs.openSync(APP_LOG, "w");
const tauriDriver = spawn(findTauriDriver(), ["--port", String(PORT), "--native-port", String(PORT + 1)], {
  // GSETTINGS_BACKEND=memory: no dconf/dbus in CI containers (silences warnings).
  env: { ...process.env, PINHOLE_DATA_DIR: DATA, GSETTINGS_BACKEND: "memory" },
  stdio: ["ignore", logFd, logFd],
});
tauriDriver.on("error", (e) => {
  console.error(`e2e: could not start tauri-driver: ${e.message}`);
  process.exit(2);
});

const cleanup = async () => {
  if (driver) await driver.quit().catch(() => undefined);
  tauriDriver.kill();
};
process.on("SIGINT", async () => {
  await cleanup();
  process.exit(130);
});

try {
  for (let i = 0; i < 50; i++) {
    try {
      const r = await fetch(`http://127.0.0.1:${PORT}/status`);
      if (r.ok) break;
    } catch {
      /* not up yet */
    }
    await sleep(200);
  }
  driver = await new Builder()
    .usingServer(`http://127.0.0.1:${PORT}/`)
    .withCapabilities({ browserName: "wry", "tauri:options": { application: APP } })
    .build();
  await driver.manage().setTimeouts({ script: 120000 });
  await driver.wait(async () => (await bodyText()).length > 20, 30000, "app did not render");

  // ---------------------------------------------------------------- first run
  await step("firstrun-welcome", async (note) => {
    await waitText("Welcome to Pinhole");
    note(`title=${await driver.getTitle()}`);
    await shot("01-firstrun-welcome");
  });

  await step("firstrun-hardware", async (note) => {
    await clickButton("Get started");
    await waitText("Your computer");
    await waitText(/No GPU found|Good to go/, 30000);
    const hw = await invoke("get_hardware");
    note(`get_hardware: backend=${hw.backend} tier=${hw.tier} gpu=${hw.gpu ? hw.gpu.name : "none"} ram=${hw.detected?.ramGb}GB threads=${hw.detected?.cpuThreads}`);
    await shot("02-firstrun-hardware");
  });

  await step("firstrun-engine", async (note) => {
    await clickButton("Continue");
    await waitText("Download the image engine");
    const st = await invoke("engine_status");
    note(`engine_status before: installed=${st.installed} backend=${st.backend}`);
    await shot("03-firstrun-engine");
    if (!WITH_ENGINE) {
      note("PINHOLE_E2E_ENGINE=0: engine download skipped");
      return;
    }
    await clickButton("Download engine");
    // Watch progress events render: sample the progress bar + label.
    const seen = new Set();
    const labels = new Set();
    let midShot = false;
    const t0 = Date.now();
    while (Date.now() - t0 < 10 * 60 * 1000) {
      const s = await driver.executeScript(
        `const bar = document.querySelector('[role=progressbar]');
         return { v: bar ? bar.getAttribute('aria-valuenow') : null, t: document.body.innerText };`,
      );
      if (s.v != null) seen.add(s.v);
      const m = s.t.match(/[\d.]+\s*(?:KB|MB|GB)\s+of\s+[\d.]+\s*(?:KB|MB|GB)|Checking[^\n]*|Unpacking[^\n]*|Starting download[^\n]*/);
      if (m) labels.add(m[0]);
      if (!midShot && s.v != null && Number(s.v) > 20) {
        await shot("03-firstrun-engine-downloading");
        midShot = true;
      }
      if (/Ready\n|Ready$/m.test(s.t) && s.t.includes("Version ")) break;
      if (/Try again/.test(s.t)) {
        await shot("03-firstrun-engine-failed");
        throw new Error(`engine install failed: ${s.t.split("\n").filter((l) => /fail|error|couldn|can't|could not/i.test(l)).join(" | ")}`);
      }
      await sleep(250);
    }
    note(`progress values seen: ${[...seen].join(",")}`);
    note(`progress labels seen: ${[...labels].slice(0, 6).join(" | ")}${labels.size > 6 ? " …" : ""}`);
    const after = await invoke("engine_status");
    note(`engine_status after: installed=${after.installed} version=${after.version} backend=${after.backend}`);
    assert(after.installed, "engine not installed");
    assert(seen.size >= 2, "no download progress rendered");
    await shot("03-firstrun-engine-ready");
    const dir = path.join(DATA, "engine");
    const bins = execFileSync("find", [dir, "-name", "sd-server*", "-type", "f"]).toString().trim();
    note(`engine files: ${bins.replace(DATA, "Data")}`);
    assert(bins, "sd-server not unpacked into Data/engine");
  });

  await step("firstrun-recommended", async (note) => {
    await clickButton("Continue");
    await waitText(/Recommended for your/);
    await sleep(1500);
    const recs = await invoke("get_recommended");
    note(`get_recommended: ${recs.map((r) => `${r.role}=${r.title ?? r.modelTitle ?? r.id ?? "?"}${r.fit ? `(${r.fit})` : ""}`).join(", ")}`);
    await shot("04-firstrun-recommended");
  });

  await step("firstrun-done", async (note) => {
    await clickButton("Done");
    await visible(`//nav[@role='tablist']`, 15000);
    await waitSetting("firstRunDone", "true").catch(async () => {
      note(`settings.yaml:\n${readSettingsYaml()}`);
      throw new Error("firstRunDone not persisted");
    });
    await shot("05-main-create-no-models");
  });

  // ---------------------------------------------------------------- create, empty
  await step("create-empty-state", async (note) => {
    await openTab("Create");
    await waitText("Get a model to start creating");
    // Wait for the cards to replace their skeletons.
    await driver.wait(async () => !(await driver.executeScript("return !!document.querySelector('#tab-create [aria-busy=true]')")), 10000);
    const cards = await driver.executeScript("return document.querySelector('#tab-create').innerText");
    note(`Create empty state: ${cards.replace(/\n+/g, " | ").slice(0, 300)}`);
    const gets = await driver.findElements(By.css("#tab-create button[aria-label^='Get ']"));
    note(`one-click Get buttons on Create: ${gets.length}`);
    await shot("05b-create-empty-recommended");
  });

  // ---------------------------------------------------------------- settings
  await step("settings-controls", async (note) => {
    await openSettings();
    await shot("06-settings");
    const info = await invoke("app_info");
    note(`app_info: dataDir=${info.dataDir} portable=${info.portable} version=${info.version}`);
    assert(path.resolve(info.dataDir) === DATA, `dataDir ${info.dataDir} != ${DATA}`);

    // Offline mode on/off → settings.yaml
    await toggleByLabel("Offline mode");
    await waitText("Saved");
    await waitSetting("offline", "true");
    await toggleByLabel("Offline mode");
    await waitSetting("offline", "false");

    // Theme
    const isDark = () => driver.executeScript("return document.documentElement.classList.contains('dark')");
    await click(`//div[@role='radiogroup']//button[normalize-space(.)='Dark']`);
    await waitSetting("theme", "dark");
    await driver.wait(isDark, 5000, "dark class not applied");
    await sleep(600);
    await shot("07-settings-dark");
    await click(`//div[@role='radiogroup']//button[normalize-space(.)='Light']`);
    await waitSetting("theme", "light");
    await driver.wait(async () => !(await isDark()), 5000, "dark class still applied");

    // Content mode, paid, trigger words, saved metadata
    await click(`//div[@role='radiogroup']//button[normalize-space(.)='Include 18+']`);
    await waitSetting("contentMode", "include_18plus");
    await click(`//div[@role='radiogroup']//button[normalize-space(.)='Safe only']`);
    await waitSetting("contentMode", "safe");
    const paid0 = yamlValue(readSettingsYaml(), "showPaid");
    await toggleByLabel("Show paid (early access) models");
    await waitSetting("showPaid", paid0 === "true" ? "false" : "true");
    await toggleByLabel("Show paid (early access) models");
    await waitSetting("showPaid", paid0);
    const tw0 = yamlValue(readSettingsYaml(), "addTriggerWords");
    await toggleByLabel("Add trigger words automatically");
    await waitSetting("addTriggerWords", tw0 === "true" ? "false" : "true");
    await toggleByLabel("Add trigger words automatically");
    await waitSetting("addTriggerWords", tw0);
    await click(`//div[@role='radiogroup']//button[normalize-space(.)='Settings (no prompt)']`);
    await waitSetting("savedMetadata", "settings");
    await click(`//div[@role='radiogroup']//button[normalize-space(.)='None']`);
    await waitSetting("savedMetadata", "none");

    // Selects: GPU, VRAM, engine backend
    const selects = await driver.findElements(By.css("[role=dialog] select"));
    note(`selects in sheet: ${selects.length}`);
    for (const sel of selects) {
      const label = await sel.getAttribute("aria-label");
      const opts = await sel.findElements(By.css("option"));
      const texts = await Promise.all(opts.map((o) => o.getText()));
      note(`select "${label}": ${texts.join(" / ")}`);
    }
    await selectValue("select[aria-label='Graphics memory']", "8");
    await waitSetting("vramOverrideGb", "8.0");
    await sleep(800);
    const hw8 = await invoke("get_hardware");
    note(`VRAM override 8 GB → get_hardware: vramGb=${hw8.vramGb} tier=${hw8.tier} backend=${hw8.backend}`);
    await shot("08-settings-vram-8");
    await selectValue("select[aria-label='Graphics memory']", "auto");
    await waitSetting("vramOverrideGb", "null");
    await selectValue("select[aria-label='Engine backend']", "vulkan");
    await waitSetting("engineBackend", "vulkan");
    await sleep(800);
    const mism = (await bodyText()).match(/You picked[^\n]*/);
    note(`backend → vulkan: ${mism ? mism[0] : "no mismatch hint shown"}`);
    await shot("08b-settings-backend-mismatch");
    await selectValue("select[aria-label='Engine backend']", "auto");
    await waitSetting("engineBackend", "auto");
    await selectValue("select[aria-label='Graphics card']", "cpu");
    await waitSetting("gpu", "cpu");
    await selectValue("select[aria-label='Graphics card']", "auto");
    await waitSetting("gpu", "auto");

    // CivitAI key → OS keychain (Secret Service on Linux; usually absent in CI containers).
    await clickButton("Add key");
    const keyInput = await visible("//input[@placeholder='Paste your key']", 5000);
    await keyInput.sendKeys("e2e-not-a-real-key");
    await shot("09-settings-apikey");
    await clickButton("Save key");
    await driver.wait(async () => /Saved in your system keychain/.test(await bodyText()) || (await driver.findElements(By.xpath("//div[@role='dialog'][.//*[contains(.,'CivitAI API key')]]//*[@role='alert']"))).length > 0, 15000, "no result after Save key");
    const keyAlert = await driver.findElements(By.xpath("//div[@role='dialog'][.//*[contains(.,'CivitAI API key')]]//*[@role='alert']"));
    if (keyAlert.length) {
      note(`set_civitai_key error: ${(await keyAlert[0].getText()).replace(/\n/g, " | ")}`);
      await shot("09b-settings-apikey-error");
      await clickButton("Not now");
    } else {
      note("key saved to the keychain; removing it again");
      assert(!fs.readdirSync(DATA, { recursive: true }).some((f) => { const p = path.join(DATA, String(f)); return fs.statSync(p).isFile() && fs.readFileSync(p).includes("e2e-not-a-real-key"); }), "API key written into Data/");
      await clickButton("Remove");
      await waitText("Not set");
    }
    await closeOverlays();
    const yaml = readSettingsYaml();
    note(`settings.yaml keys: ${yaml.split("\n").filter((l) => /^\w/.test(l)).map((l) => l.split(":")[0]).join(", ")}`);
    assert(!yaml.includes(SENTINEL), "sentinel in settings.yaml");
  });

  await step("settings-open-data-folder", async (note) => {
    await openSettings();
    await clickButton("Open Data folder");
    await sleep(1500);
    const alerts = await driver.findElements(By.css("[role=dialog] [role=alert]"));
    note(alerts.length ? `error shown: ${await alerts[0].getText()}` : "no error shown (opener succeeded or silently failed)");
    await shot("10-settings-open-folder");
    await closeSettings();
  });

  // ---------------------------------------------------------------- models
  await step("models-browse-unreachable", async (note) => {
    await openTab("Models");
    await waitText("Find models and style add-ons on CivitAI");
    // CivitAI is not reachable from CI sandboxes → expect a friendly error, not a raw reqwest dump.
    await visible("//input[@aria-label='Search CivitAI']", 15000);
    const tabText = () => driver.executeScript("return document.querySelector('#tab-models').innerText");
    await driver.wait(
      async () =>
        (await driver.findElements(By.css("#tab-models [role=alert]"))).length > 0 ||
        (await driver.findElements(By.css("#tab-models button[aria-label^='Install ']"))).length > 0 ||
        /Offline —|Nothing matches|No models/.test(await tabText()),
      60000,
      "browse neither loaded nor failed",
    );
    const alerts = await driver.findElements(By.css("#tab-models [role=alert]"));
    if (alerts.length) {
      const msg = await alerts[0].getText();
      note(`browse error: ${msg.replace(/\n/g, " | ")}`);
      assert(!/reqwest|error sending request|hyper|tcp connect|dns error/i.test(msg.split("\n")[0]), "raw transport error shown as the main message");
    } else note("browse returned without error (CivitAI reachable?)");
    await shot("11-models-browse-error");
  });

  await step("models-browse-offline", async (note) => {
    await openSettings();
    await toggleByLabel("Offline mode");
    await waitSetting("offline", "true");
    await closeSettings();
    await waitText(/offline/i, 15000);
    const t = await bodyText();
    note(`offline text: ${(t.match(/[^\n]*[Oo]ffline[^\n]*/g) || []).slice(0, 3).join(" | ")}`);
    await shot("12-models-browse-offline");
    // Every network call must fail before a socket opens.
    const e = await invoke("browse_catalog", { query: { kind: "models", look: null, content: "safe", price: "free", sort: "top_rated", period: "all_time", commercialOnly: false, compatibleOnly: true, query: "", cursor: null } }).then(
      (v) => ({ ok: true, v }),
      (err) => ({ ok: false, err: err.core }),
    );
    note(`browse_catalog while offline: ${JSON.stringify(e.ok ? { offline: e.v.offline, items: e.v.items?.length } : e.err)}`);
    await openSettings();
    await toggleByLabel("Offline mode");
    await waitSetting("offline", "false");
    await closeSettings();
  });

  await step("models-installed-empty", async (note) => {
    await openTab("Models");
    await click(`//div[@role='radiogroup']//button[starts-with(normalize-space(.),'Installed')]`);
    await waitText("Nothing installed yet");
    await shot("13-models-installed-empty");
    note(`installed.json exists: ${fs.existsSync(path.join(DATA, "catalog", "installed.json"))}`);
  });

  await step("downloads-popover", async (note) => {
    const btn = await driver.findElements(By.xpath("//header//button[@title='Downloads']"));
    if (!btn.length) {
      note("no downloads button (nothing downloaded this session)");
      return;
    }
    await btn[0].click();
    await waitText("Downloads");
    await sleep(400);
    const t = await bodyText();
    note(`popover: ${(t.match(/Downloads[\s\S]{0,200}/) || [""])[0].replace(/\n/g, " | ")}`);
    await shot("14-downloads-popover");
    await driver.actions().sendKeys(Key.ESCAPE).perform();
    await sleep(300);
  });

  await step("models-add-local-file", async (note) => {
    await openTab("Models");
    await click(`//div[@role='radiogroup']//button[starts-with(normalize-space(.),'Installed')]`);
    if (hasXdotool) {
      await clickButton("Add a file I already have");
      await answerNativeOpenDialog("Add a model file", FAKE_MODEL);
      note("native GTK file chooser answered via xdotool");
      await driver.wait(async () => /Added “|Which kind of model is this\?/.test(await bodyText()) || (await driver.findElements(By.css("#tab-models [role=alert]"))).length > 0, 30000, "no result after adding a file");
      await shot("15-models-add-file");
      const alert = await driver.findElements(By.css("#tab-models [role=alert]"));
      if (alert.length) throw new Error(`add file failed: ${await alert[0].getText()}`);
      if (/Which kind of model is this\?/.test(await bodyText())) {
        // SD 1.5 and SD 1.5 Hyper share tensor names → the user picks.
        note("family choice asked (sd15 vs sd15_fast)");
        await click(`//*[@role='dialog']//label[normalize-space(.)='Stable Diffusion 1.5']`);
        await clickButton("Add model");
        await waitText("Added “", 20000);
      }
    } else {
      // No xdotool → the native file chooser can't be answered; use the same IPC calls.
      note("xdotool missing: adding through add_local_model/confirm_family IPC directly");
      const r = await invoke("add_local_model", { path: FAKE_MODEL });
      if (r.needsChoice) await invoke("confirm_family", { token: r.needsChoice.token, familyId: "sd15" });
    }
    const models = await invoke("list_models");
    note(`list_models: ${models.map((m) => `${m.friendlyName} [${m.familyId}] missing=${JSON.stringify(m.missingComponents)}`).join("; ")}`);
    assert(models.length === 1, "model not registered");
    await shot("16-models-installed-one");
  });

  // ---------------------------------------------------------------- create
  await step("create-prompt-finetune", async (note) => {
    await openTab("Create");
    const box = await visible("//textarea[@id='prompt']", 15000);
    await box.click();
    await box.sendKeys(PROMPT);
    await clickButton("Fine-tune");
    await sleep(800);
    const t = await bodyText();
    note(`fine-tune fields: ${["Negative", "Sampler", "Scheduler", "Steps", "Seed", "Clip skip", "Hires", "VAE tiling", "Final prompt"].filter((f) => t.includes(f)).join(", ")}`);
    await shot("17-create-finetune");
  });

  await step("create-paste-civitai", async (note) => {
    await clickButton("Paste from CivitAI");
    const area = await visible("//textarea[@aria-label='Generation data']");
    // Type via JS + input event: sendKeys of long text is slow in WebKitWebDriver.
    await driver.executeScript(
      `const [el, v] = arguments; const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set;
       set.call(el, v); el.dispatchEvent(new Event('input', { bubbles: true }));`,
      area,
      CIVITAI_TEXT,
    );
    await sleep(500);
    await shot("18-create-paste-dialog");
    const applyBtn = await visible(`//*[@role='dialog']//button[contains(normalize-space(.),'Apply')]`);
    await applyBtn.click();
    await driver.wait(async () => /Applied|Skipped|Couldn|applied/i.test(await bodyText()), 30000, "no paste summary");
    await sleep(1500);
    const t = await bodyText();
    note(`summary: ${(t.match(/(Applied|applied)[\s\S]{0,400}/) || [""])[0].replace(/\n/g, " | ").slice(0, 400)}`);
    const prompt = await driver.findElement(By.id("prompt")).getAttribute("value");
    assert(prompt.includes(`${SENTINEL}_PASTE`), "pasted prompt not applied");
    await shot("19-create-paste-summary");
  });

  await step("create-generate-fake-model", async (note) => {
    // The fake SD 1.5 file can't load → the engine error path must be friendly.
    const btn = await visible(`//aside[@aria-label='Create settings']//button[contains(normalize-space(.),'Generate')]`);
    await btn.click();
    await driver.wait(
      async () => (await driver.findElements(By.css("aside[aria-label='Create settings'] [role=alert]"))).length > 0,
      180000,
      "no error after Generate with an unloadable model",
    );
    const msg = await driver.findElement(By.css("aside[aria-label='Create settings'] [role=alert]")).getText();
    note(`generate error: ${msg.replace(/\n/g, " | ").slice(0, 300)}`);
    await shot("20-create-generate-error");
  });

  await step("create-clear-session", async () => {
    await clickButton("Clear session");
    await sleep(800);
    const prompt = await driver.findElement(By.id("prompt")).getAttribute("value");
    assert(prompt === "", `prompt not cleared: ${prompt.length} chars left`);
    await shot("21-create-cleared");
  });

  // ---------------------------------------------------------------- edit + describe
  await step("edit-import", async (note) => {
    await openTab("Edit");
    await sleep(500);
    await shot("22-edit-empty");
    let t = await bodyText();
    note(`edit empty: ${(t.match(/Get the best edit model[^\n]*|Choose an image|Drop an image[^\n]*/g) || []).join(" | ")}`);
    await setFileInput(PNG, "#tab-edit");
    await sleep(1500);
    t = await bodyText();
    await shot("23-edit-imported");
    note(`after import: ${(t.match(/Restyle|Instruction|How much to change|Stay close to original|No edit model[^\n]*|edit model[^\n]*/g) || []).slice(0, 5).join(" | ")}`);
    assert(await driver.executeScript("return [...document.querySelectorAll('#tab-edit img')].some(i => i.src.startsWith('blob:') && i.naturalWidth > 0)"), "imported image not shown as a blob: image");
  });

  await step("describe-import", async (note) => {
    await openTab("Describe");
    await sleep(500);
    await shot("24-describe-empty");
    await setFileInput(PNG, "#tab-describe");
    await sleep(1500);
    const t = await bodyText();
    note(`describe: ${(t.match(/[^\n]*(captioner|describer|Get the|download)[^\n]*/gi) || []).slice(0, 4).join(" | ")}`);
    await shot("25-describe-imported");
    assert(await driver.executeScript("return [...document.querySelectorAll('#tab-describe img')].some(i => i.src.startsWith('blob:') && i.naturalWidth > 0)"), "imported image not shown");
  });

  // ---------------------------------------------------------------- privacy
  await step("privacy-scan-data", async (note) => {
    const webview = path.join(DATA, "webview");
    const hits = scanFor(DATA, SENTINEL, (p) => p.startsWith(webview));
    const wvHits = scanFor(webview, SENTINEL);
    const tmpHits = scanFor(os.tmpdir(), SENTINEL, (p) => p.startsWith(path.dirname(DATA)) || !/pinhole/i.test(p));
    const logHit = fs.readFileSync(APP_LOG).includes(SENTINEL);
    note(`Data/ hits: ${hits.length}; webview-profile hits (test-only, non-incognito): ${wvHits.length}; tmp hits: ${tmpHits.length}; app stdout/stderr hit: ${logHit}`);
    for (const h of [...hits, ...wvHits, ...tmpHits]) note(`  hit: ${h}`);
    assert(hits.length === 0 && tmpHits.length === 0 && !logHit, "sentinel prompt found on disk or in the app's output");
  });
} catch (e) {
  console.error("e2e: fatal", e);
  results.push({ name: "fatal", ok: false, error: String(e?.stack || e) });
} finally {
  await cleanup();
  const failed = results.filter((r) => !r.ok);
  fs.writeFileSync(path.join(OUT, "report.json"), JSON.stringify({ app: APP, data: DATA, results }, null, 2));
  console.log(`\ne2e: ${results.length - failed.length}/${results.length} steps passed. Screenshots + report.json in ${OUT}`);
  if (!KEEP_DATA) fs.rmSync(path.dirname(DATA), { recursive: true, force: true });
  process.exit(failed.length ? 1 : 0);
}
