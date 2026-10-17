#!/usr/bin/env sh
# Downloads the PDFium library for this machine (Linux / macOS) from bblanchon/pdfium-binaries
# and puts it in the project root, where `cargo run` and `cargo test` look for it.
# Usage: scripts/fetch-pdfium.sh [destination-folder]      (PDFIUM_VERSION=chromium/XXXX to pick another build)
set -eu
VERSION="${PDFIUM_VERSION:-chromium/8100}"
DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  PLATFORM=linux-x64;  LIB=lib/libpdfium.so ;;
  Linux-aarch64) PLATFORM=linux-arm64; LIB=lib/libpdfium.so ;;
  Darwin-x86_64) PLATFORM=mac-x64;    LIB=lib/libpdfium.dylib ;;
  Darwin-arm64)  PLATFORM=mac-arm64;  LIB=lib/libpdfium.dylib ;;
  *) echo "Unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/$(printf '%s' "$VERSION" | sed 's|/|%2F|')/pdfium-$PLATFORM.tgz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "Downloading $URL"
curl -fsSL "$URL" -o "$TMP/pdfium.tgz"
tar -xzf "$TMP/pdfium.tgz" -C "$TMP"
cp "$TMP/$LIB" "$DEST/"
echo "Installed $(basename "$LIB") in $DEST"
