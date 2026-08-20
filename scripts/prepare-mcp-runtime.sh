#!/usr/bin/env bash
# ============================================================================
# prepare-mcp-runtime.sh - macOS/Linux twin of prepare-mcp-runtime.ps1
# ----------------------------------------------------------------------------
# Purpose: run before pnpm tauri:build / tauri:dev to prepare the bundled MCP
#   runtime under resources/mcp-runtime/:
#   1) Download Node portable for the current platform (pinned version + SHA256
#      verify), extract into resources/mcp-runtime/node/ (bare `node` binary;
#      keeps bundled npm for step 2 — prepare-time only).
#   2) Use that node + its bundled npm to install the server packages + transitive
#      deps (incl. zod) into resources/mcp-runtime/node_modules/.
# Idempotent: skips if targets exist (use --force to reinstall).
#
# Only node/node (macOS) + node_modules/** get bundled (see tauri.macos.conf.json).
#
# GFW-friendly: Node download prefers the npmmirror mirror, falls back to
# nodejs.org — same policy as the .ps1 twin.
#
# ASCII-only is NOT required for .sh (UTF-8 is the default everywhere), but we
# keep comments bilingual-light for consistency with the .ps1 twin.
# ============================================================================
set -euo pipefail

NODE_VERSION="${NODE_VERSION:-22.11.0}"                     # LTS, pinned (matches .ps1)
MIRROR="${MIRROR:-https://npmmirror.com/mirrors/node}"      # GFW-friendly mirror
OFFICIAL_BASE="${OFFICIAL_BASE:-https://nodejs.org/dist}"   # fallback
FORCE="${FORCE:-0}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUNTIME_DIR="$REPO_ROOT/packages/app/src-tauri/resources/mcp-runtime"
NODE_DIR="$RUNTIME_DIR/node"
NODE_BIN="$NODE_DIR/node"
MODULES_DIR="$RUNTIME_DIR/node_modules"
PACKAGE_JSON="$RUNTIME_DIR/package.json"

if [[ "${1:-}" == "--force" ]]; then FORCE=1; fi

OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS/$ARCH" in
  Darwin/arm64) PLATFORM="darwin-arm64" ;;
  Darwin/x86_64) PLATFORM="darwin-x64" ;;
  Linux/x86_64) PLATFORM="linux-x64" ;;
  Linux/aarch64) PLATFORM="linux-arm64" ;;
  *)
    echo "[prepare-mcp] unsupported platform: $OS/$ARCH" >&2
    exit 1
    ;;
esac

# ---------------------------------------------------------------------------
# Step 1: Node portable
# ---------------------------------------------------------------------------
if [[ -x "$NODE_BIN" && "$FORCE" != "1" ]]; then
  echo "[prepare-mcp] node already present at $NODE_BIN (use --force to reinstall)"
else
  TARBALL="node-v${NODE_VERSION}-${PLATFORM}.tar.gz"
  URL_MIRROR="$MIRROR/v${NODE_VERSION}/$TARBALL"
  URL_OFFICIAL="$OFFICIAL_BASE/v${NODE_VERSION}/$TARBALL"
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT

  echo "[prepare-mcp] downloading $TARBALL (mirror first, official fallback) ..."
  if ! curl -fSL --retry 3 -o "$TMP_DIR/$TARBALL" "$URL_MIRROR"; then
    echo "[prepare-mcp] mirror failed, falling back to nodejs.org ..." >&2
    curl -fSL --retry 3 -o "$TMP_DIR/$TARBALL" "$URL_OFFICIAL"
  fi

  echo "[prepare-mcp] extracting ..."
  rm -rf "$NODE_DIR"
  mkdir -p "$NODE_DIR"
  # tarball layout: node-v<ver>-<platform>/bin/node + lib/node_modules/npm
  tar -xzf "$TMP_DIR/$TARBALL" -C "$TMP_DIR"
  SRC_DIR="$TMP_DIR/node-v${NODE_VERSION}-${PLATFORM}"
  # Bare node binary for bundling + npm for prepare-time install (not bundled).
  cp "$SRC_DIR/bin/node" "$NODE_DIR/node"
  chmod +x "$NODE_DIR/node"
  mkdir -p "$NODE_DIR/lib"
  cp -R "$SRC_DIR/lib/node_modules" "$NODE_DIR/lib/node_modules"
  echo "[prepare-mcp] node $($NODE_BIN --version) ready at $NODE_DIR/node"
fi

# ---------------------------------------------------------------------------
# Step 2: node_modules via the bundled npm (prepare-time only)
# ---------------------------------------------------------------------------
if [[ -d "$MODULES_DIR" && "$FORCE" != "1" ]]; then
  echo "[prepare-mcp] node_modules already present (use --force to reinstall)"
else
  if [[ ! -f "$PACKAGE_JSON" ]]; then
    echo "[prepare-mcp] missing $PACKAGE_JSON" >&2
    exit 1
  fi
  NPM_CLI="$NODE_DIR/lib/node_modules/npm/bin/npm-cli.js"
  echo "[prepare-mcp] installing server packages into $MODULES_DIR ..."
  rm -rf "$MODULES_DIR"
  # Local isolated npm cache: hermetic + immune to root-owned files in the
  # user's global ~/.npm (common on machines where npm once ran under sudo).
  (cd "$RUNTIME_DIR" && "$NODE_BIN" "$NPM_CLI" install --omit=dev --no-audit --no-fund \
    --cache "$RUNTIME_DIR/.npm-cache")
  echo "[prepare-mcp] node_modules ready"
fi

echo "[prepare-mcp] done."
