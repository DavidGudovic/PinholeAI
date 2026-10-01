# Safeguards

This file is part of the [Pinhole Licence](LICENSE) (condition 2). Every copy or changed version
of Pinhole that someone shares must keep these safeguards working at least as strictly as in the
version they started from. What each one does is described in [SAFETY.md](SAFETY.md) and
[docs/SAFETY-MATRIX.md](docs/SAFETY-MATRIX.md).

## The safeguards

1. **Word check** on prompts, styles, trigger words, add-ons, "Improve my prompt", Describe output
   and Browse search (`pinhole-engine/src/words.rs`, `pinhole-core/src/text_check.rs`).
2. **Image check** on every result and on brought-in pictures, with its rules for minors, real
   people in brought-in pictures and "safe images only" models (`pinhole-check`,
   `pinhole-core/src/imagecheck.rs`), in every mode that makes or changes a picture, whatever the
   Safe mode setting.
3. **One way through:** the image engine only takes a word-checked prompt, and a picture only
   reaches the screen through the image check (`pinhole-core/src/one_way.rs`,
   `pinhole-core/src/generate.rs`).
4. **Fail closed:** missing, damaged or unverified check files stop Create, Edit and Upscale
   (`pinhole-check/src/files.rs`).
5. **Model limits:** models and add-ons marked as showing a minor or a real person can't be
   installed or used, and models that aren't recognized count as "safe images only"
   (`pinhole-catalog`, `pinhole-core/src/lookup.rs`).
6. **AI marking:** every saved or copied picture Pinhole made is labelled as made with AI, and
   labels on brought-in pictures are kept (`pinhole-core/src/session.rs`,
   `pinhole-engine/src/watermark.rs`, `pinhole-engine/src/provenance.rs`).
7. **Release gate:** release builds run the safety tests and contain no test-only checks
   (`.github/workflows/release.yml`, `scripts/check-release-features.sh`).

Paths are under `src-tauri/crates/` unless shown otherwise. If code moves, the safeguard moves
with it.

## Reference-test rule

A change may make a safeguard wrongly block less only if all of these hold:

1. **Safety tests:** every test in the safety test run (the `cargo test` command in the release
   workflow's "Safety, enforcement and privacy tests" step) passes, and no existing test was
   removed or changed to accept weaker behaviour.
2. **Catch rate:** on the developer measurements that come with Pinhole (`falsepos` and
   `measure` in `pinhole-check/examples`, `wordcheck` and `doccheck` in `pinhole-engine/examples`), run on
   copies of the same inputs before and after the change, the changed version catches at least
   as many of the cases it is meant to catch. For the age estimates (`falsepos` with
   `FALSEPOS_AGES=1`), the number of faces in folders of people under 20 (the 0–9 and 10–19
   folders, or one folder per age) that the age rule would block does not go down.
3. **Published results:** the numbers from 1 and 2, before and after, are published with the
   change, together with the test inputs used or a way to get them. Test inputs must be lawful,
   openly licensed material.
