#!/usr/bin/env bash
# ============================================================================
# prepare-pdfium.sh - macOS/Linux twin of prepare-pdfium.ps1
# ----------------------------------------------------------------------------
# Purpose: run before pnpm tauri:build so the chromium/6721 pdfium library is
# bundled into the installer (pdfium-render loads it at runtime — NOT
# statically linked). Target: packages/app/src-tauri/resources/pdfium/libpdfium.dylib
#
# Source priority (same policy as the .ps1 twin):
#   1) Local sodium-prebuilt/pdfium/bin/libpdfium.dylib  (already downloaded by dev)
#   2) Download chromium/6721 from bblanchon/pdfium_binaries (self-bootstrap)
#
# Idempotent: skips if target exists (use --force to re-place).
# NOTE: the Chromium tag MUST match the pdfium_6721 feature pinned in Cargo.toml.
# ============================================================================
set -euo pipefail

CHROMIUM_TAG="${CHROMIUM_TAG:-6721}"   # must match Cargo.toml pdfium_6721 feature
FORCE="${FORCE:-0}"

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET_DIR="$REPO_ROOT/packages/app/src-tauri/resources/pdfium"
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS/$ARCH" in
  Darwin/arm64) PDFIUM_LIB="libpdfium.dylib"; PLATFORM="mac-arm64" ;;
  Darwin/x86_64) PDFIUM_LIB="libpdfium.dylib"; PLATFORM="mac-x64" ;;
  Linux/x86_64) PDFIUM_LIB="libpdfium.so"; PLATFORM="linux-x64" ;;
  Linux/aarch64) PDFIUM_LIB="libpdfium.so"; PLATFORM="linux-arm64" ;;
  *)
    echo "[prepare-pdfium] unsupported platform: $OS/$ARCH" >&2
    exit 1
    ;;
esac

TARGET="$TARGET_DIR/$PDFIUM_LIB"
LOCAL_SRC="$REPO_ROOT/sodium-prebuilt/pdfium/bin/$PDFIUM_LIB"

if [[ "${1:-}" == "--force" ]]; then FORCE=1; fi

if [[ -f "$TARGET" && "$FORCE" != "1" ]]; then
  echo "[prepare-pdfium] $TARGET already present (use --force to re-place)"
  exit 0
fi

mkdir -p "$TARGET_DIR"

# 1) local prebuilt
if [[ -f "$LOCAL_SRC" ]]; then
  echo "[prepare-pdfium] copying from $LOCAL_SRC"
  cp "$LOCAL_SRC" "$TARGET"
  exit 0
fi

# 2) self-bootstrap download.
#    Primary: github.com release redirect (fast outside GFW).
#    Fallback: api.github.com asset endpoint with octet-stream accept — reaches
#    the same bytes when github.com release CDN (objects.githubusercontent.com)
#    is unreachable but api.github.com resolves (common on CN networks).
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F${CHROMIUM_TAG}/pdfium-${PLATFORM}.tgz"
echo "[prepare-pdfium] downloading $URL ..."
if ! curl -fSL --retry 2 --max-time 60 -o "$TMP_DIR/pdfium.tgz" "$URL"; then
  API_URL="https://api.github.com/repos/bblanchon/pdfium-binaries/releases/tags/chromium%2F${CHROMIUM_TAG}"
  echo "[prepare-pdfium] direct download failed; resolving asset via api.github.com ..." >&2
  ASSET_URL="$(curl -fsSL --max-time 30 "$API_URL" | grep -B6 "\"name\": \"pdfium-${PLATFORM}.tgz\"" | grep '"url"' | head -1 | sed -E 's/.*"(https[^"]+)".*/\1/')"
  if [[ -z "$ASSET_URL" ]]; then
    echo "[prepare-pdfium] asset pdfium-${PLATFORM}.tgz not found in release chromium/${CHROMIUM_TAG}" >&2
    exit 1
  fi
  curl -fsSL --retry 3 --max-time 180 -H "Accept: application/octet-stream" "$ASSET_URL" -o "$TMP_DIR/pdfium.tgz"
fi
tar -xzf "$TMP_DIR/pdfium.tgz" -C "$TMP_DIR"
# archive layout: lib/libpdfium.dylib (+ include/)
cp "$TMP_DIR/lib/$PDFIUM_LIB" "$TARGET"
echo "[prepare-pdfium] $TARGET ready"
