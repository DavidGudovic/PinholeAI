# Pinhole

A simple, private, offline AI image generator. Pick a model, type a prompt, press **Generate**.

Pinhole is for people who want good local image generation without learning ComfyUI:
no node graphs, no Python, no jargon. Everything is decided for you — and every
decision can be seen and changed in the **Fine-tune** drawer.

- **Auto-wiring.** Every model just works: VAE, text encoders and the best settings are picked
  from a model registry (`config/models.yaml`), not guessed.
- **Best model for your GPU, one click.** First run detects your GPU and recommends the best
  Realistic, Anime, Edit and Describe models that fit. Every model shows how much VRAM it needs
  (**Fits / Tight / Too big**).
- **Built-in CivitAI browser** with plain-language filters (Realistic · Anime · Illustration · 3D ·
  Brand & product), safe-only by default, paid/early-access hidden by default, SafeTensor/GGUF only.
- **Create** (text → image), **Edit** ("keep the face, change the shirt to a navy hoodie"),
  **Describe** (image → prompt), reusable **Styles** and **Presets**.
- **Private by construction.** Never stores your prompts. No telemetry. Works fully offline
  once models are downloaded.
- **Windows 10/11 and Linux.** Powered by [stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp)
  (`sd-server`) and [llama.cpp](https://github.com/ggml-org/llama.cpp) (`llama-server`) —
  NVIDIA (CUDA, incl. RTX 50xx), AMD/Intel (Vulkan), or CPU.

## Privacy

What Pinhole promises (enforced by tests in CI, see [Tests](#tests)):

- **Your prompts are never written anywhere** — not to disk, logs, presets, file names, PNG
  metadata or crash output. The engine's output is kept in a small in-memory buffer, with your
  prompt redacted.
- **Generated images stay in memory** until you click **Save**. Closing the app or
  **Clear session** discards them. Saved files are named `pinhole_YYYYMMDD_HHMMSS_<seed>.png` and
  carry no metadata unless you turn on "Include generation settings (no prompt)".
- **The one exception is Styles:** text you explicitly save as a named Style is stored in
  `Data/styles/`. Styles and prompts are separate fields, combined only in memory.
- **No telemetry, analytics, crash reporting or update checks.** No remote fonts or CDNs.
- **Network traffic happens only when you start it** — browsing CivitAI, downloading a model or
  the engine — and only to `civitai.com`, `huggingface.co` and `github.com` (plus their download
  CDNs). The user interface itself makes no network calls. **Offline mode** (Settings) blocks
  every request before a connection is opened.
- The engines listen on `127.0.0.1` only. The optional CivitAI API key lives in your OS keychain.

Honest limitations:

- Your operating system may page memory to swap / the pagefile, and Pinhole cannot control that.
  If that matters to you, use full-disk encryption (BitLocker, LUKS).
- On Windows, the WebView2 runtime that draws Pinhole's window keeps its own browser cache under
  `%LOCALAPPDATA%\app.pinhole.desktop` (also in portable mode). Pinhole never puts prompts or
  images there.

## Install

Download the latest build from the [Releases](../../releases) page (or, for development builds,
from the **Artifacts** section of a green [CI run](../../actions/workflows/ci.yml)). Check the
download with `SHA256SUMS.txt` (`sha256sum -c SHA256SUMS.txt`, or `Get-FileHash` on Windows).

### Windows 10 / 11 (64-bit)

- **Installer:** `Pinhole-<version>-windows-x64-setup.exe` — installs for the current user (no
  admin rights). It also installs the Microsoft Edge **WebView2** runtime if it is missing.
- **Portable:** `Pinhole-<version>-windows-x64-portable.zip` — unzip anywhere you can write to
  (e.g. a USB drive), run `Pinhole\Pinhole.exe`. The `Data\` folder next to it makes Pinhole
  portable: models, engine, settings and saved images all stay in that folder.
  Needs WebView2: Windows 11 already has it; on Windows 10 install the
  [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/) if
  Pinhole doesn't open.

Windows SmartScreen may warn about an unrecognized app (builds are not code-signed yet):
choose **More info → Run anyway**.

### Linux (x86_64)

- **AppImage:** `chmod +x Pinhole-<version>-linux-x86_64.AppImage && ./Pinhole-<version>-linux-x86_64.AppImage`
- **Debian/Ubuntu:** `sudo apt install ./Pinhole-<version>-linux-amd64.deb`

**Ubuntu 24.04 or newer is required for image generation:** the upstream stable-diffusion.cpp
Linux builds that Pinhole downloads are built on Ubuntu 24.04 and need glibc ≥ 2.38. The app
itself is built on Ubuntu 22.04 and starts there, but the engine will not run on 22.04.
Upstream ships no Linux CUDA build, so NVIDIA GPUs on Linux use the Vulkan engine
(install your distribution's Vulkan driver, e.g. `mesa-vulkan-drivers` or the NVIDIA driver).

## First run

1. Pinhole detects your GPU and VRAM (Settings can override both).
2. It downloads the matching image engine **once** (CUDA for NVIDIA, Vulkan for AMD/Intel, CPU
   otherwise — the CPU engine works but is very slow). Every download is SHA-256 verified.
3. **Recommended for your GPU:** one card per role (Realistic, Anime, Edit, Describe) with
   download size and VRAM needed. Click **Get** (or **Get all**), or skip and browse CivitAI in
   the **Models** tab. Models are 2–25 GB each; nothing is bundled with the app.
4. Type what you want to see and press **Generate** (Ctrl+Enter).

## Where your data lives

Everything Pinhole writes is in one folder called `Data/` (**Settings → Open Data folder**):

| Mode | Location |
|---|---|
| Portable (Windows zip) | `Data\` next to `Pinhole.exe` |
| Installed, Windows | `%LOCALAPPDATA%\Pinhole\Data` |
| Installed, Linux | `~/.local/share/pinhole/Data` (`$XDG_DATA_HOME`) |

```
Data/
  models/      checkpoints, diffusion models, text encoders, VAEs, LoRAs, upscalers, captioners
  outputs/     images you saved (only when you click Save)
  styles/      styles you saved (the only user text Pinhole stores)
  presets/     your presets (model + style reference + settings; never the prompt)
  config/      settings.yaml, overrides.yaml (your registry overrides)
  catalog/     installed.json (files, hashes, families — no prompts)
  engine/      downloaded sd-server / llama-server builds
```

Uninstalling does not delete `Data/` — remove it yourself to free the disk space.

## Build from source

Prerequisites: [Rust](https://rustup.rs) stable (≥ 1.88), Node.js 22, and the
[Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/):

- **Windows:** Microsoft C++ Build Tools (MSVC) and WebView2 (preinstalled on Windows 11).
- **Ubuntu/Debian:**
  ```sh
  sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
    libsoup-3.0-dev libssl-dev libxdo-dev libdbus-1-dev pkg-config patchelf build-essential file
  ```

```sh
npm ci
npm run tauri dev      # run the app with hot reload
npm run tauri build    # release build + installers in target/release/bundle/
npm run dev            # UI only, in a browser, against a mock backend (no Rust needed)
```

To produce the same file names as CI (incl. the Windows portable zip):
`node scripts/package.mjs --platform windows` (or `linux`) after `npx tauri build`.

## Tests

```sh
npm test                                  # frontend unit tests (vitest)
node scripts/privacy-lint.mjs             # static privacy check (CLAUDE.md), --self-test proves each rule
cargo test --workspace                    # Rust unit tests + tests/ (privacy + offline)
PINHOLE_SMOKE=1 cargo test -p pinhole-tests --test engine_smoke --release -- --nocapture
                                          # real engine: downloads sd-server + SD 1.5 (~2.2 GB,
                                          # cached in target/smoke-cache), one 256×256 image on CPU
```

- `tests/tests/privacy.rs` — generates with a sentinel prompt, negative prompt and a saved style
  (against a mock engine that bakes the prompt into PNG metadata, like sd-server's default), saves
  images and a preset, then scans every byte of `Data/` and Pinhole's temp files, including
  compressed PNG text chunks: the prompt must appear nowhere, the style only in `Data/styles/`.
- `tests/tests/offline.rs` — with Offline mode on, every network call fails before a socket
  opens (a local listener counts zero connections); non-allow-listed hosts are refused.
- `tests/tests/engine_smoke.rs` — the real pinned engine end to end (Linux smoke runs on Ubuntu 24.04).
- `scripts/privacy-lint.mjs` — fails CI if logging macros see prompt fields, prompt types reach
  file-writing code, `log`/`tracing`/telemetry/updater dependencies appear, the UI uses
  `console.*`, browser storage near prompts, remote assets, or the CSP allows remote origins.

## CI and releases

- **CI** (`.github/workflows/ci.yml`, every push/PR): frontend tests + build + privacy lint,
  `cargo test` on Windows and Ubuntu, then installers. Downloads on each run's Summary page:
  `pinhole-windows-x64` (setup exe + portable zip + SHA256SUMS), `pinhole-windows-x64-portable`
  (the unzipped portable app), `pinhole-linux-x64` (AppImage + deb + SHA256SUMS). The engine
  smoke test runs on pushes to `main` and on demand.
- **Bundle** (`bundle.yml`): installers only, can be started by hand.
- **Release** (`release.yml`): push a tag `v<version>` matching `src-tauri/tauri.conf.json`
  (e.g. `git tag v0.1.0 && git push origin v0.1.0`) → GitHub Release with all files and
  `SHA256SUMS.txt`.
- **Verify pins** (`verify-pins.yml`, on `config/**` changes or by hand): checks every model and
  engine URL + SHA-256 in `config/*.yaml` against Hugging Face / GitHub, CivitAI `baseModel`
  strings against the live API, and suggests values for remaining `TODO`s.

## Project layout

```
src-tauri/        Tauri app + Rust crates (registry, net, store, hardware, engine, catalog, core)
src/              React + TypeScript UI
config/           shipped YAML: models.yaml, engine.yaml, catalog-filters.yaml, styles/, presets/
tests/            workspace integration tests (privacy, offline, engine smoke)
scripts/          CI helpers (Node, no dependencies): privacy lint, pin verification, packaging
docs/             SPEC.md (what), ARCHITECTURE.md (how)
```

Contributing: read [`CLAUDE.md`](CLAUDE.md) and [`docs/SPEC.md`](docs/SPEC.md) first.
Model knowledge belongs in `config/models.yaml`, not in code.

## Status

Version 0.1 — in active development (milestones M0–M5 in [`docs/SPEC.md`](docs/SPEC.md#11-milestones)).
Expect rough edges; model recommendations and VRAM numbers are still being measured.

## License

MIT — see [LICENSE](LICENSE). Third-party licenses (stable-diffusion.cpp, ggml, llama.cpp and the
libraries compiled into Pinhole) are in [THIRD_PARTY_LICENSES](THIRD_PARTY_LICENSES).
Model weights are not part of Pinhole; each model's license is shown on its card
(e.g. FLUX.1-dev is non-commercial).
