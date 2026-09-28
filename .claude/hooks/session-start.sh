#!/bin/bash
# SessionStart hook for Claude Code on the web: makes a fresh cloud session ready to build,
# lint and test Pinhole (Rust workspace + Tauri 2 + React). Idempotent; the container state is
# cached after it finishes, so later sessions start with everything already installed and built.
set -euo pipefail

# Only for remote (web) sessions — local machines manage their own toolchains.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-$(pwd)}"
log() { echo "[session-start] $*" >&2; }

# 1. Tauri 2 Linux build dependencies (WebKitGTK etc.). Skipped when already present.
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null || ! pkg-config --exists dbus-1 2>/dev/null; then
  log "installing Tauri Linux build dependencies (apt)…"
  SUDO=""
  if [ "$(id -u)" != "0" ] && command -v sudo >/dev/null; then SUDO="sudo -n"; fi
  $SUDO apt-get update -qq
  DEBIAN_FRONTEND=noninteractive $SUDO apt-get install -y -qq --no-install-recommends \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
    libsoup-3.0-dev libssl-dev libxdo-dev libdbus-1-dev pkg-config patchelf build-essential file \
    >/dev/null
fi

# 2. Frontend dependencies + a built `dist/` (tauri::generate_context!() embeds ../dist, so the
#    Rust app crate won't compile without it).
log "npm install…"
npm install --no-audit --no-fund --loglevel=error
log "building the frontend (dist/)…"
npm run build --silent >/dev/null || log "frontend build failed — run 'npm run build' to see why (continuing)"

# 3. Rust: fetch crates and pre-compile the workspace tests so the first `cargo test` is fast.
log "cargo fetch + compiling workspace tests (first run takes a few minutes)…"
cargo fetch --locked --quiet || log "cargo fetch failed (network?) — continuing"
cargo test --workspace --locked --no-run --quiet >/dev/null 2>&1 \
  || log "workspace doesn't compile yet — run 'cargo test --workspace --no-run' to see why (continuing)"

# 4. Read-only checkout of the pinned upstream engine source (server API docs, CLI flags,
#    get_sd_version() for registry detection rules) — see CLAUDE.md "Engine rules".
SD_TAG="$(sed -n '/^stable_diffusion_cpp:/,/^[^ ]/{s/^  version: *//p}' config/engine.yaml | head -1)"
SD_SRC="${HOME}/leejet/stable-diffusion.cpp"
# The tag ends in the commit's short hash (master-929-3f8527a); an existing checkout of that commit is kept.
SD_SHA="${SD_TAG##*-}"
if [ -n "$SD_TAG" ] && [ "$(git -C "$SD_SRC" rev-parse --short=7 HEAD 2>/dev/null || true)" != "$SD_SHA" ]; then
  log "cloning stable-diffusion.cpp ${SD_TAG} (read-only reference)…"
  rm -rf "$SD_SRC"
  mkdir -p "$(dirname "$SD_SRC")"
  GIT_LFS_SKIP_SMUDGE=1 git -c advice.detachedHead=false clone -q --depth 1 --branch "$SD_TAG" \
    https://github.com/leejet/stable-diffusion.cpp "$SD_SRC" \
    || log "could not clone stable-diffusion.cpp (network?) — continuing without it"
fi
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  echo "export SD_CPP_SRC=\"$SD_SRC\"" >> "$CLAUDE_ENV_FILE"
fi

log "ready."
