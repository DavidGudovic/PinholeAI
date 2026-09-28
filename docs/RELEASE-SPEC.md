# Pinhole — Public Release Spec

Pinhole is currently a **private, personal test build**. Nothing in this file is needed for that.

**Everything here must be done before any build is shared with anyone** — a public GitHub
release, a zip for a friend, a store listing. Giving even one copy to someone else counts as
distributing / supplying the software, and the rules below start to apply.

Last reviewed: 2026-09-28. Not legal advice — get a lawyer to review this for the target
countries before the first release, and re-review before each major release.

---

## 0. Why this exists

| Rule | What it requires | Section |
|---|---|---|
| EU AI Act Art. 50(2) (applies since 2 Aug 2026; the open-source exemption does **not** cover it) | Every generated image marked as AI-generated in a machine-readable, detectable way | §2 |
| UK Crime and Policing Act 2026, s.99 → Sexual Offences Act 2003 s.66I | Offence to make or supply something "for use as" a generator of fake intimate images. **Defence: all reasonable steps taken to prevent non-consensual use.** Same Act also criminalises supplying CSA image generators. | §3, §4, §5 |
| FLUX.1 [dev] / Kontext [dev] Non-Commercial License | Filters or manual review when using the model; licence acceptance | §3, §6 |
| GitHub Acceptable Use Policy (synthetic media) | No project "designed for, encourage, promote, support, or suggest" creating non-consensual intimate imagery | §8 |

The goal is not an unbreakable filter (local open software can always be modified). The goal is:
Pinhole is clearly general-purpose, blocks the easy one-click misuse path, and can show it took
reasonable steps. Whoever strips the safeguards from a copy has built a different tool.

---

## 1. Build now: single choke points (cheap, no behaviour change)

Keep these as **one function each** in the Rust core from M1 onward, so §2–§4 can be added later
as a few calls instead of a refactor:

1. **Request builder** — every `img_gen` request (Create, Edit, Restyle, Variations, Upscale)
   is built in one place, after prompt + style + prefix are combined. → guard LLM hook (§4).
2. **Input intake** — every image entering the Edit tab (Edit this, drag-and-drop, paste,
   file picker) passes through one function. → face check hook (§3).
3. **Result intake** — every image coming back from `sd-server`, **including live TAESD
   previews**, passes through one function before the UI sees it. → image check hook (§3).
4. **Export** — Save and Copy to clipboard both go through one function. → AI marker hook (§2).

---

## 2. AI-generated marking (EU AI Act Art. 50)

The EU Code of Practice on marking (final, June 2026) expects **at least two layers**.

- **Metadata** on every saved file:
  - IPTC/XMP `DigitalSourceType`:
    `http://cv.iptc.org/newscodes/digitalsourcetype/trainedAlgorithmicMedia` for Create,
    `.../compositeWithTrainedAlgorithmicMedia` for Edit / Restyle.
  - A C2PA manifest (`c2pa` Rust crate): claim generator `Pinhole <version>`, action
    `c2pa.created` or `c2pa.edited` with the digital source type. Decide on signing (a
    self-signed certificate shows as "unknown signer" in validators).
- **Invisible pixel watermark** that survives JPEG re-encode, resizing and clipboard copy
  (candidates: Adobe TrustMark — verify licence and ONNX export — or a DWT-DCT watermark ported to Rust).
- **Contents:** "made with AI" and the app name/version only. **No prompt, negative prompt, style
  text, seed, user name or machine identifier.** This keeps the privacy rules intact.
- **Always on.** Not a setting. The existing "Include generation settings (no prompt)" setting
  stays separate and optional.
- `embed_image_metadata: false` to `sd-server` stays — its metadata contains the prompt. Pinhole's
  export function writes the marker instead.
- **Tests:**
  - Saved PNG contains the `DigitalSourceType`, a valid C2PA manifest, and a watermark that is
    still detectable after JPEG q85 re-encode and 50 % downscale.
  - The existing sentinel-prompt privacy test still passes (the marker never contains prompt text).
- **When implemented, update:** SPEC §4 rule 9 (metadata), SPEC §2 engine lifecycle note,
  CLAUDE.md privacy rules (add "the AI-generated marker is the one metadata always written").

---

## 3. Local image safety check

All local and offline. CPU only via ONNX Runtime (`ort` crate) → **zero VRAM**. Loaded on demand,
unloaded when idle (like the captioner). Scores and verdicts are held in memory only and are
**never logged or written anywhere** — same rules as prompts.

**Block only these two cases** (keep it narrow to avoid blocking normal work):

1. **Real person + sexual:** the Edit input contains a real photographic face **and** the output
   is sexually explicit.
2. **Minor + sexual:** the output is sexually explicit **and** depicts someone who appears under
   18 — in every mode (Create and Edit), photo or drawn/anime style.

Not blocked: adult explicit images of fictional people when 18+ content is on; swimwear; art
nudes (tune thresholds against a safe test set).

- **Coverage:** final results, live TAESD previews (check them too, or turn previews off in Edit
  when the input has a face), Variations, Upscale.
- **Hide / blur each result until it has been checked** (a few hundred ms).
- **Fail closed:** if a classifier file is missing or its SHA-256 doesn't match, disable the Edit
  tab and 18+ generation with a plain message.
- **Block message** (plain words, no engine dump): "Pinhole doesn't make sexual images of real
  people from photos, or of anyone who looks under 18."
- **Budget:** ≤ 1 s per image on CPU, ≤ 500 MB RAM while loaded, 0 VRAM. Run the check for
  image N while image N+1 generates.
- **Config:** thresholds, model URLs and hashes in `config/safety.yaml` (data, not code).

Candidate models (sizes approximate — **verify each licence before bundling**):

| Job | Candidate | Licence | Size |
|---|---|---|---|
| Face in the Edit input | YuNet (OpenCV Zoo) | MIT | ~0.2 MB |
| Explicit content, photos | ViT-base NSFW classifier (e.g. Falconsai) | Apache 2.0 | ~90–350 MB |
| Apparent age | pick one whose licence allows redistribution | — | ~1–100 MB |
| Explicit rating + age tags, drawn/anime | WD14-style tagger | Apache 2.0 | ~300–450 MB |

Avoid: InsightFace models (non-commercial research only), NudeNet (AGPL via YOLOv8) unless that
licence is acceptable, the original SD "safety checker" (~1.2 GB, CLIP-L, no better). Don't run
classifiers on the GPU — a second CUDA context alone costs several hundred MB of VRAM.

**Testing rule:** never collect, generate or store prohibited images as test fixtures. Test the
blocking logic with mocked classifier scores; measure false positives on a safe image set only.

---

## 4. Local prompt guard

- A small open guard model on the `llama-server` Pinhole already ships, on CPU:
  e.g. Qwen3Guard-Gen 0.6B (Apache 2.0, multilingual) or Llama Guard 3 1B (Llama licence).
  Verify licence and GGUF availability.
- Runs on the **final combined prompt** (prompt + style + prefix) and on Edit instructions,
  inside the request builder (§1). In memory only; verdict never logged or stored.
- Blocks the same narrow categories as §3: sexual content involving minors; sexual content
  involving a real, named person.
- Budget ≤ 300 ms, no VRAM. Same lifecycle as the captioner (idle shutdown).
- Complements §3; does not replace it. The riskiest case (a real photo + a mild instruction)
  is only visible in the image.
- **Never sent to a server.** See Decisions.

---

## 5. Catalog and recommended models

- **CivitAI flags** (on every model in `/api/v1/models`):
  - `minor: true` → never shown in any 18+ mode.
  - `poi: true` (real-person likeness) → never shown in 18+ modes, never offered in the Edit tab.
  - `sfwOnly: true` → show a badge; while such a model is active, §3 blocks explicit output
    regardless of content mode.
- **Remove the "18+ only" content mode.** Keep *Safe only* (default) and *Include 18+*.
  Update SPEC §5.4, §8, §11 (M2), §13 and `config/catalog-filters.yaml` (`only_18plus`).
- **`recommended.anime`:** remove `sdxl_pony`; pick an SFW-leaning Illustrious checkpoint.
- **`recommended.edit`:** FLUX.1 Kontext only if §3 ships and §6 licence acceptance is in place.
- Keep the once-per-session 18+ confirmation; add "I am 18 or older" wording to it.

---

## 6. Licences

- Add a `license` field (name + link) to **every** family, component and captioner in
  `config/models.yaml`, and show it on every download — not only on model cards.
- **Require explicit acceptance** before downloading non-commercial or gated models:
  - FLUX.1 Kontext [dev] and FLUX.1 [dev] — non-commercial; requires filters or manual review.
    The configured Kontext URL is a third-party re-upload that skips Black Forest Labs' gate.
  - Qwen2.5-VL-3B (default captioner) — reportedly the Qwen Research (non-commercial) licence;
    verify, or switch to an Apache-2.0 captioner.
- Store only the accepted licence id + version in `settings.yaml`.
- SD 1.5 / SDXL (OpenRAIL-M / ++) use restrictions are repeated in the terms (§7).
- `THIRD_PARTY_LICENSES` covers engines, bundled classifiers and the watermark model.

---

## 7. Terms and notices

- **First-run Acceptable Use screen** (click-through). Prohibits: child sexual abuse material;
  sexual or intimate images of real people without consent; deepfakes of real people meant to
  deceive or harass; forged documents, receipts or evidence; harassment. The user is responsible
  for model licences and local law. Store only `aup_accepted: <version>`.
- **Edit tab, first use:** one-time notice — "Only edit photos of people who have agreed to it.
  Making sexual or humiliating images of real people without consent is a crime in many countries."
- MIT licence and its disclaimer stay.

---

## 8. Wording and marketing

Applies to the README, repo description, release notes, screenshots, issue templates and UI.

- Describe privacy as ownership of your work: "Your prompts and images stay on your computer."
- Never use: "leaves no trace", "untraceable", "no one will know", "uncensored", "unfiltered",
  "NSFW", "undress", "nudify", "face swap".
- Edit examples show changes to **scenes, objects, lighting and style** — never changing a real
  person's body or clothes while keeping their face.
- Screenshots: safe for work, fictional subjects, no celebrities or real people.

---

## 9. Paper trail

- `SAFETY.md` in the repo root: threat model, the safeguards in §2–§5, known limits, how to report
  abuse, and what happens after a report.
- A monitored abuse-report contact address.
- Keep this file's "Last reviewed" date current.

---

## 10. Legal and business

- Lawyer review for the target countries before the first release.
- Developer is based in Montenegro: Montenegrin law applies first; the EU AI Act applies when
  distributing to EU users; UK law is relevant when supplying to UK users.
- US TAKE IT DOWN Act: not a "covered platform" (Pinhole hosts nothing). Re-check if that changes.
- **If Pinhole is ever monetised** (paid builds, paid support): set up a company for liability
  protection, and re-check the EU Product Liability Directive (software is a product from
  Dec 2026) and the Cyber Resilience Act — both exempt only non-commercial open source.

---

## 11. Decisions

- **Remote prompt classification — rejected** (e.g. sending prompts to a server running the Jev
  API). Prompts would leave the machine (Jev is API-only; standard data retention for
  non-enterprise accounts), it breaks Offline mode, it's bypassable because generation stays
  local, and it creates data-protection obligations for the developer.
- **Telemetry / prompt logging for abuse detection — rejected.** More liability than it removes.
- **Safety classifiers on GPU — rejected.** VRAM cost; CPU is fast enough.

---

## 12. Release checklist

- [ ] §1 choke points exist and every path goes through them
- [ ] §2 AI marker: metadata + C2PA + watermark, always on, tests pass
- [ ] §3 image check: two block rules, previews covered, fail-closed, false-positive rate measured
- [ ] §4 guard LLM on the final combined prompt and Edit instructions
- [ ] §5 CivitAI flags enforced, "18+ only" removed, recommended lists updated
- [ ] §6 licence field everywhere, acceptance for non-commercial / gated models
- [ ] §7 first-run terms + Edit consent notice
- [ ] §8 wording pass over README, repo description, UI, screenshots
- [ ] §9 `SAFETY.md` + abuse contact
- [ ] §10 lawyer review done
- [ ] SPEC.md, CLAUDE.md and the privacy tests updated to match

---

## References

- EU AI Act Art. 50 guide: https://artificialintelligenceact.eu/transparency-rules-article-50/
- EU Code of Practice on marking AI content: https://digital-strategy.ec.europa.eu/en/news/commission-publishes-code-practice-marking-and-labelling-ai-generated-content
- UK Crime and Policing Act 2026 (intimate images): https://www.legislation.gov.uk/ukpga/2026/20/part/5/chapter/4/crossheading/intimate-images-etc
- UK CSAM factsheet: https://www.gov.uk/government/publications/crime-and-policing-act-2026-factsheets/crime-and-policing-act-2026-child-sexual-abuse-material-factsheet
- GitHub synthetic media policy: https://docs.github.com/en/site-policy/acceptable-use-policies/github-synthetic-media-and-ai-tools
- FLUX.1 Kontext [dev] model card: https://huggingface.co/black-forest-labs/FLUX.1-Kontext-dev
- CivitAI models API: https://developer.civitai.com/site/reference/models
