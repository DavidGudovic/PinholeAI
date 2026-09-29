#!/usr/bin/env bash
# Local pre-merge check: everything the fast CI tier ran, so merging doesn't depend on
# GitHub Actions. Run from anywhere; stops at the first failing step.
#
#   scripts/check.sh            frontend + privacy lint, rustfmt, Rust tests, clippy
#   scripts/check.sh --smoke    also the engine smoke + app end-to-end tests (needs internet
#                               for the engine and a tiny model, CPU only; slow, optional)
#
# Not covered here (run Actions → CI → "Run workflow" when minutes allow): Windows tests,
# the WebDriver e2e, and installers.
set -euo pipefail
cd "$(dirname "$0")/.."

smoke=0
[ "${1:-}" = "--smoke" ] && smoke=1

step() { printf '\n==> %s\n' "$1"; STEP_START=$SECONDS; }
done_() { printf '    ok (%ss)\n' "$((SECONDS - STEP_START))"; }
total_start=$SECONDS

step "npm ci (only if node_modules is missing)"
[ -d node_modules ] || npm ci --no-audit --no-fund
done_

step "Privacy lint self-test + lint"
node scripts/privacy-lint.mjs --self-test
node scripts/privacy-lint.mjs
done_

step "Frontend unit tests (vitest)"
npm test -- --passWithNoTests
done_

# tsc -b && vite build; also produces dist/, which the Tauri crate embeds at compile time.
step "Typecheck + build frontend"
npm run build
done_

step "Rust formatting (cargo fmt --check; fix with: cargo fmt --all)"
cargo fmt --all --check
done_

step "Rust tests (cargo test --workspace, includes the privacy + offline tests)"
cargo test --workspace --locked --no-fail-fast
done_

step "Clippy (warnings are errors)"
cargo clippy --workspace --all-targets --locked -- -D warnings
done_

if [ "$smoke" = 1 ]; then
  export PINHOLE_SMOKE=1
  export PINHOLE_SMOKE_CACHE="${PINHOLE_SMOKE_CACHE:-$PWD/.smoke-cache}"
  export PINHOLE_SMOKE_OUT="${PINHOLE_SMOKE_OUT:-$PWD/smoke-out}"
  step "Engine smoke test (256x256, CPU)"
  cargo test -p pinhole-tests --test engine_smoke --release --locked -- --nocapture
  done_
  step "App end-to-end (engine install, add model, generate, save)"
  cargo test -p pinhole-tests --test app_e2e --release --locked -- --nocapture
  done_
fi

printf '\nAll checks passed in %ss.\n' "$((SECONDS - total_start))"
