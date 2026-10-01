# Pinhole — project brief

The one-page context for anyone (human or Claude) picking up Pinhole. Details live in
[`SPEC.md`](SPEC.md) (what), [`ARCHITECTURE.md`](ARCHITECTURE.md) (how the code is organised),
[`RELEASE-SPEC.md`](RELEASE-SPEC.md) (the safeguards and release rules every public build follows) and
[`../CLAUDE.md`](../CLAUDE.md) (working rules).

## What it is
A simple, private, offline AI image generator for Windows and Linux: pick a model, type a prompt,
press Generate. Tauri 2 + Rust core + React/TS UI. Images are made by
[stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp)'s `sd-server`; Describe uses
[llama.cpp](https://github.com/ggml-org/llama.cpp)'s `llama-server`. Both are pinned, SHA-256-verified
downloads (`config/engine.yaml`), never bundled. Your prompts and images stay on your computer.

## Status (v1.0.1)
- The app is feature-complete (milestones M0–M6 done). v1.0.0 is the first GitHub release
  (2026-10-01); v1.0.1 followed the same day (safety and model-trust fixes; Pinhole Licence 1.0).
  `RELEASE-SPEC.md` §12 lists what is still open. In-app updates open the release page until
  a signing key is set up (`src-tauri/update-key.pub` + the `PINHOLE_UPDATE_KEY` secret).
- Proven in CI on every full run: engine download + launch + real 256×256 generation on Windows and
  Ubuntu 24.04 (CPU), the app's own install → add model → wire → generate → save path, a
  WebDriver test that drives the real app, the privacy sentinel scan, Offline mode, installers.
- First real-GPU test (RTX 5070 Ti 16 GB, Windows 11): Z-Image Turbo bf16 failed when ~9 GB of VRAM
  was already used by another program → prompt encoding ran out of memory. Fixed, awaiting a real-GPU
  re-test: ≤ 16 GB now gets the Q8_0 model + Q8_0 GGUF text encoder (an already installed encoder
  option is still used), leftover engines are killed at start, other programs' VRAM is named, and an
  out-of-memory job retries with the text encoder on the processor, then with VAE tiling
  (Settings → Engine → "Read the prompt on the processor").

## Features
Create (dials: Shape, Quality, Stick to prompt, How many, Keep this look; Fine-tune drawer with every
engine setting), **Paste from CivitAI** (reads "Copy generation data" text in memory, fills prompt +
settings, matches or installs the checkpoint/LoRAs), Edit (instruction edit with Qwen-Image 2.1,
Qwen Image Edit or Kontext, Restyle, Fix details, Extend (wider/taller canvas), "Only change here" brush, undo chain, compare slider), Describe (sentence/tags),
Styles (the only user text ever stored), Presets (never the prompt), Models (CivitAI browser with
plain-language filters, one-click installs with component resolution, VRAM "Fits / Tight / Too big",
use a ComfyUI / A1111 / Forge models folder in place),
first-run "Recommended for your GPU", Settings (Offline mode, GPU/VRAM overrides, backend, content
mode, API key in the OS keychain, theme), portable mode (`Data/` next to the exe).

## Code map
`src-tauri/crates/`: `pinhole-registry` (models.yaml, header detection mirroring sd.cpp
`get_sd_version()`, wiring, VRAM), `pinhole-net` (the only HTTP client: allow-list, Offline mode,
resumable verified downloads), `pinhole-store` (Data folder, settings, installed.json, styles,
presets, keychain), `pinhole-hardware`, `pinhole-engine` (engine install, processes, sd-server /
llama-server clients, PNG scrub), `pinhole-catalog` (CivitAI), `pinhole-core` (service layer behind
every command). `src-tauri/src/commands/*` are thin Tauri wrappers. UI in `src/`
(`lib/api.ts` + `lib/types.ts` are the IPC contract; `lib/mock/` runs the UI in a plain browser).
Tests: crate unit tests, `tests/` (privacy, offline, engine smoke, app e2e), `tests/e2e/run.mjs`
(real app via WebDriver), vitest for UI logic.

## Key decisions (see SPEC §13 for the full list)
- Model knowledge is YAML (`config/models.yaml`); every URL + SHA-256 is verified against the live
  Hugging Face / GitHub APIs (verify-pins workflow). Gated repos are never used for one-click downloads.
- sd.cpp auto-fit places weights GPU → RAM (no `--offload-to-cpu`); no live TAESD preview (the server
  has no preview API); cancel during generation restarts the engine.
- Local by design: prompts only in RAM, `embed_image_metadata:false` + `--disable-image-metadata`
  + PNG text-chunk scrub, engines on 127.0.0.1, incognito WebView, WebView makes no network calls.
- Hardening: patched sd-server with a per-launch key that rejects browser requests
  (`ENGINE_LOCKDOWN`); engine stops after Reset / 5 min idle (it keeps results 600 s);
  llama-server per-launch API key, engine identity check, CivitAI files content-checked,
  imports re-encoded to PNG, downloads size-bounded, release builds refuse unpinned engines.
- Linux engine needs Ubuntu 24.04+ (the builds use glibc 2.38); Linux NVIDIA uses CUDA on RTX 30xx+ with the NVIDIA driver, else Vulkan.
- Windows engines need the VC++ runtime: bundled and copied next to the engine only when missing.

## Waiting on a real-GPU test (RTX 5070 Ti 16 GB, Windows 11)
CI runs on CPU only, so these are built but unproven:
- Z-Image Turbo (Q8 model + Q8 encoder) on 16 GB while another program holds ~9 GB of VRAM.
- The out-of-memory retries (text encoder on the processor, then VAE tiling) and the Settings toggle.
- Qwen-Image 2.1 (now the Realistic and Edit pick): Q6_K on 12 GB, Q8_0 on 16 GB, bf16 on 24 GB:
  speed, fit, edit quality, and the Q4_K edit pick on 6–10 GB (all figures are estimates).
- Windows: the leftover-engine sweep at start and the `nvidia-smi` "other programs" note.
- Defaults and VRAM figures for the new families (priority: FLUX.2 klein, Anima, Chroma, SD 3.5,
  Qwen-Image 2.1); int8 files on Vulkan.

Not checked against live CivitAI (the API probe can only reach `civitai.com`, not
`image.civitai.com`): the `width=450,optimized=true` card thumbnail and video-still URLs, and
whether CivitAI actually gzips its JSON. Check both in a real app build.

Behaviour note: Pinhole adds no prompt filter, prefix or negative prompt to Z-Image Turbo; what the
base model will draw is the model's own behaviour. CivitAI images made with a fine-tune need that
fine-tune installed to reproduce.

## CI, releases, repo habits
- GitHub Actions are manual only (Actions ran out of minutes; David, 2026-09-29): pushes and PRs run
  nothing. The required pre-merge check is `scripts/check.sh` (~2 min: privacy lint, vitest, tsc +
  build, cargo test, clippy). Actions → CI → Run workflow ("full") still covers Windows tests, engine
  smoke, app e2e, WebDriver e2e; installers on demand (Actions → Bundle) or via Release.
- Release: Actions → Release → Run workflow (tag `v<version>`, untick draft) — publishes a normal
  release (a pre-release only for versions like `1.1.0-rc.1`). Must be started by a person: Claude sessions can't
  create releases, push tags or delete branches.
- `main` is the only long-lived branch; branches are deleted as soon as their PR merges.
- Actions → API probe fetches CivitAI / Hugging Face URLs on a runner (sessions' containers can't
  reach those hosts).

## Open work
1. **Real-GPU checks** of everything under "Waiting on a real-GPU test" above, including defaults
   and VRAM figures for the newer families (Krea 2, Anima, Flux.1 Krea, Flux.2, Chroma, SD 3 / 3.5,
   HiDream-O1, ERNIE-Image, Mage-Flow, CivitAI int8 files). Not runnable: MiniMax H3 (video and
   audio only in the engine), "Qwen 2" (API-only on CivitAI).
2. **CivitAI browser** real-app check (Safe mode rules, paging, gzip, thumbnails).
3. Measure real VRAM on 8 / 12 / 16 GB cards (SPEC §14) and record observed peak VRAM.
4. The open safeguard and release items in `RELEASE-SPEC.md` §12 (false-positive re-measure,
   licence field on every download, signed updates and release files, Windows code signing,
   dependency review).
5. Optional: our own Ubuntu 22.04 engine build; live preview once sd-server supports it.

Never market Pinhole as "uncensored", "unfiltered" or "no one will know" (CLAUDE.md wording rules).
