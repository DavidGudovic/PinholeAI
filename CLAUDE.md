# CLAUDE.md — working rules for Pinhole

Pinhole is a simple, private, offline AI image generator (Tauri 2 + Rust + React/TS) that
drives stable-diffusion.cpp (`sd-server`) and llama.cpp (`llama-server`) as sidecar processes.

**Read `docs/SPEC.md` before any work.** It is the source of truth. If you need to deviate,
update the spec in the same PR and explain why.

Pinhole is a **personal test build** for now. No build is shared with anyone until everything in
`docs/RELEASE-SPEC.md` is done.

## Repo layout (target)
```
src-tauri/        Rust core (registry, detector, wiring, engine, catalog, downloads, presets, hardware)
src/              React + TypeScript UI
config/           shipped YAML: models.yaml, catalog-filters.yaml, engine.yaml, presets/
docs/SPEC.md      product + technical spec
docs/RELEASE-SPEC.md  what must be done before any build is shared (marking, safety checks, licences)
docs/ARCHITECTURE.md  crates, IPC contract, flows, where each area lives
docs/PROJECT-BRIEF.md status, decisions, roadmap (start here)
tests/            Rust integration tests + privacy tests; tests/e2e/ drives the real app (WebDriver)
scripts/          check.sh (pre-merge check), privacy lint, pin verification, packaging (Node, no Python)
.claude/          session start hook for Claude Code on the web
.github/          CI (manual), Bundle, Release, Verify pins, API probe; issue + PR templates
```

## How to work
- Build milestone by milestone (SPEC §11). One PR per milestone, or smaller.
- Each PR: `scripts/check.sh` passes (Actions CI is manual, see below), a manual full CI run
  when the change is Windows-specific, short PR description with screenshots for UI changes.
- Keep the default UI minimal. New options go in the **Fine-tune** drawer unless the spec says otherwise.
- Prefer boring, well-maintained crates. No Python anywhere in the product.
- Keep the four choke points from RELEASE-SPEC §1 as one function each (request builder, Edit
  input intake, result intake incl. previews, export for Save/Copy) so release safeguards can be
  added without a refactor.

## Working in a Claude Code session
- **Start with** `docs/PROJECT-BRIEF.md` (status, decisions, roadmap), then SPEC.md and
  `docs/ARCHITECTURE.md` (where each area lives). Feature requests and bugs come in as GitHub issues
  (templates in `.github/ISSUE_TEMPLATE/`).
- The session start hook (`.claude/hooks/session-start.sh`) installs the Tauri Linux deps, runs
  `npm install`, builds `dist/`, pre-compiles the Rust tests and clones the pinned
  stable-diffusion.cpp source to `$SD_CPP_SRC` (read-only reference). It runs in the background, so
  a build or test right after the session starts can fail on missing deps: wait and try again.
- **Required pre-merge check: `scripts/check.sh`** (~2 min warm). It runs the privacy lint
  (+ self-test), `npm test`, `npm run build` (includes `tsc`), `cargo test --workspace --locked`
  and `cargo clippy --workspace --all-targets -- -D warnings`, and stops at the first failure.
  Run it on the branch (rebased/merged on current `main`) and merge only when it passes; say so in
  the PR. `scripts/check.sh --smoke` adds the CPU engine smoke + app e2e (needs internet for the
  engine and a tiny model, so not from the cloud container). A bug fix comes with a regression test.
  `cargo fmt` isn't enforced yet (the repo isn't fmt-clean).
- **Flow:** work on your session branch → `scripts/check.sh` → PR to `main` (use
  `.github/pull_request_template.md`) → merge → the branch is deleted. One PR per issue or milestone.
- **GitHub Actions are manual only.** Pushes and PRs run nothing (no minutes spent). Actions → CI →
  Run workflow (`full`, optional `installers`) covers what can't run locally: Windows tests, the
  WebDriver e2e, Linux + Windows engine smoke and test installers. Run it before a release or after
  Windows-specific changes (engine/process handling, hardware, data dir, updater, packaging).
  Release still builds on a `v*` tag or Actions → Release → Run workflow.
- **Blocked hosts:** the session container can't reach huggingface.co or civitai.com. Check live
  data with the **API probe** workflow (GitHub MCP `actions_run_trigger`, workflow `api-probe.yml`,
  ref `main`, inputs `urls` = space-separated GET URLs on civitai.com / huggingface.co /
  api.github.com and an optional `jq` filter; read the output with `get_job_logs`). Never guess a
  model URL, size or SHA-256 — verify it this way (or with the verify-pins workflow).
- **Things a session can't do** (GitHub refuses them for this integration): create releases, push
  tags, delete branches, change repo settings. Ask the user (e.g. "Actions → Release → Run workflow").
- Real-GPU behaviour (CUDA/Vulkan, VRAM fitting) can't be tested in the container or CI (CPU only).
  Say what is untested and ask the user to try a build; keep error Details useful for that.

## Non-negotiable privacy rules
1. Never write prompt or negative-prompt text to disk, logs, presets, filenames, PNG metadata,
   crash output or `installed.json`. Don't `println!`/`log::*`/`console.log` request bodies.
   **Only exception:** a Style the user explicitly saves goes to `Data/styles/`. Styles and the
   main prompt are separate fields and are only combined in memory at request time.
2. Always send `"embed_image_metadata": false` to `sd-server` (its default is `true`).
3. Generated images stay in memory (Rust `Vec<u8>` / JS Blob) until the user clicks Save.
4. No telemetry, analytics, crash reporters, automatic update checks, remote fonts or CDN assets.
   The only update check is the user pressing **Check for updates** (SPEC §4 rule 6), through the
   Rust client below; no updater plugin.
5. All network calls go through ONE Rust HTTP client wrapper that enforces Offline mode and
   an allow-list of hosts: `civitai.com`, `huggingface.co`, `github.com` (+ their download CDNs).
   The WebView makes no network calls of its own; CivitAI preview images are fetched through
   the Rust client and handed to the UI as blobs.
6. Engines bind to `127.0.0.1` only. Engine stdout/stderr go to an in-memory ring buffer only.
7. CivitAI API key and the optional GitHub token (updates while the repo is private) live in the
   OS keychain (`keyring` crate), never in `Data/`. Secrets go only in the `Authorization` header.

**Privacy tests (must exist and pass in CI):**
- Generate with a sentinel prompt (e.g. `PINHOLE_SENTINEL_7f3a`) plus a saved style, save the
  image, create a preset, then recursively scan `Data/` (and the OS temp dir for Pinhole files)
  → the sentinel prompt must not appear anywhere (including PNG chunks); the style text may
  appear only in `Data/styles/`.
- Static check: a script that fails CI if request/prompt structs derive `Serialize` into any
  file-writing path, or if any logging macro receives a prompt field.
- Offline mode test: with Offline mode on, every network call returns an error before a
  socket opens.

## Registry rules (`config/models.yaml`)
- Model knowledge belongs in YAML, not Rust. Code reads families, components, flags, dials.
- Before release every `sha256: TODO` and every URL must be verified by actually downloading.
- Verify CivitAI `baseModel` strings against the live API
  (`https://civitai.com/api/v1/enums` or real `/api/v1/models` responses); fix the YAML.
- Header detection must mirror `get_sd_version()` in stable-diffusion.cpp
  `src/model_loader.cpp` for the pinned engine version. Note the loader prefixes standalone
  diffusion files with `model.diffusion_model.`; match on suffixes.
- Families that share tensor names (Flux dev/schnell/Kontext; Qwen-Image vs Qwen-Image-Edit;
  SDXL vs Pony/Illustrious) are disambiguated by known hash → CivitAI metadata → asking the user.

## Engine rules
- Pinned versions in `config/engine.yaml`. Verify SHA-256 before first launch.
- Use the native async API: `POST /sdcpp/v1/img_gen`, poll `GET /sdcpp/v1/jobs/{id}`,
  cancel with `POST /sdcpp/v1/jobs/{id}/cancel`. Read `/sdcpp/v1/capabilities` after launch.
- LoRAs go in the structured `lora` array; `<lora:...>` prompt tags are not supported by the server.
- Switching model = restart `sd-server` with new component paths. Show progress; never block the UI thread.
- Keep an engine smoke test: download the smallest registered model (CI cache), generate one
  256×256 image on CPU, assert a PNG comes back.

## Security rules for downloads
- Only `SafeTensor` and `GGUF` from CivitAI; reject `PickleTensor`/`.ckpt`/`.pt`/`.pth`.
- Require CivitAI `pickleScanResult == Success` and `virusScanResult == Success`.
- Resumable downloads, SHA-256 verify, check free disk space first, write to `*.part` then rename.
- Parse safetensors headers defensively (cap header size, e.g. 100 MB; reject malformed JSON).

## UX rules
- Plain words in the UI: "Stick to prompt", "Stay close to original", "How much to change",
  "Fits / Tight / Too big". Technical names appear only in the Fine-tune drawer.
- Every automatic choice can be seen and overridden in Fine-tune.
- Errors must say what to do next ("Not enough VRAM — try the Fast setting or the smaller
  version of this model"), not dump engine output. Engine output is behind a "Details" toggle.

## Wording rules (docs, README, UI, commits, release notes)
- Describe privacy as "your prompts and images stay on your computer". Never "leaves no trace",
  "untraceable", "no one will know", "uncensored", "unfiltered", "undress", "nudify", "face swap".
- Privacy copy states facts (what stays on the computer, what is saved and when, what goes online).
  Don't frame it as hiding what someone made ("forgets everything", "wipes your tracks", "nobody
  will see"). The top-bar control is **Reset**. No one-click adult-content shortcuts in the UI.
- Edit examples change scenes, objects, lighting or style — never a real person's body or clothes
  while keeping their face.
- Screenshots and examples: safe for work, fictional subjects, no real people.

## Git
- Conventional commits (`feat:`, `fix:`, `docs:`…).
- `main` is the only long-lived branch. Delete a branch as soon as its PR is merged (the repo has
  "Automatically delete head branches" on). If a branch can't be deleted from the current
  session, tell the user which ones to delete.
- Never commit or bundle model weights, engine binaries, or anything under `Data/`. Models are
  always one-click downloads.
