# Pinhole — Product & Technical Spec (v1)

Pinhole is a local, offline AI image generator for people who don't want to learn ComfyUI.
You pick a model, type a prompt, press Generate. Everything else is decided for you, and
every decision can be overridden.

---

## 1. Principles (in priority order)

1. **Private by construction.** Prompts are never written anywhere. No telemetry, no analytics,
   no crash reporting, no update pings. The only network traffic is traffic the user starts
   (browsing CivitAI, downloading a model or engine).
2. **Zero-knowledge default path.** A new user never has to know what a VAE, text encoder,
   sampler, scheduler or CFG is.
3. **Light and fast.** Small installer, low idle RAM, no Python, no bundled browser.
4. **Data, not code, for model knowledge.** Everything model-specific (components, defaults,
   dial ranges) lives in YAML so it can be updated without touching code.
5. **Windows 10/11 and Ubuntu 24.04+ are first-class.** macOS is out of scope for v1.
   (The app itself runs on 22.04, but the pinned upstream Linux engine builds need glibc 2.38 —
   see §13.)

---

## 2. Architecture

```
┌──────────────────────── Pinhole (Tauri 2 app) ────────────────────────┐
│  UI: React + TypeScript + Vite + Tailwind (WebView)                    │
│      images held as in-memory Blob URLs only                           │
│                          │ Tauri commands / events                     │
│  Core: Rust                                                           │
│   • registry     – loads config/*.yaml (+ Data/config/overrides.yaml) │
│   • detector     – reads safetensors/GGUF headers, identifies family   │
│   • wiring       – resolves model → components + flags + defaults      │
│   • engine mgr   – spawns/stops sd-server & llama-server sidecars      │
│   • catalog      – CivitAI API client + filters                        │
│   • downloads    – resumable, SHA-256 verified, disk-space checked     │
│   • presets      – YAML presets in Data/presets                        │
│   • hardware     – GPU vendor + VRAM + RAM detection                   │
└─────────────┬─────────────────────────────────┬──────────────────────┘
              │ HTTP on 127.0.0.1:<random port>  │
     ┌────────▼─────────┐              ┌─────────▼─────────┐
     │ sd-server         │              │ llama-server       │
     │ (stable-diffusion │              │ (llama.cpp, VLM    │
     │  .cpp, MIT)       │              │  for img2text)     │
     └───────────────────┘              └────────────────────┘
```

### Why these choices
- **stable-diffusion.cpp (`sd-server`)**: MIT license, single native binary, no Python.
  Supports SD1.5, SDXL (incl. Pony/Illustrious finetunes), Flux.1, Z-Image, Qwen-Image,
  and instruction-edit models (Flux.1 Kontext, Qwen Image Edit 2509/2511). Backends: CUDA,
  Vulkan, CPU. Its native async API (`POST /sdcpp/v1/img_gen`, poll `/sdcpp/v1/jobs/{id}`)
  returns images as base64 in the response, so nothing touches disk.
- **llama.cpp (`llama-server`)**: MIT, runs vision-language models (GGUF + mmproj) for img2text.
- **Tauri 2**: ~10 MB shell using the system WebView (WebView2 on Windows, WebKitGTK on Linux).
- **Rust core**: memory-safe process/download/file handling; easy cross-compilation.

### Engine lifecycle
- `sd-server` loads **one model set per process**. Switching model = stop process, start a new
  one with the new component paths. Show a friendly "Loading <model>… (~10–30 s)" state.
- Bind to `127.0.0.1` on a random free port. Never `0.0.0.0`.
- Run with `--log-level warn`. Pipe stdout/stderr into an in-memory ring buffer (last ~200
  lines) for error display. **Never write engine logs to disk.** Strip anything after `-p`/
  `prompt` if a line would ever contain it.
- Always send `"embed_image_metadata": false` in `img_gen` requests (default is `true`,
  which would bake the prompt into the PNG).
- Only one diffusion model is resident at a time (VRAM). The captioner runs on demand and
  is shut down after 60 s idle.

### Engine binaries
- Pinned version + per-platform download URLs + SHA-256 in `config/engine.yaml`.
- First-run setup detects GPU and downloads the matching build once:
  NVIDIA → CUDA 12.x build (must support Blackwell / RTX 50xx), AMD/Intel → Vulkan build,
  no GPU → CPU build (warn: very slow).
- Stored in `Data/engine/{sd,llama}/<version>/<backend>/`. Verify hash before first launch.
- Linux has no upstream CUDA build of `sd-server`: NVIDIA on Linux uses the Vulkan build.
- Windows: the upstream builds need the MSVC runtime (VC++ 2015–2022 x64); Pinhole bundles the
  redistributable DLLs and copies them next to an engine when the system lacks them.

---

## 3. Data folder

All app files live in one folder called `Data/`:

- **Portable mode**: if a writable `Data/` folder exists next to the executable, use it
  (Windows zip release).
- **Installed mode**: `%LOCALAPPDATA%\Pinhole\Data` (Windows), `~/.local/share/pinhole/Data` (Linux).
- Settings has an **"Open Data folder"** button.

```
Data/
  models/
    checkpoints/      all-in-one files (SD1.5/SDXL-style)
    diffusion/        diffusion-only files (Flux, Z-Image, Qwen)
    text_encoders/
    vae/
    loras/
    upscalers/
    captioners/
  outputs/            only written when the user clicks Save
  presets/            *.yaml, one per preset
  styles/             *.yaml, one per user-saved style
  config/
    settings.yaml     app settings (no prompts, no history)
    overrides.yaml    user overrides merged over the shipped registry
  catalog/
    installed.json    index of installed files: path, sha256, family, civitai ids
  engine/
```

**Forbidden in Data/**: prompts, negative prompts, generation history, thumbnails of
unsaved images, logs.

---

## 4. Privacy rules (hard requirements — tests must enforce them)

1. No prompt text is ever written to disk, logs, crash dumps, presets, file names, or PNG metadata.
2. Generated images live in RAM until the user clicks **Save**. Closing the app discards them.
3. **Clear session** button: drops all in-memory images and prompt fields immediately.
4. No outbound network except: CivitAI API calls, model/engine downloads, and Hugging Face
   component downloads — all started by the user.
5. **Offline mode** toggle (Settings): blocks all network calls at the Rust HTTP client
   layer. The catalog shows "Offline" and only installed models.
6. No telemetry SDKs, no auto-update checks, no remote fonts/CDNs in the UI (bundle everything).
7. The CivitAI API key (optional) is stored in the OS keychain (`keyring` crate), never in `Data/`.
8. Saved file names: `pinhole_YYYYMMDD_HHMMSS_<seed>.png`. Never derived from the prompt.
9. Saved-image metadata: **none** by default. Optional setting "Include generation settings
   (no prompt)" writes model name, seed, steps, dials into a PNG text chunk.
10. CI check: grep-based test fails the build if any code path writes a `prompt` field to a
    file or log (see CLAUDE.md).
11. **The one exception is Styles** (§7): text the user explicitly saves as a named Style is
    stored in `Data/styles/`. That is a deliberate user action, clearly labelled
    ("Saved styles are stored on this computer"). The main prompt is never stored, and
    nothing is ever saved as a style automatically.

Honest limitation to put in the README: the OS may page RAM to swap/pagefile; Pinhole cannot
control that.

---

## 5. Screens

Four tabs: **Create**, **Edit**, **Describe**, **Models**. Plus a Settings sheet.

### 5.1 Create (txt2img)

Default view shows only:
- the model picker (installed models, with a friendly name and a style badge)
- the prompt box ("What do you want to see?")
- the **Style** picker next to it: *None* or a saved style (§7). A small "＋ Save as style"
  link turns the current style text into a named style.
- **Generate** (Ctrl/Cmd+Enter)
- the result area

The **prompt** says *what* (subject, scene). The **style** says *how it looks* (e.g. "35mm film,
soft window light, shallow depth of field"). They are combined only at request time (§7), so the
same style can be reused on any prompt and any model.

If no model is installed, the Create tab shows the **Recommended models** card instead (§6.1).

Under the prompt, a single row of **simple dials** (all values come from the registry for the
active model family):

| Dial | UI | Maps to |
|---|---|---|
| Shape | chips: Square · Portrait · Landscape · Wide | width/height from `resolutions` |
| Quality | 3-stop slider: Fast · Balanced · Best | `steps` (and hires fix at Best if the family allows it) |
| Stick to prompt | slider: Loose ↔ Strict | `cfg` within `cfg_range`; hidden when the family is fixed-CFG (e.g. `cfg: 1`) |
| How many | 1 · 2 · 4 | `batch_count` |
| Keep this look | toggle | locks the seed of the selected result |

**Fine-tune** drawer (collapsed by default): negative prompt (only for families that use it),
sampler, scheduler, steps, CFG, seed, flow shift, clip skip, hires fix, LoRAs with weights,
VAE tiling. Each field shows the registry default and a "reset" button.

Result card actions: **Save** · **Edit this** · **Describe** · **Variations** (same prompt,
new seeds) · **Upscale 2×/4×** · **Copy to clipboard**.

**Paste from CivitAI**: CivitAI's "Copy generation data" button yields A1111-style text (prompt,
`Negative prompt:`, `Steps: …, Sampler: …, CFG scale: …, Seed: …, Size: …, Clip skip: …, Civitai
resources: [...]`). A **Paste from CivitAI** button next to the prompt (and pasting such text into
the prompt box) parses it in memory, fills prompt, negative, steps, CFG/guidance, sampler +
scheduler (mapped to sd.cpp names), seed, size, clip skip and hires, selects the installed
checkpoint/LoRAs (matched by CivitAI version id or hash) or offers one-click installs, and shows
what was applied and what was skipped. The pasted text is never stored or logged.

Live preview: if a TAESD file is registered for the family, show a low-res preview while
generating. Progress bar + **Cancel** (`POST /sdcpp/v1/jobs/{id}/cancel`).

### 5.2 Edit (img2img + instruction editing)

Entry points: **Edit this** on any result, drag-and-drop, paste from clipboard, file picker.
The image comes in as an in-memory buffer (never copied into `Data/`).

Two modes, picked automatically:

1. **Instruction edit** (default when an edit model is installed): the user types what to change:
   "keep the same face, change the shirt to a navy hoodie" or "keep the logo, replace the mug
   with a water bottle". Uses the edit family (Qwen Image Edit 2511 preferred, Flux.1 Kontext
   as the lower-VRAM option) with the image passed as `ref_images[0]`.
   - Dial: **Stay close to original** (maps to the family's guidance setting).
   - Optional **"Only change here"** brush: paint a mask → `mask_image`.
2. **Restyle** (classic img2img with the current Create model): image as `init_image`.
   - Dial: **How much to change** (Subtle · Medium · Strong → `strength` 0.35/0.55/0.75).

If no edit model is installed, the Edit tab shows one card: "Get the best edit model for your
GPU" — one-click download of the top edit model that fits (§6.1), showing its download size
and VRAM need.

**Edit chain**: each edit result can be edited again. Keep an in-memory undo stack
(original → edit 1 → edit 2…) with a before/after comparison slider.

### 5.3 Describe (img2text)

- Drop an image → **Describe**.
- Output styles: **Sentence** (natural description, good for Flux/Z-Image/Qwen) and **Tags**
  (comma-separated booru-style tags, good for anime SDXL models). Both come from a VLM prompt
  template in `config/models.yaml → captioner`.
- Buttons: **Use as prompt** (sends to Create) · **Copy**.
- Captioner resolution: if the Qwen2.5-VL 7B text encoder + mmproj for the edit model are
  installed, reuse them (no extra download). Otherwise offer the small default captioner.

### 5.4 Models (CivitAI browser + installed models)

Two sub-views: **Browse** and **Installed**.

#### Browse — filters with plain-language labels

| Filter | Labels shown to the user | API mapping (`GET /api/v1/models`) |
|---|---|---|
| Kind | Models · Style add-ons | `types=Checkpoint` · `types=LORA` |
| Look | Realistic · Anime · Illustration · 3D · Brand & product | tag sets from `config/catalog-filters.yaml` |
| Content | Safe only · Include 18+ · 18+ only | `nsfw=false` (default) · `nsfw=true` · `nsfw=true` + keep only items with `model.nsfw == true` |
| Price | Free (default) · Include early access (paid) · Early access only | free = drop models whose latest version is in early access; paid items are **hidden by default** |
| Sort | Top rated · Most downloaded · Newest | `sort=Highest Rated / Most Downloaded / Newest` |
| Time | This week · This month · This year · All time | `period` |
| Commercial use | Any · OK for client work | `allowCommercialUse` includes `Image` |
| Compatibility | Works with Pinhole (default on) | `baseModels=` every family in the registry |
| Search | free text | `query` |

- Paging with `cursor` (page×limit > 1000 returns 429).
- 18+ modes require a one-time confirmation per session (stored in RAM only).
- Default content mode is **Safe only**. When 18+ is off, also blur any preview image flagged NSFW.
- "18+ only" and "Free" are partly client-side filters: keep fetching pages until the grid is
  full (cap at 5 extra requests per scroll, then show "Load more").
- Verify the exact early-access fields on real API responses before relying on them
  (`earlyAccess` query param, version `availability` / early-access end date).

**Model card** shows: preview image, name, friendly style badge, rating (thumbs-up ratio and
download count), download size, **VRAM needed** (§6.2) with a Fits / Tight / Too big badge for
this GPU, price badge (only when paid models are shown), commercial-use badge, and the base
model in small text.

**Install** button:
1. Pick the best file: primary, `SafeTensor` or GGUF format only. **Never PickleTensor.**
   Require `pickleScanResult == Success` and `virusScanResult == Success`.
2. Resolve the family (§6) and list the extra components needed (VAE, text encoders),
   skipping any already installed (matched by SHA-256).
3. Show the total download size and a free-disk-space check, then download everything with
   resume + SHA-256 verification.
4. If CivitAI answers 401/403, prompt for an API key (explain why; optional; stored in keychain).
5. For LoRAs, save trigger words from the version's `trainedWords` into `installed.json`,
   and offer a toggle "Add trigger words automatically".

#### Installed
List with friendly name, family, size, last used, **Delete** (removes orphaned components too,
after confirmation), and **Add a file I already have** (pick a .safetensors/.gguf in the file
chooser → detected). Dropping files onto the window is not supported: the native drop handler
is disabled so HTML5 image drag-and-drop works in Edit/Describe on Windows.

---

## 6. Auto-wiring (model → working pipeline)

Resolution order for "what is this file":
1. **Known hash**: SHA-256 is found in `config/models.yaml → known_files` → exact family + variant.
2. **CivitAI metadata**: the version's `baseModel` maps to a family via `civitai_base_models`.
3. **Header sniffing**: read only the safetensors JSON header / GGUF metadata (no tensor data)
   and match tensor names against each family's `detect` rules. Mirror the logic of
   `get_sd_version()` in stable-diffusion.cpp `src/model_loader.cpp`, and keep our rules
   in YAML.
4. **Ask**: if still ambiguous (e.g. Flux.1 dev vs Kontext, or Qwen-Image vs Qwen-Image-Edit
   share tensor names), show a picker with the candidate families.

Also detect: all-in-one vs diffusion-only (does the file contain VAE/text-encoder tensors?),
dtype/quant (fp16/bf16/fp8/Q4_K…) for VRAM estimates.

Then the **wiring** step builds the `sd-server` command line from the family entry:
components (`--vae`, `--clip_l`, `--t5xxl`, `--llm`, `--llm_vision`), flags
(`--diffusion-fa`, `--offload-to-cpu`, `--vae-tiling`, `--model-args ...`) and the default
request parameters.

**Hardware-aware flags** (`config/models.yaml → hardware_profiles`): VRAM tiers choose
offload/tiling flags and, where the registry lists several quants, the recommended quant to
download. The 5070 Ti (16 GB) tier should run Z-Image Turbo bf16 and Qwen Image Edit 2511
Q4_K_M with CPU offload.

**Updating model knowledge**: edit `config/models.yaml` (shipped with the app) or add entries in
`Data/config/overrides.yaml` (deep-merged, user wins). No code changes needed for a new
finetune of a known family.

### 6.1 Recommended models (one-click, best that fits)

**No model weights ship with the app.** Everything is a one-click download.

- `config/models.yaml → recommended` holds a ranked list per **role**: Realistic, Anime,
  Edit, Describe. Each candidate has a download spec and its VRAM needs.
- For each role Pinhole picks the **first (best) candidate whose `vram_gb.min` fits this GPU**,
  choosing the best quant that fits (bf16 → Q8 → Q4). The last Realistic candidate is the
  small SD 1.5, so PCs **without a usable GPU** (and cards under 5 GB) still get a one-click
  model: there, candidates are sized against system RAM instead (§6.2).
- **First run**: after the engine download, show "Recommended for your GPU (16 GB)" (or
  "Recommended for your computer" without a usable GPU) with one
  card per role: model name, what it's good at, download size, VRAM needed, and a **Get** button.
  A **Get all** button downloads the Realistic + Edit picks. Skippable.
- The same card appears wherever a role is empty (Create with no models, Edit with no edit
  model, Describe with no captioner).
- Shared components (VAE, text encoders) are downloaded once and reused across models, and the
  card's download size counts only what is actually missing.

### 6.2 VRAM needed (shown everywhere a model is shown)

Every model card, installed-model row and recommendation shows **"Needs ~X GB VRAM"** plus a
badge against the detected GPU:

- **Fits** — `X ≤ VRAM − 1 GB` headroom
- **Tight** — fits only with CPU offload / VAE tiling (Pinhole enables them automatically; slower)
- **Too big** — will not run acceptably; install button warns before downloading

Without a usable GPU (engine backend `cpu`, or no known VRAM; a VRAM override only counts with a
GPU backend) the figure is **"Needs ~X GB memory"**: weights of the model and its components +
activations. The badge is **Slow** ("runs on the processor — slow", i.e. Tight) when the family is
marked `cpu_friendly` (SD 1.5) or the weights are ≤ 4 GB, and it leaves ≥ 4 GB of RAM free;
everything else is **Too big**. The UI then says "your computer", not "your GPU".

How X is computed:
1. **Known models**: `vram_gb: { min, recommended }` from the registry (measured, not guessed).
2. **Unknown models** (CivitAI, dropped-in files): estimate =
   diffusion weights size + VAE + text encoders that stay on GPU (per family `flags`) +
   activation overhead for the family's default resolution (`activation_gb` in the registry).
   Label it "~X GB (estimate)".
3. After a real run, record the observed peak VRAM for that file in `installed.json`
   (a number only) and show the measured value from then on.

VRAM detection: `nvidia-smi` for NVIDIA; DXGI adapter memory on Windows and sysfs on Linux for
AMD/Intel (no Vulkan loader needed); manual
override in Settings.

---

## 7. Styles & presets

### Styles
A **Style** is reusable look-and-feel text, kept separate from the prompt.

- Fields: `name`, `positive` (e.g. "35mm film photo, soft window light, shallow depth of
  field, natural skin texture"), optional `negative` (e.g. "cartoon, plastic skin"), optional
  `thumbnail` (only if the user picks one of their images for it), optional `families`
  (which model families it's written for; empty = all).
- Stored as `Data/styles/<slug>.yaml`. Built-in styles ship in `config/styles/` (read-only):
  e.g. "Film photo", "Studio product shot on white", "Anime cel shading", "Watercolor".
- Picker next to the prompt box: *None* + list with thumbnails. Manage styles (rename, edit,
  duplicate, delete) in a small dialog.
- **Combining at request time** (never stored): per family, a `style_template` in the registry
  decides how prompt + style are joined:
  - natural-language families (Flux, Z-Image, Qwen): `"{prompt}. Style: {style}"`
  - tag-based families (SDXL, Pony, Illustrious, SD1.5): `"{prompt}, {style}"`
  - `auto_prompt_prefix` (e.g. Pony score tags) is added in front, if enabled.
  - style `negative` is appended to the family's default negative prompt (only for families
    that use negatives).
- The Fine-tune drawer shows a read-only "Final prompt sent to the model" preview so the user
  can see exactly what was combined.
- In the Edit tab, a style can be applied to an instruction edit ("make it look like: {style}")
  or to Restyle.

### Presets
- **Save as preset** from Create or Edit: name + model + style (by reference) + all dial values +
  Fine-tune values + LoRAs.
- **Never includes the main prompt.** Style text lives in the Style, not in the preset.
- Stored as `Data/presets/<slug>.yaml`, human-editable.
- Preset picker next to the model picker. Applying a preset for an uninstalled model offers
  a one-click download.
- Built-in presets shipped in `config/presets/` (read-only): e.g. "Photo portrait",
  "Anime illustration", "Product shot on white".

---

## 8. Settings

- Offline mode
- Data folder location (portable / installed) + Open folder
- GPU override (auto / pick device / force CPU) and a VRAM tier override
- Default content mode (Safe only / Include 18+ / 18+ only)
- Show paid (early access) models (off by default)
- Saved-image metadata (None / Settings without prompt)
- CivitAI API key (set / remove; keychain)
- Theme (system / light / dark)

---

## 9. Performance targets

- Installer: < 30 MB. **No model weights and no engine binaries are bundled**; both are
  one-click downloads on first run.
- Idle RAM of the app with no model loaded: < 150 MB.
- Time from Generate click to request sent: < 50 ms (model already loaded).
- UI stays responsive during generation and downloads (all heavy work off the UI thread).
- Result images are decoded once and kept as Blobs; thumbnails are generated in a worker.

---

## 10. Packaging

- Windows: NSIS installer + portable zip (with an empty `Data/` folder → portable mode).
- Linux: AppImage + .deb (built on Ubuntu 22.04; engine needs 24.04+, see §13).
- GitHub Actions matrix build for both; release artifacts with SHA-256 sums.
- License: MIT (compatible with stable-diffusion.cpp and llama.cpp). Include their license
  files in `THIRD_PARTY_LICENSES`.
- Model licenses are the user's concern, but show the license name on the model card
  (e.g. FLUX.1-dev is non-commercial).

---

## 11. Milestones

**M0 – Skeleton**: Tauri + React app, Data folder resolution, settings, hardware detection,
engine download + launch of `sd-server`, health check.

**M1 – Generate**: registry loader, header detector, wiring, VRAM estimate, Create tab with
simple dials and Style field, in-memory results, Save, Cancel, Clear session. First-run
"Recommended for your GPU" screen with one-click download (§6.1).

**M2 – Models tab**: CivitAI browse with all filters (incl. 18+ only, paid hidden by default),
model cards with VRAM needed, install flow with component resolution, Installed view, delete.

**M3 – Edit**: instruction edit (Qwen Image Edit 2511 / Kontext), restyle img2img, mask brush,
edit chain with undo and compare slider.

**M4 – Describe + Styles + Presets**: llama-server captioner, sentence/tags modes, Use as prompt;
style library (built-ins + user styles, final-prompt preview); presets save/load/built-ins.

**M5 – Polish**: live preview (TAESD), upscale, LoRAs + trigger words, offline mode enforcement
test, packaging, README with screenshots.

---

## 12. Nice-to-haves (post v1, pick later)

- **Prompt helper**: "Improve my prompt" using the local captioner/LLM, fully offline.
- **Face fix** pass (ADetailer-style) for small faces in full-body shots.
- **Outpaint / extend canvas** (change aspect ratio of an existing image).
- **Background remover** for product shots (brand work).
- **Batch edit**: apply the same instruction to several images (e.g. a product line).
- **Seed grid**: 4 seeds side by side, pick one to continue.
- **Keyboard-first flow**: Ctrl+Enter generate, E edit, S save, D describe.

---

## 13. Decisions log

- **Styles**: yes, as a separate library combined with the prompt at request time (§7). The
  main prompt is still never stored.
- **Models**: never bundled; one-click download; Pinhole auto-picks the best model per role
  that fits the user's GPU (§6.1).
- **VRAM**: every model shows how much VRAM it needs (§6.2).
- **Paid (early access) models**: hidden by default.
- **Content filter**: Safe only (default) · Include 18+ · 18+ only.

### Implementation decisions (v1 build-out)
- **Live TAESD preview is deferred**: `sd-server` has no preview API and `--taesd` replaces the
  final VAE decode, so the Create tab shows step progress (parsed from the engine's progress bar)
  instead. `models.yaml → engine_features.taesd_preview` turns it on when the engine supports it.
- **VRAM fitting uses sd.cpp auto-fit** (default in the pinned engine) instead of
  `--offload-to-cpu`, which disables auto-fit and forces every weight into RAM. Low/mid tiers keep
  `--vae-tiling`.
- **Cancel while generating restarts `sd-server`** (the server answers 409 to cancelling a running
  job); the next Generate reloads the model.
- **Linux engine = Ubuntu 24.04+**: upstream only publishes Ubuntu 24.04 builds (glibc 2.38). Building
  our own 22.04 engine is a possible follow-up.
- **"Stay close to original"** maps strict → the low end of the edit family's CFG/guidance range
  (more guidance moves the edit further from the source).
- **WebView is private**: the main window runs incognito (no cookies/cache/storage on disk); in
  portable mode its profile folder lives in `Data/webview`.
- **Observed peak VRAM** is not recorded yet (§6.2 step 3) — follow-up.
- Code layout: a Cargo workspace of small crates under `src-tauri/crates/` (see
  `docs/ARCHITECTURE.md`).
- **Local engine API exposure (security review).** Upstream `sd-server` has no authentication,
  answers any CORS `Origin` (with credentials) and keeps every finished job — base64 images
  included — at `GET /sdcpp/v1/jobs/{id}` for 600 s. Another program on this computer, or a web
  page that finds the random port and a job id, could read recent images while the engine runs.
  Interim mitigations: loopback-only random port; the engine is stopped on **Clear session** and
  5 min after the last generate/upscale once it has run a job (next Generate reloads the model);
  after start-up Pinhole checks that the server on the port is its own child reporting the model
  it launched (port squatting). `llama-server` (Describe) gets a random per-launch API key via
  `LLAMA_API_KEY` and only `/health` stays public. **Real fix (follow-up):** ship a patched
  `sd-server` build that rejects any request carrying an `Origin` header and requires a
  per-launch bearer token (passed via the environment), then drop the idle-stop workaround.

## 14. Open questions

1. Which specific Realistic and Anime models head the `recommended` lists? Pick by testing
   the top-rated candidates on an 8 GB, 12 GB and 16 GB card; record measured VRAM.
