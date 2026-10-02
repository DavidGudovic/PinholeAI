# Safety matrix

Each thing the [usage guidelines](../src/components/UsageGuidelines.tsx) prohibit, with what in
Pinhole enforces it, where it applies and how it is tested. "Usage rule only" means Pinhole asks
users not to do it and has no check that detects it. See [SAFETY.md](../SAFETY.md) for the
overview and its Limitations.

Status words:
- **Implemented:** in the code, always on.
- **Tested:** automated tests prove the decision logic and that every path goes through it.
  They use made-up classifier scores, so they don't show how often a classifier is right.
- **Measured:** run on real, openly licensed pictures with the results written down. So far:
  how often the image check stops harmless pictures (about 950 photos, portraits, paintings and
  anime pictures; measured with an earlier version of the measuring tool and due to be
  re-measured with the current, read-only one), how often the word check stops ordinary prompts
  (27,572 public prompts; for documents, about 1.6 million), and how often the age estimates
  act on ordinary, non-sexual face photos (UTKFace portraits with exact ages 10–30, and FairFace
  portraits by apparent age). These sets are small, so the rates are rough. The age figures are
  for the age estimates alone, not how often the whole check catches harmful pictures.
- **Not yet validated:** how often harmful content gets through. It can't be measured with real
  examples of that content, which must never be collected, so there is no figure for it.

The checks run in every mode that makes or changes a picture: Create, Edit (all modes,
including Fix details and Extend), batches, queued jobs and Upscale. Safe mode doesn't change them.

| Guideline: do not create or share content that… | What enforces it | Status | Tests | Not covered |
|---|---|---|---|---|
| sexualizes minors, or anyone who appears to be under 18 | **Word check** on the prompt (with styles, trigger words and add-ons), "Improve my prompt", Describe output and Browse search: text that pairs an under-18 term with a sexual term is refused. **Image check rule 2:** a sexual result with the tagger's child tags (drawn or photo), or a photo-style face that one age estimate places in a child or under-20 age group or the other estimates under 22, or, for a face estimated 22 to 25, a setting, clothing or object that presents the person as under 18. **Models:** models and add-ons CivitAI marks as showing a minor can't be installed or used (Browse, pasted data, and files added by hand or found in linked folders, looked up by hash). | Implemented, tested. Wrong blocks measured on harmless pictures and prompts. On ordinary face photos with exact ages, the age half of rule 2 acts on about 96% of faces aged 10–17 (87% at 16–17), and on about 15% of faces aged 20–25 and 3% aged 26–30; that is the age estimates alone, not an overall detection rate. Detection on sexual content not yet validated. | `pinhole-engine/src/words.rs` tests; `pinhole-check/src/rules.rs` (`sexual_images_with_child_tags_are_blocked`, `sexual_photos_with_a_child_face_are_blocked`, `sexual_photos_with_a_teenage_face_are_blocked`, `origin_never_exempts_a_result_from_rules_2_and_3`); `pinhole-core/src/one_way.rs`; `pinhole-catalog/src/api.rs` | Age detection is incomplete, and weakest for teenagers. The word check matches words, not meaning. |
| shows a real person in a sexual or intimate way without their consent, including edits of their photos | **Image check rule 1:** an intimate result made from any brought-in picture that shows a person, at any step of a chain of edits, whatever that picture already showed. Consent can't be seen in a picture, so the rule applies whether or not there was consent. **Models:** models and add-ons CivitAI marks as showing a real person can't be installed or used. **Design:** no face swap, identity or likeness-training features. | Implemented, tested. Detection not yet validated. | `rules.rs` (`brought_in_photo_made_intimate_is_blocked`, `an_intimate_brought_in_picture_of_a_person_cant_be_edited_intimate`); `pinhole-core/src/tests/check_tests.rs` (`adult_results_pass_unless_made_from_a_brought_in_photo_of_someone`, `a_reopened_save_keeps_the_photo_it_was_made_from`) | A real person's likeness is only recognized as such when it comes from a brought-in picture. Face finding isn't perfect. |
| impersonates, deceives, bullies or harasses real people | Usage rule only. Supporting measures: every saved picture Pinhole made is labelled as made with AI (file details and an invisible watermark; Copy carries the watermark), and labels other tools wrote on brought-in pictures are kept. | Marking implemented, tested (including after re-saving and resizing). | `pinhole-core/src/session.rs`, `pinhole-engine/src/watermark.rs`, `pinhole-engine/src/provenance.rs` tests | Content isn't checked for deception or harassment. Labels mark a picture as made with AI; they aren't proof of anything else. |
| forges documents, IDs, money, receipts or evidence, or is used for fraud | **Word check:** text that names an identity document or a banknote together with a word asking for a usable copy of it is refused. AI marking as above. | Implemented, tested. Wrong blocks measured on public prompts (13 of 1,528,513 and 0 of 81,910). | `pinhole-engine/src/words.rs` (`blocks_usable_copies_of_documents_and_banknotes`, `documents_and_money_in_a_scene_pass`) | Pictures aren't checked for documents, receipts or evidence. The word check matches words, not meaning. |
| is otherwise illegal | Usage rule only. | — | — | Not detected. |
| (models marked "safe images only" by their author) | **Image check rule 3:** an intimate result from a model or add-on CivitAI marks "safe images only" is blocked. A file CivitAI doesn't know, or that hasn't been looked up yet (for example in Offline mode), counts as "safe images only". Files in Pinhole's own model list (matched by SHA-256) are exempt. | Implemented, tested. | `rules.rs` (`safe_images_only_models_cant_make_intimate_images`); `pinhole-catalog/src/api.rs` (`sfw_only_fails_closed_without_the_model`) | The flag is only as complete as CivitAI's data. |

## Always on

- Missing or damaged check files stop Create, Edit and Upscale until they are downloaded again
  (fail closed).
- The check files are verified against SHA-256 values compiled into the app before they load.
- A picture only reaches the screen through the image check, and the image engine only takes a
  word-checked prompt; both are enforced by the types and by `one_way.rs`.
- The Release workflow runs these tests and stops the release if any fails, and checks that no
  test-only code (fake checks) is compiled into the app.
