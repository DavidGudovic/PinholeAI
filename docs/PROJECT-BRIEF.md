# Pinhole — project brief

The one-page context for anyone (human or Claude) picking up Pinhole. Details live in
[`SPEC.md`](SPEC.md) (what), [`ARCHITECTURE.md`](ARCHITECTURE.md) (how the code is organised),
[`RELEASE-SPEC.md`](RELEASE-SPEC.md) (what must happen before anything is shared) and
[`../CLAUDE.md`](../CLAUDE.md) (working rules).

## What it is
A simple, private, offline AI image generator for Windows and Linux: pick a model, type a prompt,
press Generate. Tauri 2 + Rust core + React/TS UI. Images are made by
[stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp)'s `sd-server`; Describe uses
[llama.cpp](https://github.com/ggml-org/llama.cpp)'s `llama-server`. Both are pinned, SHA-256-verified
downloads (`config/engine.yaml`), never bundled. Your prompts and images stay on your computer.

## Status (v0.1.0, personal test build)
- Milestones M0–M5 implemented on `main`. M6 (release readiness, `RELEASE-SPEC.md`) not started —
  **no build is shared with anyone until it is done.**
- Proven in CI on every full run: engine download + launch + real 256×256 generation on Windows and
  Ubuntu 24.04 (CPU), the app's own install → add model → wire → generate → save path, a
  WebDriver test that drives the real app, the privacy sentinel scan, Offline mode, installers.
- First real-GPU test (RTX 5070 Ti 16 GB, Windows 11): Z-Image Turbo bf16 failed when ~9 GB of VRAM
  was already used by another program → prompt encoding ran out of memory. Fixed, awaiting a real-GPU
  re-test: ≤ 16 GB now gets the Q8_0 model + Q8_0 GGUF text encoder (an already installed encoder
  option is still used), leftover engines are killed at start, other programs' VRAM is named, and an
  out-of-memory job retries with the text encoder on the processor, then with VAE tiling
  (Settings → Engine → "Run the text encoder on the processor").

## Features
Create (dials: Shape, Quality, Stick to prompt, How many, Keep this look; Fine-tune drawer with every
engine setting), **Paste from CivitAI** (reads "Copy generation data" text in memory, fills prompt +
settings, matches or installs the checkpoint/LoRAs), Edit (instruction edit with Qwen Image Edit /
Kontext, Restyle, "Only change here" brush, undo chain, compare slider), Describe (sentence/tags),
Styles (the only user text ever stored), Presets (never the prompt), Models (CivitAI browser with
plain-language filters, one-click installs with component resolution, VRAM "Fits / Tight / Too big"),
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
- Privacy by construction: prompts only in RAM, `embed_image_metadata:false` + `--disable-image-metadata`
  + PNG text-chunk scrub, engines on 127.0.0.1, incognito WebView, WebView makes no network calls.
- Hardening: engine stops after Reset / 5 min idle (upstream sd-server has no auth and keeps
  results 600 s), llama-server per-launch API key, engine identity check, CivitAI files content-checked,
  imports re-encoded to PNG, downloads size-bounded, release builds refuse unpinned engines.
- Linux engine needs Ubuntu 24.04+ (upstream builds use glibc 2.38); Linux NVIDIA uses Vulkan.
- Windows engines need the VC++ runtime: bundled and copied next to the engine only when missing.

## Waiting on a real-GPU test (RTX 5070 Ti 16 GB, Windows 11)
CI runs on CPU only, so these are built but unproven:
- Z-Image Turbo (Q8 model + Q8 encoder) on 16 GB while another program holds ~9 GB of VRAM.
- The out-of-memory retries (text encoder on the processor, then VAE tiling) and the Settings toggle.
- Krea 2 Turbo on 12 and 16 GB: speed and whether it fits.
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
- CI is tiered for the private repo's Actions minutes: every push/PR ≈ 4 min (Linux tests + frontend
  + privacy lint); push to `main` adds Windows tests, engine smoke, app e2e, WebDriver e2e (≈ 40 billed
  min); docs-only changes run nothing; installers only on demand (Actions → Bundle) or via Release.
- Release: Actions → Release → Run workflow (tag `v<version>`, untick draft) — publishes a
  **pre-release** marked "personal test build". Must be started by a person: Claude sessions can't
  create releases, push tags or delete branches.
- `main` is the only long-lived branch; branches are deleted as soon as their PR merges.
- Actions → API probe fetches CivitAI / Hugging Face URLs on a runner (sessions' containers can't
  reach those hosts).

## Open work / roadmap
1. **VRAM robustness on real GPUs** (built, awaiting a real-GPU re-test): see Status. Untested in
   CI: CUDA/Vulkan behaviour, the Windows leftover-engine sweep and `nvidia-smi` on Windows.
2. **More model families** (registry entries done, not yet run on a real GPU): Krea 2 (Turbo is
   the second "Realistic" one-click pick on 12 GB+), Anima, Flux.1 Krea, Flux.2 (dev, klein 4B/9B
   + base), Chroma, Qwen-Image 2.1, SD 3 / 3.5, HiDream-O1, ERNIE-Image, Mage-Flow; CivitAI int8
   (ComfyUI int8_tensorwise) files now install. Not runnable: MiniMax H3 (video + audio only in the
   engine), "Qwen 2" (API-only on CivitAI). Needs real-GPU checks of defaults and VRAM figures.
3. **CivitAI browser** (built, awaiting a real-app check): Safe only = Stability Matrix's default
   (hide CivitAI-flagged models, PG previews only) plus YAML tag / name / sample-rating rules tuned on
   live data; opens on Most downloaded · All time; full pages (client-side filters fetch more, then
   "Load more"); gzip JSON, CivitAI's own 450 px card renditions, RAM caches, prefetch.
4. **Engine auth patch** (decided 2026-09-28: before any shared build, not now): build a patched
   sd-server in CI that rejects browser requests and requires a per-launch token. Listed in the
   RELEASE-SPEC §12 checklist.
5. Measure real VRAM on 8 / 12 / 16 GB cards (SPEC §14) and record observed peak VRAM.
6. Code signing (e.g. Azure Trusted Signing) to remove SmartScreen warnings.
7. M6 / `RELEASE-SPEC.md`: AI-generated marking, local image check + guard LLM, licence acceptance,
   terms, SAFETY.md.
8. Optional: our own Ubuntu 22.04 engine build; live preview once sd-server supports it.

## Positioning (for later, after M6)
"Local AI images that just work — no nodes, no Python; your prompts and images stay on your
computer." Audiences: freelancers/agencies/e-commerce (client work stays local, commercial-use filter,
licences shown), RTX owners who bounced off ComfyUI/A1111, CivitAI users (Paste from CivitAI),
AMD/Intel GPU owners (Vulkan), indie game devs/writers. Channels: r/StableDiffusion, r/LocalLLaMA,
Show HN, Product Hunt, stable-diffusion.cpp's README/Discord, YouTube reviewers, winget/Scoop/Flathub.
Never market it as "uncensored/unfiltered/no one will know" (see CLAUDE.md wording rules).
