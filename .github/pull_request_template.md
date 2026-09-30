## Summary
<!-- What changed and why. Link the issue: "Fixes #123". -->

## Spec
<!-- Which docs/SPEC.md sections this touches. If it deviates from the spec, update SPEC.md in this PR and say why. -->

## Test plan
<!-- `scripts/check.sh` result (required before merging; Actions no longer run on push), new/updated
     tests, and whether a manual full CI run (Actions → CI → Run workflow, "full") was done for
     Windows-specific / engine / packaging changes. -->

## Screenshots
<!-- Required for UI changes (light + dark). Safe for work, fictional subjects. -->

## Privacy checklist
- [ ] No prompt / negative-prompt text reaches disk, logs, errors, file names, PNG metadata or the network
- [ ] New network calls (if any) go through `pinhole_net::HttpClient` and respect Offline mode, and are added to the "What goes online" list (`src/components/WhatGoesOnline.tsx`)
