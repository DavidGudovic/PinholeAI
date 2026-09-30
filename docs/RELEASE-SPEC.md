# Pinhole — Release Spec

Pinhole is currently **private**: a personal build, used by the developer and shown in demos.
Nothing in this file is needed for that.

Going beyond that happens in three levels. Each level needs everything in its own checklist
(§12) **and** everything in the levels before it.

| Level | What it means | What starts to apply | Checklist |
|---|---|---|---|
| 0 (now) | Private repo; personal use; demos, screen shares, recorded videos, read access to the repo for a code reviewer | nothing in this file | — |
| 1 | **Public repo.** Source code only, no builds | publishing source code (the position of Forge or Stability Matrix); GitHub's Acceptable Use Policy; how the project describes itself | §12.1 |
| 2 | **GitHub release + testers.** Runnable builds | supplying software: EU AI Act marking, the UK offences, model licences | §12.2 |
| 3 | **Marketing.** Posts, listings, a website, store entries | a consumer product: press, platform and store scrutiny | §12.3 |

**Giving anyone a runnable build is Level 2**, even one zip to one friend. A GitHub release on a
public repo can be downloaded by anyone, so "testers only" doesn't narrow who is being supplied.
Showing Pinhole is not supplying it.

Last reviewed: 2026-09-29. Not legal advice — see §10 for when a lawyer is needed.

---

## 0. Why this exists

| Rule | What it requires | From level | Section |
|---|---|---|---|
| GitHub Acceptable Use Policy (synthetic media) | No project "designed for, encourage, promote, support, or suggest" creating non-consensual intimate imagery | 1 | §8, §9 |
| EU AI Act Art. 50(2) (applies since 2 Aug 2026; the open-source exemption does **not** cover it) | Every generated image marked as AI-generated in a machine-readable, detectable way | 2 | §2 |
| UK Crime and Policing Act 2026, s.99 → Sexual Offences Act 2003 s.66I | Offence to make or supply something "for use as" a generator of fake intimate images. **Defence: all reasonable steps taken to prevent non-consensual use.** The offence also covers *making*, so how Pinhole is designed and described matters from Level 1. | 2 | §3, §5, §7 |
| Same Act: child sexual abuse image generators | Offence to supply CSA image generators | 2 | §3, §5 |
| FLUX.1 [dev] / Kontext [dev] Non-Commercial License | Filters or manual review when using the model; licence acceptance | 2 | §3, §6 |
| Krea 2 Community License v1 (§4.2) | "Reasonable and appropriate" content filters for any deployment; licence copy + notice when distributing; commercial use only under $1M revenue | 2 | §3, §6 |
| EU Product Liability Directive, Cyber Resilience Act | Exempt only non-commercial open source | only if monetised | §10 |

The goal is not an unbreakable filter: local open software can always be modified. The goal is
that Pinhole is clearly general-purpose, blocks the few misuse paths the law targets, blocks
**nothing else**, and can show it took reasonable steps. Whoever strips the safeguards from a copy
has built a different tool.

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

---

## 2. AI-generated marking (EU AI Act Art. 50) — Level 2

The EU Code of Practice on marking (final, June 2026) expects **at least two layers**.

- **Metadata** on every saved file (XMP `DigitalSourceType` built 2026-09-30:
  `session::export_png`, the one export function behind Save, Save as and Copy):
  - IPTC/XMP `DigitalSourceType`:
    `http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia` for Create,
    `.../compositeWithTrainedAlgorithmicMedia` for Edit / Restyle.
  - A C2PA manifest (`c2pa` Rust crate): claim generator (C2PA requires one; C2PA is on hold, and if it is added the claim generator must not name Pinhole), action
    `c2pa.created` or `c2pa.edited` with the digital source type. Decide on signing (a
    self-signed certificate shows as "unknown signer" in validators).
- **Invisible pixel watermark** (built 2026-09-30: `pinhole-engine/src/watermark.rs`, a fixed
  spread-spectrum pattern in DCT blocks of a 256×256 brightness grid, the same for everyone,
  ~52 dB PSNR; applied in `session::export_png`) that survives JPEG re-encode, resizing and clipboard copy
  (candidates: Adobe TrustMark — verify licence and ONNX export — or a DWT-DCT watermark ported to Rust).
- **Contents:** "made with AI" only (David, 2026-09-30: no app name or version either). **No prompt, negative prompt, style
  text, seed, user name or machine identifier.** This keeps the privacy rules intact.
- **Always on.** Not a setting, and no switch to turn it off. The existing "Include generation
  settings (no prompt)" setting stays separate and optional.
- It never blocks anything and costs nothing at generation time (it runs at Save / Copy only).
- `embed_image_metadata: false` to `sd-server` stays — its metadata contains the prompt. Pinhole's
  export function writes the marker instead.
- **Fallback, only if testers must start before the watermark is ready:** metadata + C2PA ship
  first and the missing watermark is written down as a known gap in `SAFETY.md`. It must be closed
  before Level 3.
- **Tests:**
  - Saved PNG contains the `DigitalSourceType`, a valid C2PA manifest, and a watermark that is
    still detectable after JPEG q85 re-encode and 50 % downscale.
  - The existing sentinel-prompt privacy test still passes (the marker never contains prompt text).
- **When implemented, update:** SPEC §4 rule 9 (metadata), SPEC §2 engine lifecycle note,
  CLAUDE.md privacy rules (add "the AI-generated marker is the one metadata always written").

---

## 3. Local image check — Level 2

All local and offline. CPU only via ONNX Runtime (`ort` crate) → **zero VRAM**. Loaded on demand,
unloaded when idle (like the captioner). Scores and verdicts are held in memory only and are
**never logged or written anywhere** — same rules as prompts.

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

### 3.2 What is blocked

Three rules. Nothing else is blocked: adult images of fictional adults (Safe mode is a catalog
setting, not a generation filter), swimwear, art nudes and every SFW edit of a real photo stay
allowed.

1. **Real photo made intimate.** The result is Imported, a face is found in one of its Imported
   inputs (image 1, image 2 or a Create reference picture), **and** the result
   is clearly less clothed than the input: its intimate score crosses the threshold **and** is
   higher than the input's by a set margin. "Intimate" means nudity, underwear only, see-through
   clothing or sexual activity — confirm where the UK definition starts (§10) and set the
   threshold there. Comparing with the input is what keeps a beach photo edited to "make it
   sunset" from being blocked. Applies to every mode whose result is Imported (Edit, Restyle, Extend, Create with a reference picture).
2. **Anyone who looks under 18, sexual.** Every mode, every source. The result is explicit
   **and** either:
   - photo style: the age model says under 18 with high confidence, or
   - drawn / anime style: the tagger returns an explicit child tag above its threshold.

   Drawn images never use the age model (it is trained on photos, and adult characters are often
   drawn young). How an image is sorted into photo or drawn style is decided during
   implementation and measured with the rest (§4).
3. **Model flagged for safe images only.** Any resource in the request carries CivitAI's `poi`,
   `minor` or `sfwOnly` flag (§5) → explicit or intimate results are blocked. This uses
   CivitAI's own labels, not a classifier guess about the model.

Block messages (plain words, no engine output):
- Rule 1: "Pinhole doesn't make intimate images of real people from photos."
- Rule 2: "Pinhole doesn't make sexual images of anyone who looks under 18."
- Rule 3: "This model is marked for safe images only, by its creator or by CivitAI."

### 3.3 How it runs

- **In order, cheapest first:** the explicit / intimate classifier runs on every result. The face
  detector runs only on Imported inputs, once, at intake (which also scores the input's intimate
  level for rule 1). The age model or tagger runs only on results already found explicit. Most
  images pay for one small classifier.
- **Coverage:** Create, Variations, Upscale, Restyle, Edit, and live previews if they are ever
  turned on (`engine_features.taesd_preview`).
- **Hide each result until it has been checked** (a few hundred ms). Check image N while image
  N+1 generates.
- **Fail closed:** if a safety model file is missing or its SHA-256 doesn't match, results can't be
  shown and Edit on Imported images is off, with a plain message and a one-click re-download.
- **Budget:** ≤ 1 s per image on CPU, ≤ 500 MB RAM while loaded, 0 VRAM.
- **Config:** thresholds, margins, model URLs and hashes in `config/safety.yaml` (data, not code).
  Code clamps every threshold to a safe range, so the YAML can tune the rules but can't turn one
  off (like Safe mode).

### 3.4 Candidate models

Sizes approximate — **verify each licence before bundling**.

| Job | Candidate | Licence | Size |
|---|---|---|---|
| Face in an Imported input | YuNet (OpenCV Zoo) | MIT | ~0.2 MB |
| Explicit **and** intimate (underwear, see-through), photos | needs classes beyond "explicit"; ViT-base NSFW classifiers (e.g. Falconsai) are explicit-only — verify candidates | Apache 2.0 | ~90–350 MB |
| Apparent age, photos only | pick one whose licence allows redistribution | — | ~1–100 MB |
| Explicit rating + child tags, drawn / anime | WD14-style tagger | Apache 2.0 | ~300–450 MB |

Avoid: InsightFace models (non-commercial research only), NudeNet (AGPL via YOLOv8) unless that
licence is acceptable, the original SD "safety checker" (~1.2 GB, CLIP-L, no better). Don't run
classifiers on the GPU — a second CUDA context alone costs several hundred MB of VRAM.

**Testing rule:** never collect, generate or store prohibited images as test fixtures. Test the
blocking logic with mocked classifier scores; measure false positives on harmless images only (§4).

---

## 4. False positives and the blocked-image experience — Level 2

- **The bar.** Before Level 2, measure every rule in §3.2 on a local set of harmless images for each
  mode: everyday photos, portraits, beach / fitness / swimwear photos, SFW edits of photos with
  faces, adult fictional art (photo and drawn styles), anime including young-looking adult
  characters. A rule ships only if it wrongly blocks **fewer than 1 in 1,000** of those images;
  otherwise narrow it first. Re-measure whenever a model or threshold changes, and on a larger set
  before Level 3.
- **The set stays local** and is never committed. It never contains prohibited content (testing
  rule, §3.4).
- **A block costs the user little:** the prompt, settings and source image are kept; only the
  blocked image is dropped from memory.
- **Nothing is recorded:** no counters, strikes, lockouts or logs. Each block stands alone.
- **Details toggle:** shows which rule fired and the scores (held in memory), so a user can report
  a false positive in a GitHub issue by hand, without the image. Nothing is ever sent automatically.

---

## 5. Catalog and recommended models

- **Safe mode** On (default) / Off, with the once-per-session "I'm 18 or older" confirmation. No
  one-click adult preset.
- **The NSFW tag stays.** It lists exactly the models Safe mode hides and needs Safe mode off.
  "NSFW" is used only as that filter label (§8).
- **CivitAI flags.** `/api/v1/models` returns `poi` (depicts a real person), `minor` (depicts
  someone under 18, usually a child character) and `sfwOnly` (creator asks for no adult content)
  on each model; `/model-versions/*` returns `poi` on its `model` object. The `live_*` catalog test
  fixtures were trimmed and don't contain them — `models_page.json` does. Today the code parses
  `poi` only (`pinhole-catalog/src/api.rs`) and uses none of them.
  - **Level 1:** models with `poi` or `minor` are not offered for install (Browse, model details,
    Paste from CivitAI, Use these settings). `sfwOnly` models show a "Safe images only" badge.
  - **Level 2:** flagged models are shown and installable again. The flags are stored in
    `installed.json` at install time (model metadata, not prompts). Files added by hand get flags
    only from the by-hash lookup "Add a file" already does when it can't tell the type; no
    background or folder-wide lookups (privacy, 2026-09-30). While any flagged resource is loaded, §3.2 rule 3 applies.
    This mirrors CivitAI's own rule and keeps the legitimate SFW uses (satire of public figures,
    historical figures, an avatar model of yourself, child characters in SFW art).
- **Recommended models:**
  - Level 1: remove `sdxl_pony` from `recommended.anime` (Pony stays a supported family; people
    install it themselves).
  - `recommended.realistic_detail` (Krea 2) and `recommended.edit` / `edit_alt` (FLUX.1 Kontext):
    one-click only once §3 ships and §6 licence acceptance is in place.
- **Edit references** (David, 2026-09-30): two-image "Describe a change" and Create's reference
  picture stay, with any picture as input. Every Imported input (image 1, image 2, a Create
  reference picture) makes the result Imported (§3.1), and rule 1's face check looks at all of
  them, so a real person's face brought in as image 2 is covered by the image check (§3).
- **"Edit this image" on CivitAI examples** stays; the image counts as Imported (§3.1).

---

## 6. Licences — Level 2

- Add a `license` field (name + link) to **every** family, component and captioner in
  `config/models.yaml`, and show it on every download — not only on model cards.
- **Require explicit acceptance** before downloading non-commercial or gated models:
  - FLUX.1 Kontext [dev] and FLUX.1 [dev] — non-commercial; requires filters or manual review.
    The configured Kontext URL is a third-party re-upload that skips Black Forest Labs' gate.
  - Qwen2.5-VL-3B (default captioner) — reportedly the Qwen Research (non-commercial) licence;
    verify, or switch to an Apache-2.0 captioner.
  - Krea 2 (one-click `recommended.realistic_detail`, GGUF mirror realrebelai/KREA-2_GGUFs) —
    Krea 2 Community License v1 (LICENSE.pdf in krea-ai/krea-2 and Comfy-Org/Krea-2): allows
    use, copying, redistribution and derivatives, but §4.2 requires content filters for any
    deployment and §2.3 limits commercial use to < $1M yearly revenue. Show the licence
    (link to the PDF) and require acceptance; the mirror's own LICENSE file is empty, so
    Pinhole must show it. §3 is the content filter.
  - Other non-commercial families now in the registry: FLUX.2 dev / klein 9B (FLUX
    Non-Commercial), Anima (CircleStone Labs non-commercial), Qwen-Image 2.1 (Qwen Research),
    SD 3.x (Stability Community License) — licence acceptance before their first download.
- Store only the accepted licence id + version in `settings.yaml`.
- SD 1.5 / SDXL (OpenRAIL-M / ++) use restrictions are repeated in the terms (§7).
- `THIRD_PARTY_LICENSES` covers engines, bundled classifiers and the watermark model.

---

## 7. Terms and notices

- **Level 1:** an acceptable-use section in the README, linking to `SAFETY.md` (§9).
- **Level 2: first-run acceptable-use screen** (click-through). Prohibits: child sexual abuse
  material; sexual or intimate images of real people without consent; deepfakes of real people
  meant to deceive or harass; forged documents, receipts or evidence; harassment. The user is
  responsible for what they make, for model licences and for local law. Store only
  `aup_accepted: <version>`.
- **Level 2: Edit notice**, the first time an Imported image is opened in Edit: "Only edit photos of
  people who have agreed to it. Making sexual or humiliating images of real people without consent
  is a crime in many countries."
- These notices support the safeguards; they don't replace them (§11).
- MIT licence and its disclaimer stay.

---

## 8. Wording and marketing — all levels

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
- Never call Pinhole "safe" or say it "prevents misuse". Say what it blocks ("has safeguards
  against …").
- Edit examples show changes to **scenes, objects, lighting and style** — never changing a real
  person's body or clothes while keeping their face.
- Screenshots and demos: Safe mode on, safe for work, fictional subjects, no celebrities or real
  people.

---

## 9. Paper trail and reporting

- **Level 1: `SAFETY.md`** in the repo root: what Pinhole blocks and doesn't (§3.2), how (on the
  computer, nothing recorded), known limits (open-source code can be modified; classifiers miss
  things), and how to report a problem (GitHub private vulnerability reporting, plus an email
  address).
- **Level 3: a monitored abuse contact with a written process:** what a report can lead to (a rule
  fixed, a threshold tightened, a recommendation or catalog entry removed) and how fast. It
  states plainly that Pinhole can't identify its users or see what they made.
- **Git history is kept** (§11). Dated commits showing when each safeguard was decided and built
  are part of the record of reasonable steps.
- Keep this file's "Last reviewed" date current.

---

## 10. Legal and business

- **Level 2: a short lawyer consult** before the first build is supplied, with these questions:
  1. Is Pinhole a "provider" under EU AI Act Art. 50 when it runs third-party models?
  2. Does one-click install from the catalog make the developer a "supplier" of those models
     under UK law?
  3. Where does the UK definition of an intimate image start (underwear only, see-through
     clothing)? This sets the §3.2 rule 1 threshold.
  4. How does Montenegrin law apply?
  5. Do donations (e.g. GitHub Sponsors) count as commercial activity under the Cyber Resilience
     Act?
- **Level 3:** full lawyer review for the target countries; re-review before each major release.
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
  non-enterprise accounts), it breaks Offline mode, it's bypassable because generation stays
  local, and it creates data-protection obligations for the developer.
- **Telemetry / prompt logging for abuse detection — rejected.** More liability than it removes.
- **Safety classifiers on GPU — rejected.** VRAM cost; CPU is fast enough.
- **Local prompt guard LLM — dropped for v1** (2026-09-29; was planned as Qwen3Guard / Llama Guard
  on `llama-server`). Small guard models are trained on chat, not tag-style image prompts, and
  misfire on ordinary anime prompts; it would cost time and RAM on every Generate; and §3 sees
  what was actually made. Reconsider only as a signal that tightens the §3.2 rule 2 threshold,
  never as a block on its own.
- **Local word check on text — added** (2026-09-30, before §3 exists). `text_check.rs` blocks
  text that pairs an under-18 term with a sexual term, in every Safe mode: the positive prompt
  at Generate in Create and every Edit mode, queued jobs included (style, trigger words and the
  picked add-ons' names and trigger words included; not the negative prompt), the idea sent to
  "Improve my prompt", what Describe / Improve write back, and Browse search text. David first
  limited it to Describe output (#68), then asked for it everywhere (#71). Unlike the dropped
  guard LLM it costs nothing, needs no model, and only fires when both lists match, so ordinary
  anime prompts pass. Word lists are compiled in (not YAML), so a config edit can't turn it off.
  It ignores zero-width characters and fullwidth letters but misses misspellings, look-alike
  letters and made-up words, so it doesn't replace §3.2 rule 2. Required before any helper model
  without its own refusals is offered.
- **A liability warning or consent checkbox instead of safeguards — rejected** (2026-09-29). An
  agreement binds only the user and the developer, not the person in the photo, prosecutors or
  regulators; the UK defence asks for steps that *prevent* non-consensual use; and the app can't
  see consent. The notices in §7 stay as a supplement.
- **Intimate edits of Imported photos with a face are blocked, even with consent** (2026-09-29).
  Consensual use is legal, but it looks identical to non-consensual use. Generated (fictional)
  images are not restricted.
- **Apparent-age estimation on drawn images — rejected** (2026-09-29). Age models are trained on
  photos; drawn images use the tagger's explicit child tags.
- **Always hiding `poi` / `minor` models — replaced** (2026-09-29) by CivitAI's own rule: flagged
  models make generation SFW-only while loaded (§5). Hidden only at Level 1, before that rule exists.
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

## 12. Checklists

### 12.1 Level 1 — public repo

- [x] §5 `poi` / `minor` models not offered for install; `sfwOnly` badge
- [x] §5 `sdxl_pony` removed from `recommended.anime`
- [x] §5 Edit references: covered by origin tracking + the image check (replaces the one-reference cap)
- [ ] §7 acceptable-use section in the README
- [ ] §9 `SAFETY.md` + reporting route (GitHub private vulnerability reporting turned on)
- [ ] §8 wording pass: README, repo description, docs, issue and PR templates, existing issue and
      PR text
- [ ] SPEC.md, CLAUDE.md, PROJECT-BRIEF.md and ARCHITECTURE.md match this file (levels instead of
      one gate; §3 rules; no prompt guard)
- [ ] History scan repeated right before switching the repo to public (secrets, large files, wording)

### 12.2 Level 2 — GitHub release + testers

- [ ] §1 choke points exist and every path goes through them
- [ ] §2 AI marker: metadata + C2PA + watermark, always on, tests pass (or the documented fallback)
- [ ] §3 origin tracking, the three block rules, fail-closed, coverage of every mode
- [ ] §4 false-positive bar met for every rule; block messages and Details toggle
- [ ] §5 flags stored at install; SFW-only rule; flagged models back in the catalog
- [ ] §6 licence field everywhere; acceptance for non-commercial, gated and filter-requiring models
- [ ] §7 first-run acceptable-use screen + Edit notice
- [ ] §10 lawyer consult with the five questions
- [ ] Engine API locked down: ship a patched `sd-server` that rejects any request carrying an
      `Origin` header and requires a per-launch bearer token (SPEC §13 "Local engine API exposure").
      Today any web page open in the user's browser that finds the port can send it jobs or read
      recent images. Decided 2026-09-28: fix before any shared build.
- [ ] Signed updates: release files signed with a key only the maintainer holds (e.g. minisign),
      and "Update and restart" refuses a file whose signature doesn't verify. Today it only checks
      `SHA256SUMS.txt` from the same release (SPEC §13 "Updates").
- [ ] Releases marked as pre-release / test build
- [ ] SPEC.md, CLAUDE.md and the privacy tests updated to match (the AI marker exception)

### 12.3 Level 3 — marketing

- [ ] §10 full lawyer review
- [ ] §2 watermark shipped (if Level 2 used the fallback)
- [ ] §4 false positives re-measured on a larger set
- [ ] §8 wording pass over every public channel: website, listings, screenshots, videos, posts
- [ ] §9 monitored abuse contact + written process
- [ ] Windows code signing (Authenticode, e.g. Azure Trusted Signing), so SmartScreen stops warning
- [ ] Store listing policies checked (winget, Flathub, …) + a short privacy statement (facts:
      nothing is collected)
- [ ] If monetised: §10 company, Product Liability Directive / Cyber Resilience Act, commercial-use
      model lineup

---

## References

- EU AI Act Art. 50 guide: https://artificialintelligenceact.eu/transparency-rules-article-50/
- EU Code of Practice on marking AI content: https://digital-strategy.ec.europa.eu/en/news/commission-publishes-code-practice-marking-and-labelling-ai-generated-content
- UK Crime and Policing Act 2026 (intimate images): https://www.legislation.gov.uk/ukpga/2026/20/part/5/chapter/4/crossheading/intimate-images-etc
- UK CSAM factsheet: https://www.gov.uk/government/publications/crime-and-policing-act-2026-factsheets/crime-and-policing-act-2026-child-sexual-abuse-material-factsheet
- GitHub synthetic media policy: https://docs.github.com/en/site-policy/acceptable-use-policies/github-synthetic-media-and-ai-tools
- FLUX.1 Kontext [dev] model card: https://huggingface.co/black-forest-labs/FLUX.1-Kontext-dev
- CivitAI models API: https://developer.civitai.com/site/reference/models
