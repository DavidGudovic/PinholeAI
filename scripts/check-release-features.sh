#!/usr/bin/env bash
# Release guard: the app the installers ship must not compile any test-only code (fake image
# checks, unchecked constructors, test HTTP helpers). Those sit behind each crate's `test-util`
# feature, which only the test crates turn on. Cargo unifies features across the packages of
# one build, so this checks the exact package `tauri build` compiles (`pinhole`), with normal
# dependencies only, for every target.
#
#   scripts/check-release-features.sh
set -euo pipefail
cd "$(dirname "$0")/.."

tree=$(cargo tree -p pinhole -e features,normal --target all --locked --prefix none)
if [ -z "$tree" ]; then
  echo "error: cargo tree printed nothing for the app package" >&2
  exit 1
fi
if grep -n 'feature "test-util"' <<<"$tree"; then
  echo "error: the app build turns on a test-only feature (above). Move it to [dev-dependencies] or a test crate." >&2
  exit 1
fi
echo "ok: no test-only features in the app build"
