# Safety

Pinhole makes pictures on your own computer. This page explains how it stops harmful content
and how to report a problem. The rules for users are the
**usage guidelines**, shown when Pinhole first opens and whenever the safety check stops something
([source](src/components/UsageGuidelines.tsx)).

## What Pinhole does

Everything below is built into Pinhole and always on. Safe mode (a Models setting) doesn't
change it.

- **Usage guidelines and model licences.** Pinhole shows its usage guidelines before first use,
  and each model's licence before it downloads. Models with a non-commercial or other special
  licence ask you to accept it once first.
- **Word check.** Prompts, "Improve my prompt" ideas, text written by the Describe helper and
  Browse searches are checked against fixed word lists in the code. Text that combines words about
  minors with sexual words is refused, and so is text that asks for a usable copy of an identity
  document or a banknote. A prompt for a picture made from pictures Pinhole made in the same
  session is checked together with the prompts that made them.
- **Image check.** Every picture Pinhole makes is checked before it is shown, by small open
  models that run on the processor (about 1.2 GB, downloaded with the engine). A result is not
  shown when:
  - it is sexual and shows someone who looks under 18: a child, or in a photo-style picture a
    face that looks like a teenager's, or a face close to adult age in a setting or clothing that
    presents the person as under 18;
  - it makes a picture of a person that was brought into Pinhole intimate, whatever that
    picture already showed;
  - it is intimate and comes from a model or add-on that is "safe images only" (its CivitAI
    author marked it so, or it was added from your computer or another app's folder and CivitAI
    hasn't confirmed what it is), or from a picture such a model or add-on made.

  A picture you bring in is checked the same way before it is first described.
  It doesn't block adult content of adults. If the check's files are missing or
  damaged, Pinhole makes nothing until they are downloaded again. When a picture is stopped, it
  isn't shown; your prompt and settings stay so you can change them.
- **Models and add-ons.** Models that CivitAI marks as showing a real person or a minor can't be
  installed from Browse or from pasted generation data. A model or add-on you add from your
  computer, or that Pinhole finds in another app's models folder, is looked up on CivitAI by its
  fingerprint (a SHA-256 hash, not the file) when you add it: one marked as a real person or a
  minor isn't used, and one CivitAI doesn't know is "safe images only". Until it has been looked
  up (for example in Offline mode) it is "safe images only" too, and it is looked up once when
  you turn Offline mode off.
- **Made with AI.** Pictures Pinhole makes carry an invisible watermark when they are saved or
  copied, and a "made with AI" label in their file details when they are saved. Neither contains the
  prompt or anything about you. A picture you bring in loses its other file details, but keeps a
  "made with AI" label if its file had one.

Like any automatic check, these can sometimes make mistakes.
[docs/SAFETY-MATRIX.md](docs/SAFETY-MATRIX.md) lists each thing the usage guidelines don't allow,
what enforces it and how it is tested. The details and models are in
[docs/RELEASE-SPEC.md](docs/RELEASE-SPEC.md) §3.

## Limitations

The automatic checks can make mistakes in both directions: they can block harmless pictures and
can miss some harmful ones. We keep improving them.

- **Safe mode isn't a filter on results.** It decides which models and previews Pinhole shows
  and keeps the prompt helper's ideas safe for work. It doesn't check what a model makes, so it
  isn't a promise that every picture is safe for work.
- **Judging age is hard.** Age checks work from how someone looks in a picture, so they can be
  wrong either way.
- **Pinhole doesn't identify people.** It treats a brought-in picture with a face as a real
  person, and it never recognises who someone is.
- **Consent can't be seen in a picture.** That is why intimate edits of brought-in pictures of
  people are blocked in every case.
- **Model flags come from CivitAI.** Flags such as "safe images only" reflect what CivitAI
  records. A model CivitAI doesn't know is treated as "safe images only".
- **The guidelines cover more than the automatic checks.** Rules such as no deception,
  harassment, forgery or fraud are part of the usage guidelines everyone agrees to; the checks
  cover only part of them.
- **Labels aren't proof.** The "made with AI" label and the watermark mark a picture as made with
  AI. They don't prove where a picture came from or who is in it.
- **How the checks are tested.** With made-up scores and on ordinary pictures. Real harmful
  content is never collected for testing, so there is no figure for how much the checks miss.

## Reporting a problem

- **A safeguard that doesn't work as described, or a security issue:** use GitHub's private
  vulnerability reporting (**Security → Report a vulnerability** on this repository). Please don't open a public issue for
  these. There is no email address for reports.
- **Something harmless was stopped:** open a normal issue and describe what you tried to make.
  Don't attach the picture or anything private.

Please never send or attach illegal content, even to show a problem.
