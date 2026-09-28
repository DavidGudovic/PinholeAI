## Summary
<!-- What changed and why. Link the issue: "Fixes #123". -->

## Spec
<!-- Which docs/SPEC.md sections this touches. If it deviates from the spec, update SPEC.md in this PR and say why. -->

## Test plan
<!-- Commands run locally (cargo test --workspace, npm test, npm run build, node scripts/privacy-lint.mjs),
     new/updated tests, and whether the full CI tier was run (Actions → CI → Run workflow, "full") for
     engine / generation / packaging changes. -->

## Screenshots
<!-- Required for UI changes (light + dark). Safe for work, fictional subjects. -->

## Privacy checklist
- [ ] No prompt / negative-prompt text reaches disk, logs, errors, file names, PNG metadata or the network
- [ ] New network calls (if any) go through `pinhole_net::HttpClient` and respect Offline mode
