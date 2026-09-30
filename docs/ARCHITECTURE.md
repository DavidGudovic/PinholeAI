# Pinhole — Architecture & build contract

`docs/SPEC.md` says *what* Pinhole does; this file says *how the code is organised*
and is the contract between the modules. Read SPEC.md and CLAUDE.md first.

## 1. Layout

```
Cargo.toml                 workspace root (MSRV-aware resolver, shared deps)
package.json, vite.config.ts, index.html
config/                    shipped YAML (bundled as Tauri resources → <resources>/config/)
  models.yaml engine.yaml catalog-filters.yaml styles/ presets/
src-tauri/                 Tauri 2 app crate `pinhole` (thin: commands + event bridge)
  src/lib.rs               builds AppCore, TauriSink, dispatch  (orchestrator)
  src/commands/<area>.rs   thin #[tauri::command] wrappers, one file per area
  crates/
    pinhole-registry/      models.yaml → families, header detector, wiring, VRAM, style combine (pure)
    pinhole-net/           THE http client (allow-list + offline), resumable verified downloads
    pinhole-store/         Data dir, settings.yaml, installed.json, styles, presets, keychain
    pinhole-hardware/      GPU vendor/VRAM/RAM detection
    pinhole-engine/        engine pins/install, sd-server + llama-server processes, API clients, PNG scrub,
                           Edit crop/blend helpers (detail.rs = Fix details, extend.rs = Extend)
    pinhole-catalog/       CivitAI API client, filters, safe-file selection, card view models
    pinhole-check/         local image check (RELEASE-SPEC §3): pinned ONNX files, rules, tract runner
    pinhole-core/          AppCore service layer (Tauri-free) used by commands and tests
tests/                     crate `pinhole-tests`: privacy sentinel scan, offline test, engine smoke test
src/                       React + TS UI
  lib/types.ts lib/api.ts  IPC contract (mirrors Rust serde camelCase)
  lib/mock/                browser mock backend (per-area handler tables)
scripts/                   CI helpers (privacy lint, pin verification) — Node, no Python
.github/workflows/         CI (build/test matrix), release packaging, pin verification
```

Why a workspace of small crates: each area compiles and tests independently
(`cargo test -p pinhole-net`), dependencies are explicit, and the privacy-critical
pieces (net, store) are small enough to audit.

## 2. Where things live (by area)

| Area | Files | Depends on |
|---|---|---|
| **registry** | `crates/pinhole-registry/**`, `config/models.yaml` (families, components, detect rules) | — |
| **net** | `crates/pinhole-net/**`, `core/src/downloads.rs`, `commands/downloads.rs` | — |
| **store / hardware** | `crates/pinhole-store/**`, `crates/pinhole-hardware/**`, `core/src/{app,library,update}.rs`, `commands/{app,library}.rs`, `config/presets/**`, `config/styles/**` | — |
| **engine / generate** | `crates/pinhole-engine/**`, `config/engine.yaml`, `core/src/{engine_setup,generate,describe,session,testing}.rs`, `commands/{generate,describe}.rs` | registry, net, store |
| **image check** | `crates/pinhole-check/**`, `core/src/imagecheck.rs` (fail closed at result intake) | net, store |
| **catalog / models** | `crates/pinhole-catalog/**`, `config/catalog-filters.yaml`, `core/src/{models,catalog}.rs`, `commands/{models,catalog}.rs` | registry, net, store |
| **UI shell + Create/Edit/Describe** | `src/App.tsx`, `src/components/**`, `src/tabs/{create,edit,describe}/**`, `src/lib/{paste,state}/**` | api.ts |
| **UI Models/Settings/First run** | `src/tabs/models/**`, `src/settings/**`, `src/firstrun/**` | api.ts |
| **IPC contract** | `src/lib/{api,types}.ts` ↔ Rust serde types (camelCase), `src/lib/mock/**` (browser mock backend, must mirror Rust behaviour) | — |
| **app shell (Rust)** | `src-tauri/src/{lib.rs,commands/mod.rs}`, `core/src/{lib,error,events}.rs` | all |
| **CI / tests / packaging** | `.github/**`, `scripts/**`, `tests/**`, `THIRD_PARTY_LICENSES` | all |

Conventions across areas:
- Public items in the crates are the cross-crate contract: extend freely, don't rename/remove/re-type
  without updating every caller. IPC changes update `types.ts`, the Rust type and the mocks together.
- Rust deps go in the crate's own `Cargo.toml`, preferring existing `workspace = true` entries.
- The upstream stable-diffusion.cpp source for the pinned engine is at `$SD_CPP_SRC` in Claude Code
  sessions (cloned by `.claude/hooks/session-start.sh`; server API `examples/server/api.md`, flags
  `examples/common/common.cpp`, detection `src/model_loader.cpp`, per-model `docs/*.md`).
- Session containers can't reach huggingface.co / civitai.com: write code + tests with fixtures, and
  check live data with the **API probe** workflow (see CLAUDE.md "Working in a Claude Code session").

## 3. Commands & events

The full command list, argument names and payload types are in `src/lib/api.ts` and
`src/lib/types.ts`. Rust command names are the snake_case names used there; each area's
`commands/<area>.rs` declares its commands with `super::area_commands![...]`
(no edits to `commands/mod.rs` needed). Command fns take
`core: tauri::State<'_, Arc<AppCore>>` and return `Result<T, CoreError>`; their bodies
call `pinhole_core::<area>::...` so tests can drive the same code without Tauri.

Binary IPC:
- `get_image`, `fetch_preview` return `tauri::ipc::Response::new(bytes)` → `ArrayBuffer` in JS.
- `import_image` receives the raw body: `fn import_image(core, request: tauri::ipc::Request<'_>)`
  and reads `request.body()` (`InvokeBody::Raw`).

Events (`pinhole_core::events::CoreEvent` → `TauriSink` → event name + payload):
`download-progress` (GroupStatus), `generation-progress` (GenerationProgress),
`engine-status` (EngineStatus), `models-changed` (no payload), `hardware-ready`.

Tagged payloads (so the UI never matches on labels):
- `GroupStatus.kind`: `engine` | `model` | `captioner` | `upscaler` (`null` only for groups
  queued without a kind) — e.g. the first-run screen finds the engine group by `kind`.
- `ResultImage.kind`: `generated` (txt2img / img2img / edit) | `upscaled` (`upscale_image`;
  model/seed/sampling copied from the source image, empty model id and seed 0 for an import).

Errors: every failure is a `CoreError { code, message, details }`. `message` says what to do
next ("Not enough VRAM — try the Fast setting or the smaller version of this model");
engine output goes in `details` (UI shows it behind a "Details" toggle).

## 4. Key flows

### First run
1. UI: `get_settings` → if `!firstRunDone` show FirstRun (frontend B).
2. `get_hardware` (detection runs in background at startup; `hardware-ready` event).
3. `engine_status` → `install_engine` (backend from hardware: NVIDIA→cuda, AMD/Intel→vulkan,
   none→cpu; Settings override). Progress arrives as `download-progress`.
4. `get_recommended` → cards per role; **Get** → `install_recommended(role)`; **Get all** =
   realistic + edit. Skippable. `set_settings({firstRunDone:true})`.

### Engine install (engine agent)
`config/engine.yaml` pins a release tag + per-platform assets + SHA-256. Install downloads
through `DownloadManager` into `Data/engine/sd/<version>/<backend>/`, verifies SHA-256,
unzips (zip-slip safe), marks executables (+x on Linux). Windows CUDA also needs the
`cudart-*` zip in the same folder. Upstream asset names (stable-diffusion.cpp CI):
`sd-master-<short>-bin-win-{cpu,cuda12,vulkan}-x64.zip`, `cudart-sd-bin-win-cu12-x64.zip`,
`sd-master-<short>-bin-Linux-Ubuntu-24.04-x86_64{,-vulkan}.zip` (no Linux CUDA build upstream:
Linux NVIDIA uses Vulkan).

### Generate (engine agent)
`generate(req)`:
1. Look up the installed model + family; resolve components (`wiring::required_components`),
   error `engine_missing`/`not_found` with a plain message if something's missing.
2. `wiring::launch_args` → if sd-server isn't running with exactly these args, stop it and start
   a new one (`--listen-ip 127.0.0.1 --listen-port <free port> --log-level warn` + args +
   `--lora-model-dir Data/models/loras`), emitting `generation-progress{phase: loadingModel}`.
   Wait for `GET /sdcpp/v1/capabilities` to answer.
3. `style::combine` (prompt + style + prefix + negatives, in memory), LoRA trigger words,
   `wiring::resolve_params` → `POST /sdcpp/v1/img_gen` with **`embed_image_metadata: false`**,
   structured `lora: [{path, multiplier}]`, `seed` (random if not locked; results get seed+i).
4. Poll `GET /sdcpp/v1/jobs/{id}` every ~300 ms; emit progress; step info parsed from the
   engine ring buffer if present. `cancel_generation` → `POST /sdcpp/v1/jobs/{id}/cancel`.
   sd-server has no auth and keeps finished jobs (images included) for 600 s, so an engine that
   ran a job is stopped on Reset and `IDLE_STOP_AFTER` (5 min) after the last
   generate/upscale. After `wait_ready`, `capabilities.model.path` must be the file we launched
   and our child must be alive (else "Another program is using Pinhole's engine port").
   Out of memory (the engine output of the job shows it; `pinhole_engine::failure::memory_failure`):
   each fallback at most once — prompt encoding → restart with `--backend te=cpu` (merged into any
   `--backend` list the wiring emits; remembered per model for the app session in RAM; Settings
   `textEncoderOnCpu: auto|on|off`), VAE / unknown stage → `--vae-tiling`, then (denoising: right
   away) `--max-vram -4` instead of the `-2` a 16 GB card launches with (`vram_reserves`: forces
   sd.cpp's segmented execution when a model only just fits; remembered per model, engine note),
   then `--offload-to-cpu` when every weight fits in RAM + 2 GB (else denoising gets tiling; kept while the
   same model runs with the same wiring args, `GenState::offloaded` → engine note); then `CoreError{code:"vram"}`
   (message names other programs using the card, engine output in `details`, after the auto-fit
   memory plan kept from this model's last launch, `pinhole_engine::failure::memory_plan`).
   Before every launch (sd-server and llama-server): previous engine fully exited, leftover engines
   under `Data/engine/` killed (`pinhole_engine::orphans`; also at app start; never other programs
   or engines this app runs), an idle Describe engine stopped and NVIDIA memory used by other
   programs measured (`nvidia-smi`, a progress note when it's a lot).
5. Decode base64 → strip every PNG text chunk (tEXt/zTXt/iTXt) defensively → store in
   `Session` (RAM) → return `ResultImage`s. Nothing touches disk until `save_image`.

`save_image`: `Data/outputs/pinhole_YYYYMMDD_HHMMSS_<seed>.png`; if Settings
`savedMetadata == "settings"`, add ONE tEXt chunk `pinhole` with model, seed, steps, cfg,
sampler, scheduler, size — never prompt/negative/style text.

### Edit (engine agent backend, frontend A UI)
- Instruction edit: edit family (role `edit`), `ref_images[0]` = source; "Stay close to original"
  → the family's `stay_close_maps_to`; optional mask → `mask_image`.
- Restyle: current Create model, `init_image` + `strength` (0.35/0.55/0.75).
- Result images carry `parentId` for the in-memory undo chain.

### Browse CivitAI (catalog)
`browse_catalog(query, forFamily)` → `core::catalog::browse` → `pinhole_catalog::browse::browse`
(`forFamily`, style add-ons for one installed model: `baseModels` narrowed to
`families::lora_base_models` and cards to versions made for them via `cards::OnlyBaseModels`): asks
`GET /api/v1/models` (`limit=50`, always `nsfw=true`, repeated `baseModels`/`types` keys, cursor
paging; JSON requested gzip-compressed) through `cache::CachedSource` (RAM-only, 12 answers / 5 min,
compacted), turns each model into a card or a hidden count (`filters::hidden_by` → `safe::SafeFilter`
for Safe mode, then Look, Tags, commercial use, price, compatibility) and keeps fetching until
24 cards or 1 + 5 requests (`partial` → "Load more"). A newer Browse request stops an older one's
extra requests (`cancelled`). Card previews are `width=450,optimized=true` CDN URLs (video → still
frame); `fetch_preview` returns their bytes. UI (`src/tabs/models/lib/`): `pageStore.ts` (RAM page
cache + shared in-flight requests, next page prefetched), `previewQueue.ts` / `preview.ts` (8
fetches at a time, on-screen first, queued fetches dropped when a card scrolls away, 48 MB RAM LRU).

### Install from CivitAI / registry (catalog agent)
`plan_civitai_install(versionId, fileId?)` → pick file (SafeTensor/GGUF, primary preferred, both
scans `Success`, else `blockedReason`; a smaller file that Fits when the usual one doesn't, or the
user's `fileId` from the Size choice, `select::select_file_for_machine`), family via `baseModel` (→ hash/known file → ask), components
missing (matched by component id / SHA-256), sizes, free disk, VRAM fit, `fileOptions`. `install_civitai(versionId, familyId, fileId?)` enqueues
one download group (model + missing components); on success registers every file in
`installed.json` and emits `models-changed`. 401/403 → `CoreError{code:"unauthorized"}` and the UI
asks for an API key (keychain). LoRAs store `trainedWords`.

### Models from another app (catalog agent)
`add_linked_folder(path)` / `remove_linked_folder(id)` / `rescan_linked_folders()` /
`list_linked_folders()` (`pinhole-core/src/linked.rs`). The pure part is
`pinhole-catalog/src/linked.rs`: `walk` (every .safetensors/.gguf, links followed, tool and
unusable-kind folders skipped), `read_note` (CivitAI data other apps saved next to a file),
`recognise` (header → family via note / name hints / base family; LoRA family via note / kohya
metadata / name hints; parts only by kind + size + SHA-256). Found files are ordinary
`InstalledFile`s in `core.installed` whose `rel_path` is `linked/<folder id>/<path in folder>`:
`InstalledIndex::abs_path` resolves them in the folder, `save_to` leaves them out of the shared
index and `save_linked` writes them (with size/mtime stamps) to `Data/catalog/linked-folders.json`.
Delete refuses them, orphan cleanup and the Models-folder move skip them. A linked add-on is
hard-linked / symlinked / copied into `models/loras/.pinhole-linked/` when a picture uses it
(`lora_path_for_engine`), emptied at start. `install_missing_parts(modelId)` downloads the
registry parts a model lacks (models without a CivitAI version).

### Paste from CivitAI (frontend A + catalog agent)
CivitAI's image page has a **Copy generation data** button producing A1111-style text:
```
<prompt lines>
Negative prompt: <negative lines>
Steps: 30, Sampler: DPM++ 2M Karras, CFG scale: 7, Seed: 123, Size: 832x1216, Clip skip: 2,
Model hash: 1a2b3c4d5e, Model: foo, Hires upscale: 1.5, Denoising strength: 0.4,
Lora hashes: "name: abcdef123456", Civitai resources: [{"type":"checkpoint","modelVersionId":1,...}], ...
```
A **Paste from CivitAI** button in Create (and Ctrl+V of such text into the prompt box) parses it
in the WebView (`src/lib/paste/`, pure + unit-tested, never logged/stored), fills prompt,
negative, steps, CFG/guidance, sampler+scheduler (mapped to sd.cpp names), seed, size, clip skip,
hires, then calls `resolve_civitai_resources(resources)` (ids/hashes only — no prompt) to select the
installed checkpoint/LoRAs or offer one-click installs. A summary lists what was applied and
what was skipped.

## 5. Privacy implementation rules (enforced by tests + `scripts/privacy-lint.mjs`)
- Prompt-bearing types: `GenerateRequest`, `FineTune` (negativePrompt), `FinalPrompt`, the
  engine request body, pasted text. They are `Deserialize` from IPC and serialized ONLY into the
  loopback HTTP request body. Never pass them to `std::fs`, `serde_yaml::to_*`, `serde_json::to_writer`,
  any logging macro, `println!/eprintln!/dbg!`, or into `CoreError`.
- No `log`/`tracing` crates. No `console.*` in `src/` except `console.error` of CoreError `code`.
- Engine stdout/stderr → in-memory ring buffer (~200 lines) only; lines containing the prompt
  (or any of its lines / comma-separated parts of 8+ characters) or anything after
  `-p`/`prompt` are redacted before storage.
- `LocalClient` only talks to `127.0.0.1`; `HttpClient` is the only internet client.
- llama-server gets a random per-launch API key through its environment (`LLAMA_API_KEY`, never
  argv) and every request sends `Authorization: Bearer <key>`. sd-server has no auth (see §4).
- Downloads: `DownloadSpec.size_bytes` is exact or `None` (rounded `size_mb`/`sizeKB` go in
  `approx_size_bytes`); every download is bounded (exact size + 1 % + 1 MiB, else 64 GiB);
  CivitAI files must pass `content_check` (safetensors/GGUF header parses) or are deleted.
- The WebView makes no network calls: CSP `connect-src ipc: http://ipc.localhost`, images are
  `blob:` URLs from bytes returned by Rust.

## 6. Testing expectations
- Every crate: unit tests for its logic (`cargo test -p <crate>`), no network in unit tests
  (use local mock servers on 127.0.0.1 where a server is needed — e.g. a tiny tokio TCP server).
- `tests/` (ci agent): privacy sentinel test using a mock sd-server that echoes the prompt into a
  PNG tEXt chunk (proves scrubbing + no disk writes), offline-mode test, engine smoke test
  (`PINHOLE_SMOKE=1`, real sd-server + tiny model, 256×256 on CPU) run in CI.
- Frontend: vitest for `src/lib/**` logic (paste parser, dial math), `npm run build` must pass.

## 7. Cross-area hooks (stubbed in code — implement, don't rename)
| Function | Implemented by | Used by |
|---|---|---|
| `core::app::hw_context(core) -> HwContext` | store | engine, catalog |
| `core::models::register_download(core, &DownloadedFile, Registration)` | catalog | engine (captioner, upscaler, engine files are NOT registered) |
| `core::describe::install_captioner(core)` | engine | catalog (recommended "describe" role) |
| `core::describe::{list_helper_models, install_captioner(core, helper_id)}` | engine | frontend B (Models → Helpers), frontend A (pickers) |
| `core::describe::improve_prompt(core, prompt, family_id, avoid)` | engine | frontend A (Create prompt box) |
| `core::generate::unload_model(core, model_id)` | engine | catalog (delete) |
| `core::testing::{use_external_engine, register_fake_model}` (feature `test-util`) | engine | ci (`tests/`) |
| `pinhole_engine::testutil::MockSdServer` (feature `test-util`): `start().await`, `base_url()`, `requests()` | engine | ci (`tests/`) |
| `pinhole_net::HttpClient::new_for_tests(offline, allow_loopback_http)` (feature `test-util`) | net | ci, engine, catalog tests |
| `src/firstrun/RecommendedCards.tsx` `RecommendedCards({roles, compact, showGetAll})` | frontend B | frontend A |
| `src/tabs/models/ModelsTab.tsx`, `src/settings/SettingsSheet.tsx`, `src/firstrun/FirstRun.tsx` | frontend B | frontend A (shell) |
| `src/components/ui/*` primitives | frontend A | frontend B (import only) |

Core service functions the `tests/` crate calls (names fixed):
`core::generate::generate(&Arc<AppCore>, GenerateRequest) -> CoreResult<GenerateResult>`,
`core::session::save_image(&AppCore, id) -> CoreResult<SavedImage>`,
`core::library::save_style(&AppCore, Style) -> CoreResult<Style>`,
`core::library::save_preset(&AppCore, Preset) -> CoreResult<Preset>`.
Rust request/response types mirror `src/lib/types.ts` field-for-field (serde camelCase),
including the contract-audit additions `GroupStatus.kind` (`DownloadKind`:
`engine|model|captioner|upscaler`, set via `DownloadManager::enqueue_kind`) and
`ResultImage.kind` (`ResultKind`: `generated|upscaled`) — see §3.
`core::session::clear(&AppCore)` is `async` (it may stop sd-server, see §4 Generate).
