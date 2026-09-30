# Safety

Pinhole makes pictures on your own computer. This page explains how it stops harmful content,
where automatic checks fall short, and how to report a problem. The rules for users are the
**usage guidelines**, shown when Pinhole first opens and whenever the safety check stops something
([source](src/components/UsageGuidelines.tsx)).

## What Pinhole does

Everything below runs on your computer and sends nothing anywhere. There is no setting, config
file or environment variable that skips it. Safe mode (a Models setting) doesn't change it.

- **Usage guidelines and model licences.** Pinhole shows its usage guidelines before first use,
  and each model's licence before it downloads. Models with a non-commercial or other special
  licence ask you to accept it once first.
- **Word check.** Prompts, "Improve my prompt" ideas, text written by the Describe helper and
  Browse searches are checked against fixed word lists in the code. Text that combines words about
  minors with sexual words is refused.
- **Image check.** Every picture Pinhole makes is checked before it is shown, by small open
  models that run on the processor (about 1.1 GB, downloaded with the engine). A result is not
  shown when:
  - it is sexual and shows someone who looks like a child;
  - it makes a photo of a real person that was brought into Pinhole intimate, when that photo
    wasn't already;
  - it is intimate and comes from a model its CivitAI author marked "safe images only".

  It doesn't block adult content of adults. If the check's files are missing or
  damaged, Pinhole makes nothing until they are downloaded again.
- **Model catalog.** Models that CivitAI marks as showing a real person or a minor can't be
  installed from Browse or from pasted generation data.
- **Made with AI.** Every saved or copied picture carries a "made with AI" label in its file
  details and an invisible watermark. Neither contains the prompt or anything about you.
- **Nothing is reported.** Pinhole never sends anything about what you make to anyone. A stopped
  picture is dropped from memory; your prompt and settings stay so you can change them.

The details, thresholds and models are in [docs/RELEASE-SPEC.md](docs/RELEASE-SPEC.md) §3.

## Where automatic checks fall short

Automatic checks are never perfect, and we'd rather say so plainly:

- **Classifiers make mistakes.** Unusual styles, crops or low resolution can get past them, and
  sometimes they stop something harmless.
- **Age estimates are rough.** Software can't tell someone's age from a picture reliably, so the
  photo check aims at people who clearly look like children. Drawn pictures are judged by their
  style tags, not by an age estimate.
- **Unmarked models.** A model or add-on of a real person that CivitAI hasn't marked, or one added
  by hand, can't be recognised. The image check still applies to what it makes.
- **Open source.** Anyone can change the code and build their own version without these checks.
  That is outside what an app can prevent; the checks are meant to make misuse of Pinhole as
  shipped impractical, not impossible for a programmer.
- **The engine's local port.** While the image engine runs, another program on the same computer
  can send it requests directly, skipping Pinhole and its checks. A locked-down engine build that
  requires a per-launch key is planned (see the README's Privacy section).

## Reporting a problem

- **A way around a safeguard, or a security issue:** use GitHub's private vulnerability reporting
  (**Security → Report a vulnerability** on this repository). Please don't open a public issue for
  these. There is no email address for reports.
- **Something harmless was stopped:** open a normal issue and describe what you tried to make.
  Don't attach the picture or anything private.

Please never send or attach illegal content, even to show a problem.
