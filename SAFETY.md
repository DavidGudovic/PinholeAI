# Safety

Pinhole makes pictures on your own computer. This page explains how it stops harmful content
and how to report a problem. The rules for users are the
**usage guidelines**, shown when Pinhole first opens and whenever the safety check stops something
([source](src/components/UsageGuidelines.tsx)).

## What Pinhole does

Everything below runs on your computer and sends nothing anywhere, except the model lookups
described under "Models and add-ons". It is built into Pinhole and always on. Safe mode (a Models
setting) doesn't change it.

- **Usage guidelines and model licences.** Pinhole shows its usage guidelines before first use,
  and each model's licence before it downloads. Models with a non-commercial or other special
  licence ask you to accept it once first.
- **Word check.** Prompts, "Improve my prompt" ideas, text written by the Describe helper and
  Browse searches are checked against fixed word lists in the code. Text that combines words about
  minors with sexual words is refused.
- **Image check.** Every picture Pinhole makes is checked before it is shown, by small open
  models that run on the processor (about 1.1 GB, downloaded with the engine). A result is not
  shown when:
  - it is sexual and shows someone who looks under 18: a child, or in a photo-style picture a
    face that looks like a teenager's;
  - it makes a picture of a person that was brought into Pinhole intimate, whatever that
    picture already showed;
  - it is intimate and comes from a model or add-on that is "safe images only": its CivitAI
    author marked it so, or it was added from your computer or another app's folder and CivitAI
    hasn't confirmed what it is.

  It doesn't block adult content of adults. If the check's files are missing or
  damaged, Pinhole makes nothing until they are downloaded again.
- **Models and add-ons.** Models that CivitAI marks as showing a real person or a minor can't be
  installed from Browse or from pasted generation data. A model or add-on you add from your
  computer, or that Pinhole finds in another app's models folder, is looked up on CivitAI by its
  fingerprint (a SHA-256 hash, not the file) when you add it: one marked as a real person or a
  minor isn't used, and one CivitAI doesn't know is "safe images only". Until it has been looked
  up (for example in Offline mode) it is "safe images only" too, and it is looked up once when
  you turn Offline mode off.
- **Made with AI.** Every picture Pinhole makes carries an invisible watermark when it is saved or
  copied, and a "made with AI" label in its file details when it is saved. Neither contains the
  prompt or anything about you. A picture you bring in loses its other file details, but keeps a
  "made with AI" label if its file had one.
- **Nothing is reported.** Pinhole never sends anything about what you make to anyone. A stopped
  picture is dropped from memory; your prompt and settings stay so you can change them.

Like any automatic check, these can sometimes make mistakes.
[docs/SAFETY-MATRIX.md](docs/SAFETY-MATRIX.md) lists each thing the usage guidelines don't allow,
what enforces it and how it is tested. The details and models are in
[docs/RELEASE-SPEC.md](docs/RELEASE-SPEC.md) §3.

## Limitations

The safeguards make misuse harder. They can't prevent it.

- **Safe mode isn't a filter on results.** It decides which models and previews Pinhole shows
  and keeps the prompt helper's ideas safe for work. It doesn't check what a model makes, so it
  isn't a promise that every picture is safe for work.
- **Age checks have limits.** Judging age from a picture is hard, most of all for teenagers and
  for drawn characters. The checks are built to stop clear cases and will miss some.
- **Pinhole doesn't know who anyone is.** It can tell that a brought-in photo shows a person, but
  it can't recognize a real person's likeness made another way, such as from a name in the
  prompt or from an add-on model.
- **Consent can't be seen in a picture.** That is why intimate edits of brought-in pictures of
  people are blocked in every case.
- **Model information can be incomplete.** Flags such as "safe images only" come from CivitAI,
  and a model's author may not have set them. A model CivitAI doesn't know is treated as "safe
  images only", but the flags only reflect what CivitAI records.
- **The guidelines cover more than the checks do.** Deception, harassment, forgery and fraud
  aren't allowed, but Pinhole has no check that detects them.
- **Labels aren't proof.** The "made with AI" label can be removed and the watermark doesn't
  survive every change to a picture. Neither proves where a picture came from or who is in it.
- **How often the checks miss something hasn't been measured.** They are tested with made-up
  scores and on ordinary pictures. They can't be tested on real harmful content.
- **Pinhole runs on your computer and its code is public,** so its safeguards can't stop someone
  determined to misuse it.

## Reporting a problem

- **A way around a safeguard, or a security issue:** use GitHub's private vulnerability reporting
  (**Security → Report a vulnerability** on this repository). Please don't open a public issue for
  these. There is no email address for reports.
- **Something harmless was stopped:** open a normal issue and describe what you tried to make.
  Don't attach the picture or anything private.

Please never send or attach illegal content, even to show a problem.
