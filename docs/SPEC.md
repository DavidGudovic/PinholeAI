# Pinhole — Product & Technical Spec (v1)

Pinhole is a local, offline AI image generator for people who don't want to learn ComfyUI.
You pick a model, type a prompt, press Generate. Everything else is decided for you, and
every decision can be overridden.

---

## 1. Principles (in priority order)

1. **Local by design.** Prompts are kept in memory, not written to disk. No telemetry, no analytics,
   no crash reporting, no automatic update checks. The only network traffic is traffic the user
   starts (browsing CivitAI, downloading a model or engine, pressing "Check for updates").
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
  Supports SD1.5, SDXL (incl. Pony/Illustrious finetunes), SD 3 / 3.5, Flux.1 (incl. Krea
  [dev]), Flux.2 (dev, klein 4B/9B), Chroma, Z-Image, Qwen-Image (incl. 2.1), Krea 2, Anima,
  HiDream-O1, ERNIE-Image, Mage-Flow, and instruction-edit models (Flux.1 Kontext, Qwen Image
  Edit 2509/2511). Video-only models (e.g. MiniMax H3) are out of scope: `img_gen` refuses them. Backends: CUDA,
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
- Linux has no upstream CUDA build of `sd-server`; Pinhole's fork builds one for RTX 30xx and newer
  (compute capability 8.6+). Older NVIDIA cards, or no NVIDIA driver, use the Vulkan build.
- Windows: the upstream builds need the MSVC runtime (VC++ 2015–2022 x64); Pinhole bundles the
  redistributable DLLs and copies them next to an engine when the system lacks them.

---

## 3. Data folder

All app files live in one folder called `Data/`:

- **Portable mode**: if a writable `Data/` folder exists next to the executable, use it
  (Windows zip release).
- **Installed mode**: `%LOCALAPPDATA%\Pinhole\Data` (Windows), `~/.local/share/pinhole/Data` (Linux).
- Settings has an **"Open Data folder"** button.
- **Saved pictures** (what Save writes) go to `Pinhole` inside the user's Pictures folder
  (the XDG pictures folder on Linux, the Pictures known folder on Windows), created on the first
  Save. A portable copy, or a computer with no Pictures folder, uses `Data/outputs/`.
  Settings → Saved pictures can point Save at another folder; that path is stored in
  `settings.yaml` with the other settings. Changing it moves nothing that was already saved.

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
  outputs/            Save folder of a portable copy; only written when the user clicks Save
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

**Models folder (optional).** Settings → **Models folder** lets the user keep `models/` somewhere
else, e.g. a partition shared by Windows and Linux on a dual-boot PC, so both installs use one set
of files. The picked folder holds the model sub-folders directly (`checkpoints/`, `vae/`, …) plus
its own index `pinhole-models.json` (same format as `installed.json`; paths stay
`models/<sub>/<file>`, so they read the same wherever the folder is mounted). Engines, settings,
presets and styles stay in each install's `Data/`. Changing the folder moves every installed model
(rename on the same drive; otherwise copy, check SHA-256, then remove the original; any failure
puts everything back), merges with models already there (same SHA-256 → not copied twice), then
restarts Pinhole. It is refused while downloads or generation run. A picked folder that is missing
(drive not mounted) is never created; installs and downloads say so instead. A read-only folder
(e.g. NTFS mounted read-only after Windows Fast Startup) and FAT32 (no files over 4 GB) get plain
error messages. The folder is not locked: two Pinhole installs running **at the same time** on one
folder can overwrite each other's index (dual boot never does that).

**Models from another app (optional).** Models → Installed → **Use models from another app** lets
the user pick a ComfyUI, A1111, Forge or Stability Matrix models folder (or any folder of model
files). Pinhole uses what it can run **in place**: nothing in that folder is ever written, moved,
copied or deleted, and no index is written into it. There is no mapping screen; each file is
recognised automatically:
- Every `.safetensors` / `.gguf` below the folder is found (links followed; tool, output and
  unusable-kind folders such as `custom_nodes`, `controlnet`, `upscale_models`, `embeddings` are
  skipped). Only headers are read (§6 step 3).
- **Main models:** the header's family candidates, narrowed by the notes other apps keep next to
  their files (A1111/Forge Civitai Helper `.civitai.info`, ComfyUI LoRA Manager `.metadata.json`,
  Stability Matrix `.cm-info.json`, A1111 `.json`: CivitAI `baseModel`, SHA-256, name, trigger
  words), then by words in the file or folder name (`name_hints` in models.yaml: `pony`,
  `illustrious`, `kontext`, `turbo`…), else the base family.
- **Style add-ons (LoRAs):** family from a note, else the kohya training metadata inside the file
  (`ss_base_model_version` / `modelspec.architecture`, `lora_metadata` in models.yaml), else a name
  hint; unknown = usable with any model, as with "Add a file". They show under Style add-ons.
- **Parts (VAE, text encoders):** used only when the file is byte-for-byte a part Pinhole knows:
  same kind and size, then the same SHA-256 (only for such size matches).
- **CivitAI lookup (RELEASE-SPEC §5):** every main model and add-on is read once to compute its
  SHA-256 (a note's own hash isn't trusted for this) and looked up on CivitAI by that hash when the
  user adds the folder or presses **Check again**. One CivitAI marks as a real person or someone
  under 18 is not used; one CivitAI doesn't know, or not looked up yet (found by the look at start
  or when Installed opens, Offline mode, a failed lookup), counts as "safe images only" until a
  lookup clears it. Files not looked up yet are looked up once when Offline mode is turned off.
  Files Pinhole offers itself (known SHA-256) need no lookup.
- Not used: anything else, and models a note or the lookup says CivitAI marks as a real person or
  someone under 18.
What was found is kept per install in `Data/catalog/linked-folders.json` (never in the shared
Models folder's index: paths differ per OS), with each file's size and time so unchanged files are
not read (or hashed) again. The folder is looked through again at start, when Installed opens and on
**Check again**. The files show in Installed with an "In <folder>" badge and no Delete; removing
the folder from the list only forgets them. Fit badges and recommendations count them like
installed files; a model that lacks parts gets **Get missing parts** (from the registry). sd-server
loads add-ons only from one folder, so a linked add-on used in a picture is hard-linked (else
symlinked, else copied) into `models/loras/.pinhole-linked/`, which is emptied at every start.
Moving Pinhole's Models folder leaves linked files where they are, and it can't be moved into (or
around) a linked folder. A file Pinhole already has (same SHA-256, from a note or a part match) is
listed once. A folder whose drive isn't connected keeps its entries and shows "Not connected".

---

## 4. Privacy rules (hard requirements — tests must enforce them)

1. No prompt text is ever written to disk, logs, crash dumps, presets, file names, or PNG metadata.
2. Generated images live in RAM until the user clicks **Save**. Closing the app discards them.
   Closing the window or pressing Reset while some pictures were never saved asks first
   (**Save all…** to a folder / close without saving / go back). Which pictures count as saved is
   kept in memory only (paths, no prompt).
3. **Reset** button: drops all in-memory images and prompt fields immediately (after the question
   above when pictures are unsaved).
4. No outbound network except: CivitAI API calls, model/engine downloads, and Hugging Face
   component downloads — all started by the user (turning Offline mode off counts: it retries,
   once, the CivitAI lookups of added or linked files that couldn't run, §6).
5. **Offline mode** toggle (Settings): blocks all network calls at the Rust HTTP client
   layer. The catalog shows "Offline" and only installed models.
   A small **Offline / Online** badge in the top bar shows the state at all times; clicking it opens
   **What goes online**, a plain list of every call the app can make (what you did, which site, what is
   sent) and what never leaves the computer, with a button to switch Offline mode. Settings → Privacy links
   to the same page. The list lives in `src/components/WhatGoesOnline.tsx`: **add to it in the same PR
   whenever a new network call is added.**
6. No telemetry SDKs, no automatic update checks, no remote fonts/CDNs in the UI (bundle everything).
   Updates are checked only when the user presses **Check for updates** (Settings → Updates): one
   request to the GitHub releases API through the same Rust client (Offline mode, allow-list).
7. The CivitAI API key (optional) is stored in the OS keychain (`keyring` crate), never in `Data/`.
8. Saved file names: `pinhole_YYYYMMDD_HHMMSS_<seed>.png`. Never derived from the prompt.
9. Saved-image metadata: only the **AI-generated marker** by default (RELEASE-SPEC §2): XMP
   `DigitalSourceType` only ("made with AI", no app name), always written to pictures Pinhole made (no
   setting turns it off; an untouched imported picture gets none). Optional setting "Include
   generation settings (no prompt)" also writes model name, seed, steps, dials into a PNG text chunk.
   **Reuse settings**: dropping such a picture on Create's results area reads that chunk back (in Rust,
   only the known fields, range-checked; the picture is not kept) and fills model (by id, else by name),
   seed ("Keep this look" on), steps, Stick to prompt, sampler, scheduler and size through the same mapping
   as Paste from CivitAI. The prompt, negative prompt and add-ons are left as they are. A picture without
   the chunk opens the same chooser as a pasted picture (§5); other programs' metadata is never read.
10. CI check: grep-based test fails the build if any code path writes a `prompt` field to a
    file or log (`scripts/privacy-lint.mjs`).
11. **The one exception is Styles** (§7): text the user explicitly saves as a named Style is
    stored in `Data/styles/`. That is a deliberate user action, clearly labelled
    ("Saved styles are stored on this computer"). The main prompt is never stored, and
    nothing is ever saved as a style automatically.

Known limit (not for the README, which keeps no limitations section): the OS may page RAM to
swap/pagefile; Pinhole cannot control that.

---

## 5. Screens

Four tabs: **Create**, **Edit**, **Describe**, **Models**. Plus a Settings sheet.

A picture pasted with Ctrl/Cmd+V on any tab opens a small chooser: **Create
reference picture** (only while Create shows the reference slot), **Edit** or **Describe**. The
choice for the current tab is first. The picture is then imported exactly like a dropped or chosen
file, and that tab opens. Text pasted into a text field stays a text paste. On Linux the WebView's
paste event carries no picture (no data, or only the page markup after a browser's Copy image), so
the picture is read from the system clipboard through Rust
(`clipboard_image`, PNG); in a text field only when the clipboard holds no text.

A picture dropped on the window opens the same chooser, wherever no drop area takes it (the Edit and
Describe tabs and the reference slot take it directly; a picture without saved settings dropped on
Create's results area goes to the chooser). A picture dragged out of a web browser works when the browser
hands over the file (Windows), or when the page holds the picture in a data: address (read in the app);
when only its web address comes through (usually Linux), a short note says to use Copy image and paste. A file that isn't a picture gets a short note too. Nothing is downloaded from a dropped
link.

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
| Keep this look | toggle ("Keeps this picture’s layout") | locks the seed of the selected result |

**Fine-tune** drawer (collapsed by default): negative prompt (only for families that use it),
sampler, scheduler, steps, CFG, seed, flow shift, clip skip, hires fix, LoRAs with weights,
VAE tiling, Repeats without seams. Each field shows the registry default and a "reset" button. The LoRA list's **Add**
menu lists installed style add-ons that work with the current model and ends with **Find add-ons
for <model>…**, which opens Models → Browse on style add-ons for that model.

**Style add-ons in use**: while at least one LoRA is added, a row of chips under the prompt shows
each one with its strength and the trigger words it adds ("Film look 0.8 + film photo ×").
Clicking a chip shows a Strength slider (0–1.5) and the add-on's trigger words as ticks: ticked
words are added to the end of the prompt at request time, in memory, and not twice if the user
typed them (whole-word match). By default a short list (up to 3) is all ticked; a longer list is
usually alternatives (one per character or outfit), so only the first is. With "Add trigger words
automatically" off in Settings none are ticked until the user ticks one. **Edit** (or **Add** when
none are saved, e.g. an add-on added from disk) lets the user type the add-on's trigger words,
which replace CivitAI's list in `installed.json` (add-on metadata, never prompt text). × removes it. An add-on made for another architecture stays in the list
greyed out with "Made for SDXL models, so it isn't used with this one" and is left out of the
request. Nothing is shown when no add-on is in use.

**Repeats without seams** (Fine-tune, off by default, stored in presets like the other Fine-tune
values): the picture's left and right edges and its top and bottom edges join up, so it can be
repeated as a pattern or texture. Shown only for families with `seamless: true` in the registry
(SD 1.5 and SDXL, both checked with a 2×2 tile on CPU). It applies to new pictures in Create only;
edits, restyles, Fix details and Extend never use it. sd-server reads it at launch (`--circular`),
not per request, so the first picture after switching it on or off restarts the engine with the
usual loading progress. A result made this way gets a **Show tiled** button that shows it 2×2 in
the preview, and its settings line says "repeats without seams".

**Named sizes** (Fine-tune, above Width and Height): **My screen** (the monitor's shape), **Phone**
(9:16), **Instagram** (4:5) and **Thumbnail** (16:9). One click sets Width and Height to that shape at
about the model's usual picture area (its Square size), in multiples of 64. "My screen" notes that
Upscale reaches the monitor's own resolution.

**Starter ideas**: while the prompt box is empty, a few plain example chips ("Cabin in the snow",
"Watercolor fox"…) fill it on click, with a hint to try Improve. Examples stay fictional and safe for work.

**Choices in braces**: `a {red|blue|green} car` makes one picture per combination in one Generate
(several groups multiply; the first group changes slowest). Only braces holding a `|` count; other
braces, and everything in a saved Style, stay plain text. Repeated options count once, and How many
is not used. The pictures share one seed (the locked one, else a new random one), so only the
choices differ. At most 16 pictures: the button reads "Generate 3" ("Add 3 to queue"), and when
there are more combinations a line under it says "Makes the first 16 of 30 combinations". The
prompts are expanded in memory only, run one after another as one queued job, and each one is a
normal Create request (word check, image check and everything else as for a typed prompt). Prompt
history keeps the prompt as typed.

**Tip line**: one quiet "Tip" under a picture in Create about a feature that is easy to miss (Keep this
look, Variations, choices in braces, Styles, Paste from CivitAI, the reference picture, the ? shortcuts list). At most one
per app session; × closes it, **Don't show tips** (or Settings → Show tips) turns it off for good.

**Reference picture** (optional, under the prompt): "make something in the style of this picture"
or "the same character somewhere else". Shown only for models whose architecture takes reference
images (`modes: [..., edit]` in `models.yaml`: FLUX.2 klein and dev, Qwen-Image 2.1 with its vision
file); **Add a reference picture** opens a file, and a picture can also be dropped, pasted
(Ctrl/Cmd+V, see §5) or picked from this session's pictures, Create and Edit results still in memory (small thumbnails next to the button). The picture goes to `sd-server` as
`ref_images[0]` of a txt2img request, and the result has no "parent" (it isn't an edit). While a
picture is set, the Shape dial has a full-width **Same as reference** chip under the four shapes: the
reference picture's shape at the model's Square area (the same rule as Named sizes), so the memory
need is unchanged; Width and Height typed in Fine-tune still win. Adding a picture picks it when the
shape is still Square and no size was typed, and the slot says "Shape: same as this picture."; picking
another shape (or a preset or pasted settings with a shape) takes over, and removing the picture goes
back to the Shape dial. It lives in session memory like every image, is kept by
queued jobs and by Variations of a batch made with it, is never saved in a preset, and Reset clears
it. Switching to a model that can't use it keeps the picture with "<model> can't use a reference
picture" and a **Switch to <model>** button for an installed one that can (ready, fits, most
recently used), or **Use it in Edit** when none is installed; Generate then says the same instead
of quietly dropping it.

**Same character** (result card): new pictures of the character or subject in that image, with one
button and no new mode or setting. It uses the first of: the Create model, if it takes a reference
picture; another installed one that does and can run now (ready, not "Too big"; switched to); else
Edit → **Describe a change** with the image loaded (any image 2 cleared), which offers the one-click
edit model when none is installed. The same applies to an Imported result (made from a picture the
user added); the notice about photos of people (RELEASE-SPEC §7) shows in Create's reference slot as
in Edit. A short note says to describe the
new scene ("the same character on a beach"). The image keeps its id, so its origin
(Generated/Imported, RELEASE-SPEC §3.1) and the image checks carry through unchanged.

**Improve** (prompt box toolbar): turns a short idea into a fuller prompt with the local Describe
model (text only, `captioner.improve` in `models.yaml`). The helper fills six short lines: the
idea reworded for the image model (PROMPT), then how the subject looks, place, shot, style and
light. Pinhole writes the first label for the helper so it keeps to the form. The rewording is
used only when it keeps the idea's intent: its numbers and negations (and no new ones; each negation stays with the
word it negates), the words in `improve.exact_words` ("more", "her", "this"…), every view, medium,
light or "same"/"keep" phrase the idea names (`improve.given`), at least four in five of its other
words (`improve.common_words` aside; singular and plural count as one), at most 40 words, and no
added person (`improve.people`), mood or quality word or add-on trigger word. Otherwise the user's
words are used as typed. The other lines are added after it: a line the idea already covers is
left out, and so are mood and quality words (`improve.drop`, quality words come from the family's
`auto_prompt_prefix`), "no …" phrases, phrases naming a person when the idea names none, lines with two or more phrases
of the form's example, long sentences,
the idea's own words and repeats. Sentences for natural-language families, tags (at most 12 added)
for families whose `style_template` is `tags` (SD 1.5, SDXL), Danbooru tags for families with
`improve_style: booru` (Pony, Illustrious, Anima). The result replaces the box text
and **Undo** puts back what was typed (shown while the box still holds the improved text; the
answer is dropped if the prompt was edited meanwhile). The instruction says to
stay safe for work while Safe mode is On (while Off: adult themes between adults are kept and
described clearly, nothing of that kind is added that the user didn't ask for, and anyone under 18 stays
non-sexual and clothed; the word check runs on the idea, the whole answer and the result either way), and not to
write the trigger words of add-ons in use (phrases containing them are left out if it does anyway, since they are
added at request time). An answer that is a refusal ("I'm sorry, but I can't…") or adds nothing
usable leaves the prompt unchanged with a short note. The prompt goes
only to the loopback llama-server, never logged or stored. Without the Describe model it offers
the one-time download, then improves. In Edit (Describe a change, next to "What should
change?") Improve works the same way on the change instruction with its own three-line form
(`improve.edit`): the instruction reworded as one command (CHANGE), how the change should look,
and two or three parts to keep (the composition, the camera angle, the background, the lighting,
a person's face and pose), joined as "<change>. <Details>. Keep … unchanged." The rewording
follows the rules above, and also may not use an edit verb of another kind than the instruction's
(`improve.edit.actions`: "add a hat" never becomes "replace the background"; an instruction with
no verb may get one, but not one in `improve.edit.never_added` such as "remove" or "replace"); an instruction that refers to another picture ("from image 2",
`improve.edit.keep_wording`) keeps its wording. Keep phrases naming something the instruction
mentions are left out (`improve.common_words` don't count, nor the owner in "the man's face"), at
most three are kept, details that only say "same …" are left out, and when the instruction keeps
something ("same bottle, different table", "keep the bottle") only details naming another thing it mentions
stay. The keep line is left out
when the instruction already says what to keep, and the details line when it removes something
(the helper can't see what is behind it) or takes something from image 2. Always sentences (every
edit model reads natural language); the Safe mode rule, add-on trigger words and word check apply
as in Create.

Result card actions: **Save** · **Edit this** · **Describe** · **More like this ▾**
(**Close to this one**: How many new pictures restyled from this one at strength 0.55 with the
same prompt, model, size and settings and new seeds, so the layout holds and the details change;
**Variations**: same prompt, new seeds; **Same character**; **On another model…**) ·
**Upscale ▾** (**Finish at Best quality** on top for pictures made at Fast or Balanced: the same
request at Best with the picture's own seed and Best's steps, one image; then **Upscale 2×/4×**)
· **Copy to clipboard**. **Save all (n)** appears next to Save when
more than one picture is unsaved. **Save as one sheet (n)** (in Save's menu, desktop app) saves
the pictures that came in the same run as the shown one (its batch, one Generate with choices in
braces, or one "Also apply to…" edit;
upscales only with upscales; 2–8, oldest first) as one grid picture through the save dialog: equal
cells the size of the largest picture, each fitted and centred, small white gaps, no text, at most
8192 px a side. It goes through the export function (`session::export_png` with several pictures):
each picture is watermarked, the sheet too when it carries a marker, and the XMP marker is
their shared source type or "composite"; no settings chunk. The pictures themselves stay unsaved.

**On another model…** (under More like this): lists installed models other than the one that made
the picture, with their Fits/Tight badge; models that are too big or not fully set up are left
out, and when the picture used a reference picture only models that take one are listed. Picking
one runs the same prompt, style, shape, Quality, reference picture and seed there as a queued job
(the model switch restarts sd-server with progress, as usual). Settings that belong to one family
(sampler, steps, CFG, Stick to prompt) use the new model's defaults, a set size (a named size,
Same as reference) keeps its shape at the new model's usual size, Hires fix on or off carries
over, and add-ons that don't work with it are left out. The new picture opens side by side with the first one, each labelled
with its model's name; the side-by-side button switches between the two views.

**Upscale** runs a 4× ESRGAN model in sd-server (2× = 4× then halved). There are three, all
pinned in `models.yaml` and each downloaded the first time it is used: `realesrgan_x4` (photos,
smooth), `nomos_webphoto_x4` (photos, keeps skin texture) and `realesrgan_x4_anime` (the anime 6B
model, drawings). With the `upscaler` setting on `auto` (the default) Upscale measures the picture
with the image check's tagger and uses `realesrgan_x4` when its `realistic` or `photorealistic`
tag reaches the photo-style threshold (`pinhole-check` `rules::PHOTO_STYLE`), the drawing upscaler
otherwise. Fine-tune in Create and Edit has an **Upscaler** row, a list with one plain line per
option: "Auto: picks by picture style", "Photo: smooth (good for hair, can look waxy)", "Photo:
skin texture (real skin, can make beards crunchy)", "Drawing: clean lines and flat colour"
(settings `auto` | `photo` | `photo_texture` | `drawing`). The upscaled picture's summary names
the upscaler used ("drawing upscaler").

**Prompt recall:** Up (at the start of the box) and Down step through the prompts sent earlier in
this session, like a shell; Down past the newest restores what was typed. Kept in memory only (last
50), gone on Reset or close.

**Done alert:** when a picture (or the last job in the queue) finishes while the window is in the
background, the taskbar icon flashes. Settings has a toggle for a soft chime (off by default,
made in code, no sound file).

**Paste from CivitAI**: CivitAI's "Copy generation data" button yields A1111-style text (prompt,
`Negative prompt:`, `Steps: …, Sampler: …, CFG scale: …, Seed: …, Size: …, Clip skip: …, Civitai
resources: [...]`). A **Paste from CivitAI** button next to the prompt (and pasting such text into
the prompt box) parses it in memory, fills prompt, negative, steps, CFG/guidance, sampler +
scheduler (mapped to sd.cpp names), seed, size, clip skip and hires, selects the installed
checkpoint/LoRAs (matched by CivitAI version id or hash) or offers one-click installs, and shows
what was applied and what was skipped. The pasted text is never stored or logged.

Live preview: if a TAESD file is registered for the family, show a low-res preview while
generating. Progress bar + **Cancel** (`POST /sdcpp/v1/jobs/{id}/cancel`).

**Queue:** pressing Generate (or Apply edit / Restyle in Edit, Close to this one, Variations, Finish at Best quality, or
Upscale in Create or Edit) while a job runs adds it to a queue instead; the button reads "Add to queue". Each queued job keeps the settings from the
moment it was pressed and runs, in order, when the one before it ends (switching models in between
as needed). A small button next to Generate shows how many are waiting and opens the list, where
each can be removed. Cancel stops only the running job (during Also apply to…, the rest of that
run too); Reset empties the queue. Queued jobs live in
memory only. While an edit runs or waits, the edit history stays put; a queued edit or upscale of an
earlier image is added at the end of the history.

### 5.2 Edit (img2img + instruction editing)

Entry points: **Edit this** on any result, drag-and-drop, paste from clipboard, file picker.
The image comes in as an in-memory buffer (never copied into `Data/`).

**From this session:** wherever a picture is asked for (the empty Edit and Describe tabs, **Another
image** in Edit and Describe, **Add another image**, the reference picture), the pictures made this
session that are still in memory (Create results and Edit results, newest first) are offered next to
choosing a file: thumbnails under an empty drop area, or a small menu (**Choose a file…** plus the
thumbnails) on the button. Without any, the button opens the file chooser as before. The picture
shown, or image 1 for image 2, is left out. A picked picture is used as it is in the session, like
**Edit this**; nothing is read from or written to disk.

Four modes; Describe a change or Restyle is picked automatically:

1. **Instruction edit** (default when an edit model is installed): the user types what to change:
   "make it evening with warm street lights" or "replace the mug with a water bottle". Uses the
   recommended edit model, Qwen-Image 2.1 (it also creates, so one download serves both tabs),
   with the image passed as `ref_images[0]`. Installed dedicated edit models (Qwen Image Edit
   2511, Flux.1 Kontext) and other generators that can edit from a reference image
   (`modes: [..., edit]` in `models.yaml`: FLUX.2 klein and dev) are offered too; the automatic
   pick ranks Qwen-Image 2.1, then the dedicated edit models, then the other generators.
   Qwen-Image 2.1 needs Qwen3-VL-8B's vision weights for edits, installed as one of its parts.
   - Dial: **Stay close to original** (maps to the family's guidance setting; hidden when the
     family has a fixed CFG and no guidance, e.g. distilled FLUX.2 klein).
   - Optional **"Only change here"** brush: paint a mask → `mask_image`.
   - Optional **"Add another image"**: a second picture (image 2) for edits like "put the
     bottle from image 2 on the shelf". Sent as `ref_images[1]`; only models with
     `multi_ref: true` (Qwen-Image 2.1, Qwen Image Edit, FLUX.2) are offered then, and one that Fits wins the
     automatic pick. The brush is hidden while image 2 is there. Image 2 stays in memory like
     the edit chain until it is removed or Reset.
   - Optional **"Also apply to…"** (also in Restyle): up to 16 more pictures, picked from this
     session's results and edited steps or from files. Apply then queues the shown picture's edit
     and, behind it, one edit per picked picture with the same text, model, settings and image 2,
     each at its own size and without the brush (hidden while pictures are picked). The shown
     picture's result joins the edit chain as usual; every result of the run also goes to
     Create's results as one group (for Save as one sheet). The list is cleared once queued;
     Try again redoes only the shown picture.
2. **Restyle** (classic img2img with the current Create model): image as `init_image`.
   - Dial: **How much to change** (Subtle · Medium · Strong → `strength` 0.35/0.55/0.75).
3. **Fix details** (same models as Restyle): the user paints over a small spot such as a face or
   hand; an optional "What is it?" text is the prompt. Rust takes a padded box around the mask
   (a quarter of its longer side, at least 32 px; at least 128 px and no more than 2:1 where the
   image allows),
   scales it to about the Quality dial's native area, inpaints only that box (img2img +
   `mask_image`, one image, no hires fix), scales the result back down and pastes it into the
   source with a feathered edge. The result keeps the source's size; with a painted spot no face detector is used.
   All of it happens in memory. **How much to change** maps to `strength` 0.3/0.45/0.6.
   With nothing painted, the button reads **Add detail**: the image check's face finder (YuNet,
   already downloaded for the check; no new file) finds the clear faces at least 40 px across
   and less than half the Quality dial's native side (a larger face would be drawn smaller than
   it is), and each face (largest first, at most 6) is fixed the same way, its box grown by 15% on every
   side as the mask, one engine pass per face, each on the previous pass's result. Without
   "What is it?" text the prompt is "photo of a face, natural skin texture, sharp focus" for a
   photo-style picture (the tagger's photo-style reading, as for the Upscale Auto pick) with no
   Style picked, else "a detailed face". With more than one face the progress note reads
   "Face N of M." and one progress bar runs across all faces. No such face → "No face found that
   needs more detail. Paint over the part to fix instead." The picture keeps its size; the finished
   picture and each redrawn face are checked like any Fix details result.
4. **Extend** (same models as Restyle): make the picture wider or taller and let the model
   draw the new edges. The user picks a **New shape** (the Create shape chips, or **Around** =
   same shape with 15% more on every side; a chip the picture already has is greyed out) and,
   for a new shape, where the space goes (**Left · Both sides · Right** or **Top · Both sides ·
   Bottom**). An optional "What's in the picture?" text is the prompt. The stage shows the new
   canvas as a dashed frame around the picture. Rust puts the source on the bigger canvas
   (never cropping; at most 8192 px a side), fills the new space with a blurred stretch of the
   nearest edge as a colour hint, masks the new space plus a seam of about 1/24 of the work
   size over the old edge, scales both to about the Quality dial's native area, inpaints
   (img2img + `mask_image`, `strength` 1, one image, no hires fix), scales the result to the
   canvas size and pastes the source's own pixels back over it, fading into the redraw across
   the seam. The source keeps its detail; the new space is as sharp as the model's size allows.
   No brush, no Output size. Works with Try again (same settings from the step before) and
   the queue. All in memory.

All modes take style add-ons (LoRAs) like Create: added in Edit's Fine-tune (with Quality,
Output size and Seed), shown as chips under the text, trigger words picked on the chip. Only
add-ons made for the edit's model (same architecture) are used. Edit keeps its own add-on list
(Reset keeps it, like Create). Pasting CivitAI generation data stays in Create: it describes a
text-to-image run (size, seed, sampler), not an edit of your own picture.

If no edit model is installed, the Edit tab shows one card: "Get the best edit model for your
GPU" — one-click download of the top edit model that fits (§6.1), showing its download size
and VRAM need.
When Edit opens in Restyle only because no edit model is installed, one slim line at the top
offers it: "To change one thing and keep the rest, get the edit model (size)" with a Get button.
Restyle stays usable; the line goes once an edit model is installed and can be closed for the
session. Picking Restyle yourself doesn't show it.

**Edit chain**: each edit result can be edited again. Keep an in-memory undo stack
(original → edit 1 → edit 2…) with a before/after comparison slider. Click any step in the
history strip to work from it; **Delete this edit** removes the shown edit (never the original) and
frees its image from memory; remaining edits are renumbered.

Actions on the shown image: **Save** (with **Save as…** in the desktop app) · **Copy** ·
**Describe** · **Upscale 2×/4×** (the upscaled image becomes the next step) · **Try again**
(redoes the shown edit from the step before it with a new seed and the current settings, the same
brush area included, and replaces it and any later steps; not for the original or an upscale step). Fine-tune shows the
read-only "Final prompt sent to the model" like Create, since the style and trigger words are added
at request time.

**Helper models** (Describe and Improve): the language models behind both come from Pinhole's own
list (`captioner.helpers` in `models.yaml`: Qwen2.5-VL 3B, the default, and 7B, which is also
Qwen Image Edit's encoder and is not downloaded twice). Settings has **Describe model** and
**Improve model** (`describeModel` / `improveModel`: `auto` or a helper id). Describe also has a
model picker at the top (the same card as Create's, listing the installed helpers and Automatic) that
reads and saves `describeModel`; Improve has no picker of its own (Settings only). **Automatic** uses the 7B when it is installed,
else the 3B (Safe mode Off: the Safe-mode-Off helper first, below). Only installed helpers can be picked; a removed one reads as Automatic. **Models →
Helpers** lists them with size and Fits / Tight / Too big and Get / Remove (Remove only for files
Pinhole downloaded as a helper). A helper with `needs_safe_off: true` is only listed while Safe
mode is Off, and while Off, Automatic uses it first once installed: **Qwen2.5-VL 7B abliterated**
(`qwen25_vl_7b_safe_off`), Qwen2.5-VL 7B Instruct with its refusals removed from the text part
([huihui-ai/Qwen2.5-VL-7B-Instruct-abliterated](https://huggingface.co/huihui-ai/Qwen2.5-VL-7B-Instruct-abliterated),
Apache-2.0; GGUF Q4_K_M by mradermacher, 4.7 GB), which shares the 7B's vision file (Remove keeps a
vision file another installed helper still uses). Its note in the app says what it does and that the
word check and the image check still apply. Installed models still lists it while Safe mode is On, so
it can be removed. The word check runs on the output of every helper model, and the image check on
every picture made from it.

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
| For (style add-ons only) | Any model · For <installed model> (defaults to the model picked in Create) | `baseModels=` the CivitAI base models of every registry family with the same architecture as that model (`families::lora_base_models`: an SDXL model also gets Pony and Illustrious LoRAs); each card shows the newest version made for them; a tag that narrows `baseModels` (Edit) with nothing in common answers "none" without asking CivitAI. Not a filter "Clear filters" resets |
| Look | Realistic · Anime · Illustration · 3D · Painting · Pixel art · Line art · Cinematic · Vintage · Brand & product | tag sets from `config/catalog-filters.yaml`. Look, Tags and the switches row each stay on one line and scroll sideways when the window is too narrow |
| Tags | multi-select: Edit model · Reference picture · Portraits · Characters · Landscapes · Architecture · Animals · Fantasy · Sci-fi · Vehicles · Robots · Food · Fashion · Objects · Backgrounds · Textures · Horror · NSFW | client-side, `catalog-filters.yaml → tags`; a model must match every picked tag (its tags, whole words in its name, or its base model). Reference picture = models whose family has both `txt2img` and `edit` in `models.yaml` `modes` (they can follow Create's reference picture); the base models come from `models.yaml` and are also sent as `baseModels`; those cards and installed models show a "Reference picture" badge. NSFW = exactly the models Safe mode hides; not shown while Safe mode is on. No one-click preset for it |
| Safe mode | On (default) · Off | always `nsfw=true` (the only way to get every sample image with its rating); On keeps models that pass `safe_filter` (see below) · Off keeps everything |
| Price | Free (default) · Include early access (paid) · Early access only | free = drop models whose latest version is in early access; paid items are **hidden by default** |
| Sort | Most liked · Most downloaded (default) · Newest | `sort=Most Liked / Most Downloaded / Newest` |
| Time | This week · This month · This year · All time (default) | `period` |
| Commercial use | Any · OK for client work | `allowCommercialUse` includes `Image` |
| Compatibility | Works with Pinhole (default on) | `baseModels=` every family in the registry |
| Size | Runs on my card (default off; models only) | client-side: hides cards whose best file is **Too big** (§6.2); the line above the grid says how many it hid |
| Style | Hide anime (switch, default off, remembered in Settings; models and add-ons) | client-side (CivitAI can only include one tag, never exclude; Browse keeps fetching until the page is full): hides models tagged or named anime / manga / cartoon / chibi / waifu, and whose newest version is on an anime-native base (Illustrious, NoobAI). Pony is not hidden by base, only by tags. Rules in `catalog-filters.yaml → hide_anime` |
| Search | free text | `query` |

Always shown: search, Kind, Sort, Time, Look, Safe mode, Works with Pinhole, Runs on my card and
Reference picture (a switch for that tag). Tags, Price, Commercial use and Hide anime sit under
**More filters**, which starts open whenever one of them is set and shows how many are set while
closed.

- Paging with `cursor` (page×limit > 1000 returns 429). Each request asks for `limit=50` models
  (`api_limit`); array filters are repeated keys (`baseModels=A&baseModels=B`); a text search is paged by `page=N` (CivitAI sends no cursor for it), other browsing by `cursor`.
- Turning Safe mode off requires a one-time "I'm 18 or older" confirmation per session (stored
  in RAM only). There is no "adult only" mode: the NSFW tag is the only way to narrow to those
  models, and it needs Safe mode off.
- Safe mode is **On** by default. While it is on, also blur any preview image flagged NSFW.
  Turning it on in Settings turns it on in Browse too, also when Browse was left with it off.
- Safe mode, Look, Tags and Price are partly client-side filters: keep fetching pages until the grid page
  (24 cards) is full (cap at 5 extra requests per scroll, then show "Load more"). A newer query
  stops the older one's extra requests. A request that times out is asked once more; if a later
  request still fails, the cards found so far are shown with "Load more" instead of an error.
- **Safe mode** (`catalog-filters.yaml → safe_filter`, tuned on live data; the public API has
  nothing stricter than `nsfw=false`, which only hides models CivitAI flags, and rejects
  `browsingLevel`): a model is hidden when CivitAI flags it NSFW, its `nsfwLevel` bitmask has
  no PG bit, it has an adult tag (or two suggestive ones), its name has an adult word (whole
  words), or more than half of its creator's rated sample images are R or above. The
  model-level `nsfwLevel` alone is not used otherwise: mainstream models such as Juggernaut XL
  are 31 (all levels) because people post every kind of image with them. Card previews with Safe
  mode on are PG images only (like Stability Matrix); no PG image → no preview.
- Opening filters are **Most downloaded · All time** (mainstream models; "This month" is
  dominated by fresh suggestive anime merges). A line above the grid says "Showing models that
  run in Pinhole — turn off “Works with Pinhole” to see all" and how many Safe mode hid.
- Speed: CivitAI answers are requested gzip-compressed and cached in RAM (never on disk: the
  Rust side keeps 12 answers for 5 min, the UI 80 pages for 10 min) and the next page is
  fetched ahead. Card previews are CivitAI's own card rendition (`width=450,optimized=true`;
  a video preview becomes a still frame), fetched by Rust 8 at a time, on-screen cards first;
  a queued fetch is dropped when its card scrolls away; the bytes stay in a RAM-only LRU (48 MB).
- Verify the exact early-access fields on real API responses before relying on them
  (`earlyAccess` query param, version `availability` / early-access end date).

**Model card** shows: preview image, name, friendly style badge, rating (thumbs-up ratio and
download count), download size, **VRAM needed** (§6.2) with a Fits / Tight / Too big badge for
this GPU, price badge (only when paid models are shown), commercial-use badge, and the base
model in small text.

**Install** button:
1. Pick the best file: primary, `SafeTensor` or GGUF format only. **Never PickleTensor.**
   Require `pickleScanResult == Success` and `virusScanResult == Success`. When that file is
   Tight or Too big for this card (§6.2) and the version has another safe, hashed file that
   Fits (e.g. an FP8 or Q4 file), pick that one instead (else a Tight one over a Too big one).
   All-in-one families (SD 1.5, SDXL) only switch within the same format: their GGUF files hold
   the diffusion model alone, without the VAE and text encoders;
   the card says "Compact (FP8) version, so it fits your card". When a version has more than
   one installable file, the dialog shows a **Size** choice ("Full quality", "Compact (FP8)",
   "Compact (Q4)"…) with each file's size and VRAM badge and a plain explanation: compact
   versions need less graphics memory, pictures keep their size, fine detail is a little softer.
   Files that would get the same row at about the same size (fp16 and bf16 copies; FP8 and
   INT8 copies) are listed once, as the file step 1 ranks best. The user's pick is re-planned and installed as chosen.
2. Resolve the family (§6) and list the extra components needed (VAE, text encoders),
   skipping any already installed (matched by SHA-256).
3. Show the total download size and a free-disk-space check, then download everything with
   resume + SHA-256 verification.
4. If CivitAI answers 401/403, prompt for an API key (explain why; optional; stored in keychain).
5. For LoRAs, save trigger words from the version's `trainedWords` into `installed.json`,
   and offer a toggle "Add trigger words automatically" (the default for the chip's ticks, §5.1).
6. An installed style add-on's card, its details page and its Installed row have a **Use**
   button: it adds the add-on to Create at strength 0.8 (once), switches to Create and says so;
   when the current model can't use it, the message says which models it is made for.

#### Model details
Clicking a card's preview or name opens the model's details page (Back or Esc returns to the
grid where it was):
- The card's facts (rating, downloads, size, VRAM needed with the fit badge, licence, client-work
  badge, LoRA trigger words) and the same Install button.
- **Example images**: the version's preview images from CivitAI, fetched through the Rust client
  like every preview. With Safe mode on, images made for adults are left out (with a count); videos are skipped.
  Opened from Installed, Safe mode counts as on unless it is off in Settings (confirmed this session)
  and in Browse.
  Images come from `GET /api/v1/model-versions/{id}`, the only endpoint that still returns each
  image's generation data (`/models` and `/images` send `meta: null`, checked 2026-09-28).
- Clicking an image shows it larger with its prompt and main settings, plus two buttons:
  **Use these settings** (turns the image's generation data into "Copy generation data" text and
  runs it through Paste from CivitAI, so Create fills the prompt and settings, selects this model
  or offers to install it, and shows what was applied) and **Edit this image** (the full-size image
  goes into the in-memory session and opens in Edit). The generation data is held in memory only.
- **From the creator**: the creator's description of the model and of this version (CivitAI HTML).
  It is rebuilt from a short allow-list (text formatting, lists, https links); pictures, video,
  iframes and styles are dropped, so nothing is loaded from another server and Safe mode applies to it
  too. Long text collapses behind Show more; links open in the system browser (https only,
  checked in Rust). Saved in `installed.json` at install (`creatorNotes`, raw HTML, 20 KB cap each)
  so installed models show it offline. Safe mode hides it for models made for adults.
- **Open on CivitAI** opens the model's page in the system browser: `civitai.red` for NSFW models,
  `civitai.com` for everything else. The URL is built in Rust from the model id; the WebView
  never navigates.

#### Installed
Each model row has **Find style add-ons** (opens Browse on add-ons for that model); each style
add-on row has **Use** (see Install step 6).
List with friendly name, family, size, last used, **Delete** (removes orphaned components too,
after confirmation), an **Open folder** button (the Models folder), a **Helpers** list (the
Describe model and the upscaler, with size and Delete), and **Add a file I already have** (pick a .safetensors/.gguf in the file
chooser → detected, and looked up on CivitAI by its SHA-256 as in §6), and **Use models from another app** (§3). A model or add-on
that can't make intimate pictures (RELEASE-SPEC §3.2 rule 3) has a **Safe images only** badge. Dropping model files onto the window is not supported: the native drop handler
is disabled so HTML5 image drag-and-drop works on Windows (§5).

---

## 6. Auto-wiring (model → working pipeline)

Resolution order for "what is this file":
1. **Known hash**: SHA-256 is found in `config/models.yaml → known_files` → exact family + variant.
2. **CivitAI metadata**: the version's `baseModel` maps to a family via `civitai_base_models`.
   "Add a file" looks every main model and add-on up by SHA-256 (`lookup.rs`), even when step 1
   already knows the family, for CivitAI's flags (RELEASE-SPEC §5): a real person or someone under
   18 is refused; no match, Offline mode or a failed lookup leave the file "safe images only" until
   a lookup clears it (adding the same file again, or turning Offline mode off, retries once).
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
download. The 5070 Ti (16 GB) tier gets Qwen-Image 2.1 **Q8_0** (Create and Edit) with the
Q4_K_M Qwen3-VL-8B text encoder, and Z-Image Turbo **Q8_0** with the Q8_0 GGUF Qwen3-4B text
encoder as the fast card (bf16 model + bf16 encoder ran out of VRAM on a real 16 GB card);
bf16 models and encoders are picked from 20–21 GB. Text encoders with a VRAM
choice in `models.yaml` (e.g. `{ vram_gte_20: bf16, vram_gte_10: q8, else: q4 }`) follow the same rule.

**Updating model knowledge**: edit `config/models.yaml` (shipped with the app) or add entries in
`Data/config/overrides.yaml` (deep-merged, user wins). No code changes needed for a new
finetune of a known family.

### 6.1 Recommended models (one-click, best that fits)

**No model weights ship with the app.** Everything is a one-click download.

- `config/models.yaml → recommended` holds a ranked list per **role**: Realistic, Anime,
  Edit, Describe. Each candidate has a download spec and its VRAM needs.
- **Qwen-Image 2.1 heads Realistic and Edit** (7B, the newest model the engine runs; one
  download does both). For Create it has `min_quant: q6_k`: it is picked only when its Q6_K or
  better Fits (12 GB+); smaller cards get Z-Image Turbo, which is also several times faster.
  Edit takes it at any quant (Q4_K on 6–10 GB), since it is smaller than the older edit models
  (Qwen Image Edit 2511 is 20B, FLUX.1 Kontext 12B), which stay supported but are no longer
  one-click picks. The per-card table is in `models.yaml → recommended` (test `picks_by_card`).
- **Optional second card** (`OPTIONAL_ROLES` in `recommend.rs`): `realistic_fast` offers
  Z-Image Turbo ("several times faster") next to Qwen-Image 2.1. It is left out when it would
  be the same model as the Realistic pick, and when it doesn't fit.
- For each role Pinhole picks the **first (best) candidate whose `vram_gb.min` fits this GPU**,
  choosing the best quant that **Fits** (bf16 → Q8 → Q6 → Q4 → Q3); only when none Fits, the
  Tight quant with the lowest need. "Recommended for your card" must be OK to run: a Tight
  pick is a last resort, not the default. A pick smaller than the family's best version
  carries a plain note (smaller version, fine detail a little softer).
- When the installed version of a pick is Tight and a smaller registry version Fits, that
  smaller version is offered (Models → Installed, and in Edit next to a tight edit model);
  the installed file stays, both show in the model picker. The last Realistic candidate is the
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
2. **Unknown models** (CivitAI, dropped-in files): estimate = the larger of two stages,
   because the pinned sd.cpp places the diffusion model first, parks text encoders in RAM when
   they don't fit and frees their GPU copy after the prompt is read:
   - diffusion stage = diffusion weights + VAE and other non-text-encoder components +
     activation overhead for the family's default resolution (`activation_gb`) + 0.5 GB;
   - prompt stage = text encoders + 1 GB compute + 0.5 GB.
   Min (Tight) = half the diffusion weights + activations + 0.5 GB (offload). Label it
   "~X GB (estimate)". Not yet checked against measured peaks on real cards.
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
  - While Safe mode is Off, a fixed set of words (`generate::SAFE_OFF_NEGATIVE`) is added at
    the end of the negative prompt, in Create and Edit, for families that use negatives. It
    can't be turned off; the user's own negative prompt still comes first. Measured on clothed
    portraits from an Illustrious model (2026-10-03, 4 seeds, same seed with and without): faces
    read 4.3 years older on average on the image check's age estimate (26.1 to 30.4).
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
- Models folder: where models live (default `Data/models`), **Change…** / **Use the Data folder
  again** (moves the models, then restarts; see §3) + Open
- GPU override (auto / pick device / force CPU) and a VRAM tier override
- Engine backend (auto / CUDA / Vulkan / CPU) and **Read the prompt on the processor**
  (Automatic / On / Off; shown with a graphics card) — Automatic keeps the text encoder on the
  graphics card and moves it to the processor for a model after the card runs out of memory while
  reading the prompt (kept for the app session); Off never moves it automatically (family flags
  such as `--clip-on-cpu` still apply)
- Safe mode default (On / Off)
- Show paid (early access) models (off by default)
- Saved pictures: the folder Save writes to (default `Pictures/Pinhole`, §3) with **Change…** /
  **Use default** / **Open folder**, and saved-image metadata (None / Settings without prompt)
- Show tips (on by default)
- Helper models: Describe model and Improve model (Automatic by default; §5.2 "Helper models")
- Safety: **Usage guidelines** and **Licence** (both shown in the app), and **Report a problem**:
  **Report privately** (GitHub private vulnerability reporting) or **Open a public issue**, both
  opened in the browser (SAFETY.md → Reporting a problem)
- CivitAI API key (set / remove; keychain)
- Theme (system / light / dark)
- Updates: **Check for updates** (never automatic). When a newer GitHub release exists:
  **Open download page**. **Update and restart** (Windows installer, Windows portable, Linux
  AppImage, Linux .deb) installs it when the release's files are signed. See §13.

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
- License: Pinhole Licence 1.0 (`LICENSE`): the MIT License plus a Safeguards condition
  (`SAFEGUARDS.md`), so source-available, not open source. The
  engines (stable-diffusion.cpp, llama.cpp) keep their MIT licences; include their license files
  in `THIRD_PARTY_LICENSES`.
- Model licenses are the user's concern, but show the license name on the model card
  (e.g. FLUX.1-dev is non-commercial). Licence acceptance: `docs/RELEASE-SPEC.md` §6.
- Every public build follows `docs/RELEASE-SPEC.md` (safeguards, marking, release gate).

---

## 11. Milestones

M0–M6 are done (skeleton, generate, models, edit, describe + styles + presets, polish, release
readiness); v1.0.0 was released on GitHub on 2026-10-01. The plans are in git history. New work
comes in as GitHub issues; open items are in `docs/PROJECT-BRIEF.md` and `docs/RELEASE-SPEC.md` §12.

---

## 12. Nice-to-haves (post v1, pick later)

- **Face fix** pass (ADetailer-style) for small faces in full-body shots.
- Built: **Batch edit** ("Also apply to…", §5.2) and **Save as one sheet** (§5.1).
- Built: "Improve my prompt" (§5.1) and the keyboard-first flow: Ctrl/Cmd+Enter generate, E edit, S save, Ctrl/Cmd+Shift+S save as, D describe, F full screen, R try again (Edit), ? shows the list (also in Settings). Letter keys are ignored while typing or with a window open. Code: `src/lib/shortcuts.tsx`.
- Ruled out: background remover, ControlNet, seed grid (Edit Fine-tune stays slim).

---

## 13. Decisions log

- **Styles**: yes, as a separate library combined with the prompt at request time (§7). The
  main prompt is still never stored.
- **Models**: never bundled; one-click download; Pinhole auto-picks the best model per role
  that fits the user's GPU (§6.1).
- **VRAM**: every model shows how much VRAM it needs (§6.2).
- **Paid (early access) models**: hidden by default.
- **Content filter**: Safe mode On (default) · Off. No "adult only" mode; the NSFW tag in the
  Tags multi-select needs Safe mode off (`docs/RELEASE-SPEC.md` §5).
- **Distribution**: GitHub releases from v1.0.0 (a free app; `docs/RELEASE-SPEC.md` sets the
  rules every public build follows).
  A release's text on GitHub is `docs/release-notes/<version>.md`: the Release workflow refuses to
  run without it and publishes it as the release text. The "Release notes" workflow sets an
  existing release's text from the same file.
- **Updates** (manual only): **Update and restart** needs a release whose `SHA256SUMS.txt` is
  signed (`SHA256SUMS.txt.sig`, minisign via `tauri signer sign`) with the maintainer's key. The
  public key is built into the app from `src-tauri/update-key.pub`; the private key is only the
  `PINHOLE_UPDATE_KEY` secret of the `release` environment (usable only from `main` and `v*`
  tags), used by the Release workflow (which checks the signature
  with `scripts/verify-update-signature.mjs` before publishing). `SHA256SUMS.txt` comes from the
  same release, so only its signature catches a release uploaded without the key (it doesn't help
  if the maintainer's GitHub account itself is taken over). While
  `update-key.pub` is empty (`update::SELF_UPDATE` false), every copy is offered
  **Open download page** and nothing is downloaded or installed in the app. The rest of this entry
  describes in-app install. Settings → Check for updates asks
  `api.github.com/repos/DavidGudovic/PinholeAI/releases` for the newest non-draft release (a pre-release
  version such as `1.1.0-rc.1` only for a copy that is itself a pre-release). Download URLs are built from the repo, the tag and the
  expected file name, never taken from the API. The signature must verify, and the file must match GitHub's size and the SHA-256
  in the signed list, or nothing is installed (the list names files by version, so an older signed
  list can't vouch for a newer file). The newest release that has this copy's file and a signed list
  is offered; one without it is offered as "Open download page". Updating is refused while a
  picture is being made or other downloads run (the restart would lose them). Windows installer: the engines stop and the
  NSIS setup runs passively (`/P /UPDATE /R`) and reopens Pinhole. Windows portable: the zip's files
  (never `Data/`) are swapped in beside the running exe and it relaunches. Linux AppImage: the new
  AppImage is renamed over the old one and relaunches. Linux .deb (the running exe is
  `/usr/bin/pinhole` from the `pinhole` package, and `pkexec` is installed): the new .deb is downloaded
  into `Data/.pinhole-update/` and installed with `pkexec apt-get install -y`, which asks for the
  password; then Pinhole relaunches. A closed password dialog leaves the old version in place. Leftovers (`.pinhole-update/`) are removed on
  the next start. The check and the downloads are unauthenticated (public release API and
  download URLs). If GitHub answers 404 (releases can't be seen), the app says so and offers the
  release page. Earlier builds could store a GitHub token in the keychain; the first start after
  the update deletes it (marker `Data/.github-token-cleared`). A copy built before the key existed can't update itself:
  it installs the first signed version by hand.
- **Safety checks** (release): local only, image classifiers on CPU (RELEASE-SPEC §3).
  Prompts are never sent to a server for moderation. Already in: a word check
  (`pinhole-core/src/text_check.rs`) blocks text that pairs an under-18 term with a sexual term,
  whatever Safe mode says: the prompt at Generate in Create and every Edit mode (after styles,
  trigger words and add-ons), the idea sent to Improve my prompt, what Describe / Improve write
  back, and Browse search text.

### Implementation decisions (v1 build-out)
- **Live TAESD preview is deferred**: `sd-server` has no preview API and `--taesd` replaces the
  final VAE decode, so the Create tab shows step progress (parsed from the engine's progress bar)
  instead. `models.yaml → engine_features.taesd_preview` turns it on when the engine supports it.
- **VRAM fitting uses sd.cpp auto-fit** (default in the pinned engine) instead of
  `--offload-to-cpu`, which disables auto-fit and forces every weight into RAM. Low/mid tiers keep
  `--vae-tiling`. Any card tiles the VAE for an output of 2.5 MP or more (hires included): sd.cpp
  retries an out-of-memory decode with tiles by itself but not the encode hires fix does at full
  size (Fine-tune "VAE tiling: Off" wins).
- **Cancel while generating restarts `sd-server`** (the server answers 409 to cancelling a running
  job); the next Generate reloads the model.
- **Linux engine = Ubuntu 24.04+**: upstream only publishes Ubuntu 24.04 builds (glibc 2.38). Building
  our own 22.04 engine is a possible follow-up.
- **"Stay close to original"** maps strict → the low end of the edit family's CFG/guidance range
  (more guidance moves the edit further from the source).
- **WebView is private**: the main window runs incognito (no cookies/cache/storage on disk); in
  portable mode its profile folder lives in `Data/webview`.
- **Observed peak VRAM** is not recorded yet (§6.2 step 3) — follow-up.
- **Running out of graphics memory**: before `sd-server` starts, leftover Pinhole engines (processes
  under `Data/engine/` that this app isn't running) are killed, an idle Describe engine is stopped,
  and on NVIDIA `nvidia-smi` tells how much graphics memory other programs use (a note while
  loading when it's more than a quarter of the card and more than 1 GB). A job that runs out of
  memory is retried with each memory-saving choice at most once: while reading the prompt → text
  encoder on the processor (`--backend te=cpu`, Settings "Read the prompt on the
  processor"); while decoding → `--vae-tiling` (an automatic tiling choice shows in the engine
  note; Fine-tune "VAE tiling: Off" wins over it per request); then, and right away when denoising
  runs out, more of the card is kept free (`--max-vram -4` instead of `-2` on a 16 GB card, less on
  smaller cards, at most a quarter of the card; remembered per model for the app session, shown in
  the engine note); then the
  weights stay in system memory and are sent to the card as needed (`--offload-to-cpu`; only when
  every weight fits in RAM with 2 GB to spare, else tiling as a last resort; kept while the same
  model runs with the same settings, also after the idle stop — another model, other settings or
  deleting it tries the card again; the engine status says so meanwhile). GPU launches pass
  `--max-vram -2` on 12 GB+ cards, `-1` on 8–12 GB, nothing smaller (a registry `--max-vram`
  wins): sd.cpp runs a model in one piece when
  its own estimate (weights + working memory + 0.5 GB) fits the free memory it measured, holding
  every weight on the card for that piece, and on Windows/CUDA the real use ran ~0.85 GB over that
  estimate (a Krea 2 edit on a 16 GB card failed with 1.6 GB free for a 1.9 GB workspace, twice,
  with the weights already in system memory). A budget below free memory makes a model that only
  just fits run in parts instead: the card caches what fits and the rest streams in, slower but it
  finishes, like Forge's reserved inference memory. "failed to encode prompt" without a memory line is not
  treated as running out of memory. The final error (code `vram`, never the generic
  "couldn't make this image") names the other programs when known and says to close them or pick
  the smaller version of the model; the engine output stays behind Details, led by the engine's
  memory plan (sd.cpp auto-fit: free memory and where each part's weights went) from the model's
  last launch. `sd-server` runs at `--log-level info` for that plan, never verbose / debug (they
  print the request). Weights auto-fit keeps in system memory are memory-mapped from the model
  file (`--mmap`) rather than copied into pinned memory, so the OS can page them out; because a
  mapped file can't be deleted on Windows, deleting a model first stops the engine when it runs
  that model or has one of its files open. sd-server's per-tensor "unknown tensor" lines are not
  kept in the output buffer (they can run to hundreds and push out the useful lines).
- Code layout: a Cargo workspace of small crates under `src-tauri/crates/` (see
  `docs/ARCHITECTURE.md`).
- **Local engine API exposure (security review).** Upstream `sd-server` has no authentication,
  answers any CORS `Origin` (with credentials) and keeps every finished job — base64 images
  included — at `GET /sdcpp/v1/jobs/{id}` for 600 s. Another program on this computer, or a web
  page that finds the random port and a job id, could read recent images while the engine runs.
  Interim mitigations: loopback-only random port; the engine is stopped on **Reset** and
  5 min after the last generate/upscale once it has run a job (next Generate reloads the model);
  after start-up Pinhole checks that the server on the port is its own child reporting the model
  it launched (port squatting). `llama-server` (Describe) gets a random per-launch API key via
  `LLAMA_API_KEY` and only `/health` stays public. **Fix (2026-10-01):** `engine.yaml` pins a patched
  `sd-server` from Pinhole's fork (upstream code + `engine/sd-cpp/` patch, built by the fork's
  workflow, `engine/sd-cpp/README.md`). Every launch gets a random key in `SD_API_KEY` (never on
  the command line) and every request sends it as `Authorization: Bearer`; requests without it
  get 401, and any request carrying an `Origin` header gets 403 (`--reject-origin`, added by the
  compiled-in `ENGINE_LOCKDOWN` in `pinhole-core/src/engine.rs`, not a setting, so an unpatched
  engine can't start). The idle stop stays (it also frees memory).

## 14. Open questions

1. Which specific Realistic and Anime models head the `recommended` lists? Pick by testing
   the top-rated candidates on an 8 GB, 12 GB and 16 GB card; record measured VRAM.
