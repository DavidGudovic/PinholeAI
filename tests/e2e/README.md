# Real-app end-to-end test (Linux)

`run.mjs` drives the **built Pinhole binary** — real Rust core, real Tauri IPC, real
WebKitGTK — through [tauri-driver](https://v2.tauri.app/develop/tests/webdriver/) and
WebKitWebDriver. Unlike the browser mocks (`src/lib/mock`), nothing is faked: the engine
is really downloaded from GitHub, SHA-256 checked and unpacked, settings land in
`Data/config/settings.yaml`, and (optionally) a real `sd-server` makes a real image.

## Setup (once)

```sh
sudo apt-get install -y webkit2gtk-driver xvfb xdotool   # + the usual Tauri build deps
cargo install tauri-driver --locked
```

`xdotool` answers the native GTK "Add a model file" chooser (WebDriver can't reach native
dialogs and Tauri's IPC internals are read-only). Without it the test adds model files
through the same IPC calls instead.

## Build and run

```sh
npm ci && npm run build
cargo build -p pinhole --features tauri/custom-protocol   # debug build that serves dist/
xvfb-run -a -s "-screen 0 1440x960x24" node tests/e2e/run.mjs
```

A plain `cargo build` makes a dev binary that loads `http://127.0.0.1:1420` (the Vite dev
server); `tauri/custom-protocol` embeds `dist/` instead. Use a **debug** build: WebKitGTK
can only be automated through a non-ephemeral web context, so debug builds switch the
window's incognito mode off when tauri-driver sets `TAURI_WEBVIEW_AUTOMATION=true`
(`src-tauri/src/lib.rs`, `under_webdriver()`); the profile then lives in `Data/webview/`.
Release builds are always incognito and can't be driven.

The WebDriver client (`selenium-webdriver`) is installed on first run into
`$PINHOLE_E2E_DEPS` (default `<tmp>/pinhole-e2e-deps`); nothing is added to `package.json`.

| Env | Default | |
|---|---|---|
| `PINHOLE_APP` | `target/debug/pinhole` | app binary |
| `PINHOLE_E2E_OUT` | `target/e2e` | screenshots, `report.json`, `app.log` (the app's stdout/stderr) |
| `PINHOLE_E2E_DATA` | fresh temp folder, deleted after | Data folder (kept when set) |
| `PINHOLE_E2E_ENGINE` | `1` | `0` skips the real engine download |
| `PINHOLE_E2E_MODEL` | unset | real generation: path to an SD 1.5 `.safetensors`, or `zero` |
| `PINHOLE_E2E_ONLY` | all | regex of step names (later steps rely on earlier ones) |
| `PINHOLE_E2E_PORT` | `4444` | tauri-driver port (`+1` for WebKitWebDriver) |

`PINHOLE_E2E_MODEL=zero` synthesizes a **zero-weight SD 1.5** checkpoint (2.1 GB sparse
file) from the installed engine's own tensor list — `sd-cli` reports every missing tensor
and every expected shape — so the full generate → progress → result → Save path runs with
no model download. It renders a flat grey 256×256 image in ~30 s on 4 CPU cores. Adding it
copies 2.1 GB into `Data/models/`; on a small disk point `PINHOLE_E2E_DATA` at a tmpfs
(e.g. `/dev/shm/pinhole-e2e/Data`).

## What it covers

First run (welcome → real hardware detection → real engine download with rendered progress
→ `get_recommended` cards → Done), Create empty state, every Settings control (persisted to
`settings.yaml`, Offline on/off, theme, keychain error), Models Browse (CivitAI unreachable
→ friendly error; Offline → offline state), Installed (empty → add a file through the
native chooser → family question → delete), downloads popover, Create (prompt, Fine-tune,
save a Style + a Preset, Final-prompt preview, Paste from CivitAI with a real A1111 string,
engine error with Details, Reset), optional real generation + Save (PNG has no
text chunks), Edit/Describe image import, a model download cancelled mid-connect, and a
**privacy scan**: the sentinel prompt must not appear anywhere under `Data/`, in Pinhole
temp files or in the app's own output; the Style text only in `Data/styles/`.

Exit code is non-zero if any step fails; `report.json` lists every step with notes.
