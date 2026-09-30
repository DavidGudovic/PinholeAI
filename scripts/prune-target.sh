#!/usr/bin/env bash
# Free disk from Cargo's target/ without a full rebuild. Safe to run any time; the next build
# only redoes what it removed.
#
#   scripts/prune-target.sh          drop incremental caches and stale test/example binaries
#   scripts/prune-target.sh --all    cargo clean (everything; the next build is a cold build)
set -euo pipefail
cd "$(dirname "$0")/.."
target="${CARGO_TARGET_DIR:-target}"
[ -d "$target" ] || { echo "no $target directory"; exit 0; }

before=$(du -sm "$target" | cut -f1)
if [ "${1:-}" = "--all" ]; then
  cargo clean
else
  # Incremental caches are the largest and least reusable part (they grow with every edit).
  find "$target" -type d -name incremental -prune -exec rm -rf {} +
  # Test binaries older than a week; Cargo relinks them on demand.
  find "$target" -path '*/deps/*' -type f -perm -u+x -mtime +7 ! -name '*.so' ! -name '*.dll' -delete 2>/dev/null || true
fi
after=$(du -sm "$target" 2>/dev/null | cut -f1 || echo 0)
echo "target: ${before} MB -> ${after:-0} MB"
