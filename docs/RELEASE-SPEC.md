# Pinhole — Release Spec

The safeguards and release rules every public build follows: AI-generated marking, the local
image check, the catalog rules, licences, notices and wording. Pinhole is released on GitHub
(v1.0.0 on 2026-10-01; v1.0.1 the same day under the Pinhole Licence 1.0). The section numbers are
cited from code, tests and `SAFETY.md`; keep them stable. What is still open is in §12.

Last reviewed: 2026-10-01.

---

## 0. Why this exists

| Rule | What it requires | Section |
|---|---|---|
| GitHub Acceptable Use Policy (synthetic media) | No project "designed for, encourage, promote, support, or suggest" creating non-consensual intimate imagery | §8, §9 |
| EU AI Act Art. 50(2) (applies since 2 Aug 2026; the open-source exemption does **not** cover it) | Every generated image marked as AI-generated in a machine-readable, detectable way | §2 |
| UK Crime and Policing Act 2026, s.99 → Sexual Offences Act 2003 s.66I | Offence to make or supply something "for use as" a generator of fake intimate images. **Defence: all reasonable steps taken to prevent non-consensual use.** The offence also covers *making*, so how Pinhole is designed and described matters too. | §3, §5, §7 |
| Same Act: child sexual abuse image generators | Offence to supply CSA image generators | §3, §5 |
| FLUX.1 [dev] / Kontext [dev] Non-Commercial License | Filters or manual review when using the model; licence acceptance | §3, §6 |
| Krea 2 Community License v1 (§4.2) | "Reasonable and appropriate" content filters for any deployment; licence copy + notice when distributing; commercial use only under $1M revenue | §3, §6 |
| EU Product Liability Directive, Cyber Resilience Act | Exempt only non-commercial open source (only if monetised) | §10 |

The goal is not a perfect filter. The goal is that Pinhole is clearly general-purpose, blocks the
few misuse paths the law targets, blocks **nothing else**, and can show it took reasonable steps.

Safeguards that fire on normal work drive people away, so every block rule here is narrow, needs
two signals to agree, and must pass a false-positive bar (§4) before it ships.

---

## 1. Build now: single choke points (cheap, no behaviour change)

Keep these as **one function each** in the Rust core, so §2–§5 can be added later as a few calls
instead of a refactor:

1. **Request builder** — every `img_gen` request (Create, Edit, Restyle, Variations, Upscale)
   is built in one place, after prompt + style + prefix are combined. → flagged-model rule (§3.2 rule 3).
2. **Input intake** — every image entering the Edit tab (Edit this, drag-and-drop, paste,
   file picker, "Edit this image" on a CivitAI example) passes through one function.
   → provenance and face check (§3.1, §3.2 rule 1).
3. **Result intake** — every image coming back from `sd-server`, **including live previews** if
   they are ever turned on, passes through one function before the UI sees it. → image check (§3).
4. **Export** — Save and Copy to clipboard both go through one function. → AI marker (§2).

**Enforced by type (done).** The two checks can't be skipped by a new code path:
- Text: `pinhole_engine::sdapi::ImgGenRequest::new` takes only a `CheckedPrompt`, which only the
  word check makes (`pinhole_engine::words`, wrapped by `pinhole_core::text_check`). `prepare`
  in generate.rs checks the final prompt (idea + style + prefix + trigger words) with the
  add-on names; the request's prompt field is private.
- Pictures: `Session::insert_generated` takes only a `CheckedPng`, which only
  `imagecheck::check_results` makes. Every made picture (Create, Edit in every mode, batches,
  Upscale) reaches the UI through it; imported pictures go in through `import_image` only.
- `pinhole-core/src/one_way.rs` fails if a session picture or a checked value is built
  anywhere else, if the engine's `submit`/`upscale`/`job` is called outside generate.rs, if
  anything but `sdapi.rs` names the engine's picture endpoints, or if the app's Cargo.toml
  turns on the test-only constructors (`test-util`). Release builds build the app package
  alone (`tauri build`, `cargo build -p pinhole`); `cargo build --workspace` would pull
  `test-util` in through `tests/`, so never ship a workspace build.
- Upscale now needs the check's files and can be blocked like any made picture (so an
  upscale of a brought-in photo can hit a false block on a borderline picture).
- Not by type: "Improve my prompt" (the idea sent to the text model) and Describe/Improve
  output are checked with `text_check::check` in describe.rs; that text only reaches the
  image engine through Create, where the type applies.
- Describe intake: a brought-in picture passes `imagecheck::check_before_describe` (the result
  rules, without originals) the first time it is described; Pinhole's own pictures passed result
  intake already. Describe needs the check's files too.

---

## 2. AI-generated marking (EU AI Act Art. 50)

The EU Code of Practice on marking (final, June 2026) expects **at least two layers**.

- **Metadata** on every saved file (XMP `DigitalSourceType` built 2026-09-30:
  `session::export_png`, the one export function behind Save, Save as and Copy):
  - IPTC/XMP `DigitalSourceType`:
    `http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia` for Create,
    `.../compositeWithTrainedAlgorithmicMedia` for Edit / Restyle.
  - No C2PA manifest (dropped 2026-09-30: a self-signed manifest only shows "unknown signer",
    and the XMP marker + watermark already give two layers). If it is ever added, its claim
    generator must not name Pinhole.
- **Invisible pixel watermark** (built 2026-09-30: `pinhole-engine/src/watermark.rs`, a fixed
  spread-spectrum pattern in DCT blocks of a 256×256 brightness grid, the same for everyone,
  ~52 dB PSNR; applied in `session::export_png`) that survives JPEG re-encode, resizing and clipboard copy.
- **Contents:** "made with AI" only (David, 2026-09-30: no app name or version either). **No prompt, negative prompt, style
  text, seed, user name or machine identifier.** This keeps the privacy rules intact.
- **Labels from other tools are carried forward** (2026-10-01, `pinhole-engine/src/provenance.rs`).
  Import drops all metadata (location, camera, prompts), but first reads whether the file says it
  was made with AI: an IPTC digital source type of `trainedAlgorithmicMedia`, `algorithmicMedia`,
  `compositeWithTrainedAlgorithmicMedia` or `algorithmicallyEnhanced`, in XMP (PNG, JPEG, WebP; compressed PNG text too) or in
  a C2PA manifest's actions. Only that label is kept, in memory. On export the unchanged picture
  gets it back as XMP; a picture made only from pictures labelled as made entirely with AI is
  marked `trainedAlgorithmicMedia` (not "edited photo" or "enhanced photo"), and a label is never
  weakened (an upscale of a picture with AI-made parts stays a composite). The label is unsigned
  and says what the file claimed; it is never used by the image check (§3.1). A signed C2PA
  manifest can't be carried over: any edit invalidates it.
- **Always on.** Not a setting, and no switch to turn it off. The existing "Include generation
  settings (no prompt)" setting stays separate and optional.
- It never blocks anything and costs nothing at generation time (it runs at Save / Copy only).
- `embed_image_metadata: false` to `sd-server` stays — its metadata contains the prompt. Pinhole's
  export function writes the marker instead.
- **Tests:** a saved PNG carries the `DigitalSourceType` and a watermark still detectable after
  JPEG q85 re-encode and 50 % downscale; the sentinel-prompt privacy test still passes (the marker
  never contains prompt text).

---

## 3. Local image check

Built 2026-09-30 (crate `pinhole-check`, wired in `pinhole-core/src/imagecheck.rs`). All local
and offline: small ONNX classifiers run on the processor through `tract` (pure Rust, no native
library to download or ship) → **zero VRAM**. Loaded on first use, dropped after 5 minutes idle.
Scores and verdicts are handled like prompts (CLAUDE.md privacy rules). Safe mode doesn't change any of it.

### 3.1 Where each image came from

Built 2026-09-30: `generate::Origin` on every session image (`session.rs`), set at input and
result intake, sent to the UI as `ResultImage.origin`.

Every image in the session carries its origin, inherited by everything made from it:

- **Generated:** the chain starts with a Create generation. Variations, Upscale, Restyle and Edit
  results of such an image are Generated too. These are fictional people.
- **Imported:** anything that entered through input intake (§1, item 2): file picker, drag-and-drop,
  paste, "Edit this image" on a CivitAI example, a saved Pinhole image opened again. It might be
  a real person.
- An image made from an Imported image stays **Imported**, through every Edit, Restyle,
  Variations and Upscale step. Restyling a photo doesn't make it fictional.
- Markers inside a file (C2PA, including Pinhole's own) are **never** trusted to mark an import as
  Generated: anyone can write one onto a real photo.

Each session image also keeps the brought-in pictures it was made from (`SessionImage::made_from`,
memory only), so rule 1 below still sees the original after it is discarded, through any number
of steps. A picture saved in this session and opened again keeps them too (SHA-256 of the saved
file → its sources, memory only, cleared by Reset); a file changed outside Pinhole starts a new
chain as a brought-in picture.

### 3.2 What is blocked

Three rules (`pinhole_check::rules`). Nothing else is blocked: adult images of adults (Safe mode is
a catalog setting, not a generation filter), swimwear, art nudes and every ordinary edit of a real
photo stay allowed. "Intimate" (rules 1 and 3) = an explicit rating ≥ 0.6, or one of the
tagger's nudity tags (including implied nudity: covered or censored) or underwear / lingerie /
see-through tags ≥ 0.5 **and** either nudity score ≥ 0.8 with the tagger's questionable +
explicit rating ≥ 0.5, or an explicit rating ≥ 0.35. The explicit-only path is there because the
tagger was trained on drawings and may not name nudity on a photo. The tags are needed because
the nudity model and the questionable rating also fire on swimwear (2026-09-30 measurement:
about 1 in 5 anime swimwear pictures counted before; ruling: adults-only change, rule 2 stays
as strict). The tags are read by id from the pinned tag list (`run.rs`, `tag_ids`).

1. **Brought-in photo of a person made intimate.** The result is intimate, and one of the
   brought-in pictures it comes from (image 1, image 2, a Create reference picture, at the start
   of any chain of edits) has a face (face finder ≥ 0.6, ≥ 12 px, since Edit and Upscale enlarge a
   small face; the finder works at 640 px, so a picture above 640 px is also searched at its own
   resolution and at half of it in overlapping 640 px windows; with no face upright, a copy shrunk
   to 1280 px is tried turned 90/180/270° and at 45/135/225/315°), whatever it already showed.
   A made picture fed into the request from such a chain (an enlarged or edited step) is
   measured too: a face that shows up in it counts as a person. Comparing with the original
   import (not the direct input) means a photo can't be walked towards intimate in small steps.
   A picture that was already intimate when brought in is no exception (2026-10-01): what a
   picture shows says nothing about the consent of the person in it.
2. **Anyone who looks like a child, sexual.** Every mode, every source. The result is sexual
   (explicit ≥ 0.35, or nudity ≥ 0.85 with questionable + explicit ≥ 0.5; no tags needed, so
   swimwear can count here) **and** either:
   - one of the tagger's tags for a character tagged as a child ≥ 0.5 (drawn or photo), or
   - photo style (tagger `realistic` or `photorealistic` ≥ 0.1; drawings score ~0) and a face
     whose age estimate's child groups (0–2 plus 3–9, `child_face`) reach ≥ 0.6, or whose
     under-20 groups (0–2, 3–9 and 10–19, `under_20_face`) reach ≥ 0.8 (`UNDER_20_FACE`,
     teenagers). The age estimate
     judges faces from 16 px; a clear face (≥ 0.8) under 16 px is too small to judge and counts
     as a child's (fails closed). With no face upright, the result is also tried turned
     90/180/270° (someone lying down). A result above 1280 px is also searched in 640 px
     windows at its own resolution and at half of it (upright).

   Drawn images never use the age estimate (it is trained on photos, and adult characters are
   often drawn young). The under-20 threshold was picked on FairFace validation portraits
   (2026-10-01, `falsepos` example with `FALSEPOS_AGES=1`; labels are apparent ages), accepting
   some wrong blocks of young-looking adults, which only matter on sexual photo-style results.
   The table is the share of ordinary, non-sexual face photos the age half of rule 2 would act on.
   The 10–19 label includes adults aged 18 and 19, so it is not a count of under-18s, and these
   are numbers for the age estimate alone, not how often the whole check catches harmful pictures:

   | Faces the age rule acts on | Labelled 10–19 (1,180) | Labelled 20–29 (1,996) | Labelled 30–39 (367) |
   |---|---|---|---|
   | Child groups ≥ 0.6 only (before) | 15.5 % | 1.7 % | 1.1 % |
   | + under 20 ≥ 0.9 | 23.6 % | 1.8 % | 1.4 % |
   | + under 20 ≥ 0.85 | 35.8 % | 2.3 % | 1.6 % |
   | **+ under 20 ≥ 0.8 (shipped)** | **41.6 %** | **2.9 %** | **1.6 %** |
   | + under 20 ≥ 0.7 | 48.5 % | 4.1 % | 1.9 % |

   The 10–19 group alone never reached 0.8 on any face, so it can't be used on its own.
   Teenagers the estimate misses are left to the word check, the brought-in photo rule
   (rule 1) and, for drawings, the child tags.
   Any check model output that isn't a finite number is an error, so the picture is dropped.
3. **Model marked "safe images only".** The model or a LoRA in the request is "safe images only"
   (`InstalledFile::safe_images_only`), or one was used for a picture this result is made from
   (Edit, Restyle, Fix details, Extend, a Create reference picture, Upscale) → intimate results
   are blocked. The image check works this out itself from how the result was made
   (`imagecheck::MadeBy`); a model that isn't installed any more counts as marked. That is: it carries
   CivitAI's `sfwOnly` flag (stored at install as `CivitaiRef.sfw_only`; when the model's data
   can't be fetched, it counts as set), or it was added by hand or found in a linked folder and
   no CivitAI by-hash lookup has cleared it yet (`InstalledFile::lookup`: not looked up yet, or
   CivitAI doesn't know the file; §5). Models flagged `poi` or `minor` can't be installed or used
   at all (§5). Installed shows a "Safe images only" badge on such models and add-ons. Rules 1
   and 2 don't depend on it.

If one picture of a batch is blocked, the whole batch is dropped.

Block message: one neutral line for every rule and for the text check, "Pinhole can't help with
this. See the usage guidelines." (`text_check::BLOCKED_MESSAGE`), shown with the usage guidelines
(§7). It never names the rule, the content or what triggered it, has no retry hint, and never reads
as an accusation, since a false block can hit an ordinary user (David, 2026-09-30).

### 3.3 How it runs

- **In order:** the nudity classifier and the tagger run on every result (the tagger always, so an
  explicit picture the nudity model scores low still reaches the child tags). A result above
  2048 px or longer than 3:1 is also measured in overlapping square sections (about a third of the
  long side): the first sexual (else intimate) section stands for the picture, and the child and
  photo-style tags are the highest seen in any section. Made pictures are capped at 3:1 and a
  hires fix at 4× (`wiring.rs`, `MAX_ASPECT`, `MAX_HIRES_SCALE`), and sampler, tiling and
  guidance override flags are never passed through from the Fine-tune extra arguments; the face finder and
  age estimate only on sexual photo-style results. Brought-in pictures are measured once (face
  finder only), only when a result made from them is intimate.
  About 2.5 s per result on 4 cores, less on more; both models preload when a job starts.
- **Coverage:** every result of Create, Variations, Restyle, Edit, Fix details and Extend is checked
  before it enters the session (result intake, `generate_inner`), so nothing unchecked reaches the
  UI. Upscale results are checked too (one way in for every made picture).
  There are no live previews; if they are ever turned on they must pass the check too.
- **Fail closed:** Create and Edit stop with `check_missing` ("Set up safety check", one click)
  while any file is absent or has the wrong size; every file is SHA-256 checked as it loads, and a
  damaged one stops the result the same way. A check that fails to run (`check_failed`) drops the
  result. The files download with the engine (Settings → Engine, first run) or from that button.
- **Blocked pictures don't linger:** after a block the engine is stopped once the job ends, since
  it keeps finished jobs readable on its local port.
- **Fixed in code:** the files' URLs (pinned commits), sizes and SHA-256 values and every
  threshold are constants in `pinhole-check`, not config. Tests use a stand-in check.
- **Resources:** ~1.1 GB download; ~2.5 s per result on 4 cores; the models take up to ~1.2 GB RAM while
  loaded, 0 VRAM.

### 3.4 Models (verified by download, 2026-09-30)

| Job | Model (pinned commit on Hugging Face) | Licence | Size |
|---|---|---|
| Nudity (restrictive: revealing clothes score high too) | `AdamCodd/vit-base-nsfw-detector` ONNX (ViT-base 384) | Apache 2.0 | 345 MB |
| Rating + child tags + photo style | `SmilingWolf/wd-vit-tagger-v3` + `selected_tags.csv` | Apache 2.0 | 379 MB |
| Face finder | `opencv/face_detection_yunet` 2023mar | MIT | 0.2 MB |
| Age estimate (photos only) | `onnx-community/fairface_age_image_detection-ONNX` | Apache 2.0 | 343 MB |

Rejected: `Freepik/nsfw_image_detector` (no ONNX), `AdamCodd/vit-nsfw-stable-diffusion` (CC BY-NC-ND,
gated), InsightFace (non-commercial), NudeNet (AGPL), the SD safety checker (~1.2 GB, no better).
The nudity model is weaker on generated pictures (86 % accuracy on its author's test) — one more
reason for §4's measurement.

**Testing rule:** never collect, generate or store prohibited images as test fixtures. The rules
are unit-tested with made-up scores (`pinhole-check` and the core's `testing` tests); false
positives are measured on harmless images only (§4). Dev builds (`npm run tauri dev`) show every
reading of the shown picture under it (all steps, the watermark, whether it would count as a face
in a brought-in photo) and the rule plus scores in a block's Details; release builds show and keep
none. `cargo run --release -p pinhole-check --example measure -- <check dir> <image>…`
prints them for a folder of test pictures.

---

## 4. False positives and the blocked-image experience

- **The bar.** Measure every rule in §3.2 on a local set of harmless images for each
  mode: everyday photos, portraits, beach / fitness / swimwear photos, SFW edits of photos with
  faces, adult fictional art (photo and drawn styles), anime including young-looking adult
  characters. A rule ships only if it wrongly blocks **fewer than 1 in 1,000** of those images;
  otherwise narrow it first. Re-measure whenever a model or threshold changes (status: §12).
- **The tool:** `pinhole-check/examples/falsepos.rs` (2026-10-01: read-only). It never moves or
  deletes a picture, scores the same steps and rules a result goes through in the app
  (`Checker::readings` + `rules::decide`) for a plain Create, a result made from a photo of a
  person and a "safe images only" model, and prints the set's identity (file count and one
  SHA-256 over names and contents) with every denominator: files, unreadable files, check errors,
  pictures measured (and, for the age-only mode, pictures with no judged face). Readings of every
  step on every picture are a separate, opt-in diagnostic (`FALSEPOS_DIAG=1`). Earlier versions
  deleted each picture that would be blocked and scored every step instead of the app's path
  (which also measures large pictures in sections), so figures measured before this change are
  from that version and are re-measured with this one before they are relied on.
- **The set stays local** and is never committed. It never contains prohibited content (testing
  rule, §3.4).
- **A block costs the user little:** the prompt, settings and source image are kept; only the
  blocked image is dropped from memory.
- **No details for users:** release builds show only the neutral message (David, 2026-09-30: never
  say what triggered). Dev builds show the rule and scores for tuning (§3.4). A user can still
  report a false positive in a GitHub issue, describing what they tried.

---

## 5. Catalog and recommended models

- **Safe mode** On (default) / Off, with the once-per-session "I'm 18 or older" confirmation. No
  one-click adult preset.
- **The NSFW tag stays.** It lists exactly the models Safe mode hides and needs Safe mode off.
  "NSFW" is used only as that filter label (§8).
- **CivitAI flags.** `/api/v1/models` returns `poi` (depicts a real person), `minor` (depicts
  someone under 18, usually a child character) and `sfwOnly` (creator asks for no adult content)
  on each model; `/model-versions/*` returns `poi` on its `model` object. The `live_*` catalog test
  fixtures were trimmed and don't contain them — `models_page.json` does. The code reads all
  three (`pinhole-catalog/src/api.rs`: `version_is_person_or_minor`, `sfw_only_of`).
  - Models with `poi` or `minor` are not offered for install (Browse, model details, Paste from
    CivitAI, Use these settings) and can't be used.
  - `sfwOnly` models show a "Safe images only" badge. The flag is stored in `installed.json` at
    install time (`CivitaiRef.sfw_only`, model metadata, not prompts). While any such resource
    is loaded, §3.2 rule 3 applies.
  - **Saved lookup results are signed** (`pinhole-store/src/seal.rs`): HMAC-SHA256 over each
    model's and add-on's lookup result, CivitAI ids and `sfw_only`, with a key made once per
    computer and kept in the OS keychain; the signatures are in `Data/catalog/model-lookups.json`.
    At start, an entry that doesn't match goes back to "not looked up yet" (so "safe images only")
    and the file is hashed again (`lookup::check_seals`). The first start with signing signs what
    is installed. Without a keychain (a Linux desktop with no Secret Service) nothing is signed
    or checked, so models keep working as before.
  - **Files added by hand or linked** (built 2026-10-01, `pinhole-core/src/lookup.rs`): every
    main model and add-on gets a CivitAI by-hash lookup (SHA-256 of the file itself; a linked
    folder's notes are not trusted for it), whether or not its family is already known. A
    `poi`/`minor` match is refused: "Add a file" refuses it, a linked file is not used, and an
    already added file that a later lookup flags can't be used in a picture. A file CivitAI
    doesn't know, or that hasn't been looked up yet, counts as "safe images only" (§3.2 rule 3)
    until a lookup clears it. Files Pinhole offers itself (a SHA-256 listed in the shipped
    `models.yaml`: a download, another quant of it, `known_files` or a test model) never count as
    unchecked. Only the hash counts, never the file name or family, and hashes added in
    `Data/config/overrides.yaml` don't count (2026-10-01). When a download's pin changes, its old
    hash moves to `known_files` so copies already downloaded stay trusted (a registry test keeps
    every hash ever shipped, `src/tests/shipped-hashes.txt`); a file Pinhole can't
    match by hash stays "safe images only" until a lookup clears it.
    - **When lookups run:** only on a user action: "Add a file" (adding the same file again
      retries), adding a linked folder, **Check again** on linked folders, and once when the user
      turns Offline mode off (only files not looked up yet; a "no match" is an answer and isn't
      asked again). Never at start, when Installed opens or in the background. Offline mode
      blocks them like every other call. The app's **What goes online** list names them.
    - **Files from before:** at start, files added by hand (no CivitAI data, not a known
      Pinhole file) or linked before this existed are marked "not looked up yet".
    This mirrors CivitAI's own rule and keeps the legitimate SFW uses (satire of public figures,
    historical figures, an avatar model of yourself, child characters in SFW art).
- **Recommended models:**
  - `sdxl_pony` is not in `recommended.anime` (Pony stays a supported family; people install it
    themselves).
  - `recommended.realistic` / `edit` (Qwen-Image 2.1, Qwen Research License, non-commercial) are
    one-click behind §6 licence acceptance. Krea 2 and FLUX.1 Kontext are not one-click picks.
- **Edit references** (David, 2026-09-30): two-image "Describe a change" and Create's reference
  picture stay, with any picture as input. Every Imported input (image 1, image 2, a Create
  reference picture) makes the result Imported (§3.1), and rule 1's face check looks at all of
  them, so a real person's face brought in as image 2 is covered by the image check (§3).
- **"Edit this image" on CivitAI examples** stays; the image counts as Imported (§3.1).

---

## 6. Licences

- **Open (§12):** a `license` field (name + link) on **every** family, component and captioner in
  `config/models.yaml`, shown on every download — not only on model cards.
- **Explicit acceptance (done).** Families and helpers with a `license_accept` id in
  `config/models.yaml` download only after one "I accept" per licence id (shared by families
  with the same licence). The download fails with code `license_needed` (message = a sentence naming the
  `license_note`, details = the id); the UI's install wrappers show `LicencePrompt` (installs
  asking for the same id at once share one prompt), call `accept_license` and retry. Only ids from the shipped list are accepted and only the ids are
  stored (`Settings.accepted_licenses`); settings and config can't change them. No licence version is stored: a changed licence gets a new id, which
  asks again. The prompt names the licence; a link to its full text is still open (first bullet).
  Current ids:
  - `flux1-dev-non-commercial`: FLUX.1 [dev] and FLUX.1 Kontext [dev]. The configured Kontext
    URL is a third-party re-upload that skips Black Forest Labs' gate, so Pinhole asks instead.
  - `flux2-dev-non-commercial`, `flux2-klein-9b-non-commercial` (FLUX Non-Commercial).
  - `krea2-community`: Krea 2 Turbo / Raw (Krea 2 Community License v1: §4.2 requires content
    filters for any deployment, §2.3 limits commercial use to < $1M yearly revenue; §3 is the
    content filter). The mirror's own LICENSE file is empty, so Pinhole names it and asks.
  - `anima-non-commercial` (CircleStone Labs), `stability-community` (SD 3.x).
  - `qwen-research`: Qwen-Image 2.1 (the one-click Realistic and Edit pick) and the default
    Describe helper Qwen2.5-VL-3B.
- SD 1.5 / SDXL (OpenRAIL-M / ++) use restrictions are repeated in the terms (§7).
- `THIRD_PARTY_LICENSES` covers engines, bundled classifiers and the watermark model.

---

## 7. Terms and notices

- An acceptable-use section in the README (usage guidelines, built-in local check, GitHub
  private reporting) with a link to `SAFETY.md` (§9).
- **First-run acceptable-use screen** (click-through; built 2026-09-30 as "Before you
  start", `src/firstrun/UseNotice.tsx`, stored as `noticeAccepted: <version>`). Short, in the style
  of Adobe Firefly / Bing Image Creator / Midjourney: one privacy line, a "Safety, built in" box
  ("Like other AI image tools, Pinhole has safeguards against harmful content. Unlike most, it does
  this with AI running entirely on your own computer.", David's pick 2026-09-30, trimmed
  2026-10-01 so no privacy line sits next to the safeguards, §8) and "Do not use Pinhole for anything illegal, harmful or
  non-consensual. By continuing, you agree to the usage
  guidelines and to each model's licence. You're responsible for what you make." The full rules
  are the in-app **Usage guidelines** (`src/components/UsageGuidelines.tsx`): no sexual content
  involving anyone under 18 or who looks under 18; no sexual or intimate images of real people
  without consent; no pictures of real people made to deceive, embarrass or harass; no forged
  documents, IDs, receipts or evidence; don't pass made pictures off as real photos; follow model
  licences; what the check stops. The owner dropped "local law" wording (2026-09-30).
- **Block screen:** whenever the check stops something (error code `blocked`), the usage guidelines
  open again with the fixed block message on top (`src/components/BlockedNotice.tsx`, via the
  command wrapper in `src/lib/api.ts`). Never says what triggered it, no retry hint. Calls made
  while typing (prompt preview, Browse search) show the message in place instead.
- **Edit notice** (built 2026-09-30, `editNoticeSeen`), the first time an Imported image is opened in Edit: "Only edit photos of
  people who have agreed to it. Making sexual or humiliating images of real people without consent
  is a crime in many countries."
- These notices support the safeguards; they don't replace them (§11).
- The Pinhole Licence (MIT text plus the Safeguards condition) and its disclaimer stay.

---

## 8. Wording and marketing

Applies to the README, repo description, docs, release notes, screenshots, videos, issue
templates, posts and UI.

- Describe privacy as ownership of your work: "Your prompts and images stay on your computer."
- Never use: "leaves no trace", "untraceable", "no one will know", "uncensored", "unfiltered",
  "NSFW", "undress", "nudify", "face swap". Only exception: "NSFW" as the label of the Browse
  tag filter (CivitAI's own term, so people can find or avoid those models), never in marketing.
- Don't frame privacy as hiding what you made from other people ("forgets everything", "wipes
  your tracks", "nobody will see", "no history"). State facts instead: what stays on the
  computer, what is saved and when, what goes online. Controls get plain names ("Reset", not
  "Clear session" or "Panic"). Portable mode is described as portable, never as "leaves nothing
  behind".
- Don't advertise that pictures or prompts aren't written to disk ("memory only", "nothing on
  disk", "never saved"), and never frame it as privacy or leaving no trace. Where saving needs explaining, say it once, like any editor: "Nothing is
  saved until you press Save."
- Never say that nothing is reported, recorded or sent about what people make, and keep privacy and
  no-telemetry facts out of safeguard text (SAFETY.md, usage guidelines, notices, block message);
  no hints at consequences or monitoring either. Limitations stay honest but neutral, never a
  challenge ("can't stop someone determined") or a pointer to how checks could be removed or fooled.
- Never call Pinhole "safe" or say it "prevents misuse". Say what it blocks ("has safeguards
  against …").
- Edit examples show changes to **scenes, objects, lighting and style** — never changing a real
  person's body or clothes while keeping their face.
- Screenshots and demos: Safe mode on, safe for work, fictional subjects, no celebrities or real
  people.

---

## 9. Paper trail and reporting

- **`SAFETY.md`** in the repo root (written 2026-09-30): what Pinhole blocks and
  doesn't (§3.2), how it works, a "Limitations" section in plain words
  (no workaround detail; 2026-10-01), and how to report a problem (GitHub private vulnerability reporting only,
  no email address; decided 2026-09-30).
- **Before marketing (§12): a monitored abuse contact with a written process:** what a report can lead to (a rule
  fixed, a threshold tightened, a recommendation or catalog entry removed) and how fast.
- **Git history is kept** (§11). Dated commits showing when each safeguard was decided and built
  are part of the record of reasonable steps.
- Keep this file's "Last reviewed" date current.

---

## 10. Jurisdiction and business

- Developer is based in Montenegro: Montenegrin law applies first; the EU AI Act applies when
  distributing to EU users; UK law is relevant when supplying to UK users.
- US TAKE IT DOWN Act: not a "covered platform" (Pinhole hosts nothing). Re-check if that changes.
- **If Pinhole is ever monetised** (paid builds, paid support): set up a company for liability
  protection; re-check the EU Product Liability Directive (software is a product from Dec 2026)
  and the Cyber Resilience Act — both exempt only non-commercial open source; and limit
  recommended models to commercially usable licences. Registry families marked Apache 2.0 or MIT
  today: Z-Image Turbo, Qwen-Image, Qwen Image Edit 2511, FLUX.1 schnell, FLUX.2 klein 4B, Chroma,
  ERNIE-Image, HiDream-O1, Mage-Flow (verify each before relying on it).

---

## 11. Decisions

- **Remote prompt classification — rejected** (e.g. sending prompts to a server running the Jev
  API). Prompts would leave the machine (Jev is API-only; standard data retention for
  non-enterprise accounts), it breaks Offline mode, it doesn't fit Pinhole's local design, and it creates data-protection obligations for the developer.
- **Telemetry / prompt logging — rejected** (privacy rules, CLAUDE.md).
- **Safety classifiers on GPU — rejected.** VRAM cost; CPU is fast enough.
- **Local prompt guard LLM — dropped for v1** (2026-09-29; was planned as Qwen3Guard / Llama Guard
  on `llama-server`). Small guard models are trained on chat, not tag-style image prompts, and
  misfire on ordinary anime prompts; it would cost time and RAM on every Generate; and §3 sees
  what was actually made. Reconsider only as a signal that tightens the §3.2 rule 2 threshold,
  never as a block on its own.
- **Local word check on text — added** (2026-09-30). `text_check.rs` blocks
  text that pairs an under-18 term with a sexual term, in every Safe mode: the positive prompt
  at Generate in Create and every Edit mode, queued jobs included (style, trigger words and the
  picked add-ons' names, trigger words and CivitAI names and trained words included, even when
  the user edited the trigger words; not the negative prompt, so CFG is never sent below 1,
  where the engine would follow the negative prompt), the idea sent to
  "Improve my prompt", what Describe / Improve write back, and Browse search text. David first
  limited it to Describe output (#68), then asked for it everywhere (#71). Unlike the dropped
  guard LLM it costs nothing, needs no model, and only fires when both lists match, so ordinary
  anime prompts pass. Word lists are compiled in (not YAML).
  Before matching it normalizes spellings: invisible characters, fullwidth and styled letters,
  accents, Cyrillic/Greek look-alikes, numbers and symbols for letters, spaced-out letters,
  repeated letters and two listed words glued together (2026-09-30; no text model, by ruling).
  On 27,572 public prompts (Stable-Diffusion-Prompts, midjourney-prompts) it blocked nothing
  new. It is a first line; the image check (§3.2) is the main safeguard. Required before any helper model
  without its own refusals is offered.
- **A liability warning or consent checkbox instead of safeguards — rejected** (2026-09-29). An
  agreement binds only the user and the developer, not the person in the photo, prosecutors or
  regulators; the UK defence asks for steps that *prevent* non-consensual use; and the app can't
  see consent. The notices in §7 stay as a supplement.
- **Intimate edits of Imported photos with a face are blocked, even with consent** (2026-09-29).
  Consensual use is legal, but it looks identical to non-consensual use. Since 2026-10-01 this
  includes pictures that were already intimate when brought in. Pictures made in Pinhole aren't
  restricted by this rule, but where a picture came from never exempts it from rules 2 and 3.
- **Apparent-age estimation on drawn images — rejected** (2026-09-29). Age models are trained on
  photos; drawn images use the tagger's explicit child tags.
- **Always hiding `poi` / `minor` models — replaced** (2026-09-29) by CivitAI's own rule: flagged
  models make generation SFW-only while loaded (§5). `poi` / `minor` models are still not offered (§5).
- **NSFW tag — kept** (2026-09-29). Adult images of fictional adults are a legitimate use. Safe mode
  is on by default, turning it off needs the 18+ confirmation, and there is no one-click adult preset.
- **Rewriting git history before going public — rejected** (2026-09-29). A force push doesn't remove
  old commits: GitHub keeps every pull request's commits (`refs/pull/*`) and only GitHub Support
  can purge them. Old commits add nothing over today's source, and the history is the dated
  record of when safeguards were decided. Full-history scan on 2026-09-29 (220 commits including
  every pull request ref): no secrets, no files over 5 MB, no banned wording in commit messages.
- **Never add these features:**
  - Identity features: PhotoMaker, PuLID, InstantID, IP-Adapter FaceID, face swapping, or
    training a LoRA from photos of a person. The wiring parser knows `--photo-maker` and
    `--pulid-weights` (`pinhole-registry/src/wiring.rs`); nothing exposes them, and nothing will.
  - Concealment features: a panic key, hide-window, a disguised app name or icon, secure delete of
    saved images, clipboard auto-clear, a switch to turn off the AI marker, or stripping other
    tools' AI markers.

---

## 12. Open items

Done items live in git history. Still open:

- **§4 false-positive bar:** measured on about 950 harmless pictures with the earlier measuring
  tool, too few to show 1 in 1,000. Re-measure with the read-only tool, then on a larger set.
- **§6 licence field** (name + link) on every family, component and captioner, shown on every
  download, with a link to the full text in the acceptance prompt.
- **Signed updates:** release files signed with a key only the maintainer holds (e.g. minisign),
  and "Update and restart" refuses a file whose signature doesn't verify. Until then
  `update::SELF_UPDATE = false`: "Check for updates" only opens the release page (SPEC §13
  "Updates"). Then set `SELF_UPDATE = true`.
- **Release files traceable to the source commit** (today `SHA256SUMS.txt` proves integrity, not
  who made them) and **Windows code signing** (Authenticode), so SmartScreen stops warning.
- **Security review** of dependencies and the native engine build.
- **Before any marketing beyond the GitHub release:** §8 wording pass over every channel, §9
  monitored abuse contact, store listing policies (winget, Flathub) with a short privacy
  statement, and §10 if Pinhole is ever monetised.

The engine API lock-down (SPEC §13 "Local engine API exposure") is done: `config/engine.yaml` pins
Pinhole's patched `sd-server`, and the engine smoke test checks 401 without the key and 403 with
an `Origin`.

---

## References

- EU AI Act Art. 50 guide: https://artificialintelligenceact.eu/transparency-rules-article-50/
- EU Code of Practice on marking AI content: https://digital-strategy.ec.europa.eu/en/news/commission-publishes-code-practice-marking-and-labelling-ai-generated-content
- UK Crime and Policing Act 2026 (intimate images): https://www.legislation.gov.uk/ukpga/2026/20/part/5/chapter/4/crossheading/intimate-images-etc
- UK CSAM factsheet: https://www.gov.uk/government/publications/crime-and-policing-act-2026-factsheets/crime-and-policing-act-2026-child-sexual-abuse-material-factsheet
- GitHub synthetic media policy: https://docs.github.com/en/site-policy/acceptable-use-policies/github-synthetic-media-and-ai-tools
- FLUX.1 Kontext [dev] model card: https://huggingface.co/black-forest-labs/FLUX.1-Kontext-dev
- CivitAI models API: https://developer.civitai.com/site/reference/models
