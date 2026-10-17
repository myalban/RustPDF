#!/usr/bin/env sh
# Downloads the PDFium library for this machine (Linux / macOS) from bblanchon/pdfium-binaries
# and puts it in the project root, where `cargo run` and `cargo test` look for it.
# The archive is checked against a pinned SHA-256 so that a tampered download is refused.
# Usage: scripts/fetch-pdfium.sh [destination-folder]
#   PDFIUM_VERSION=chromium/XXXX  another build (then also give PDFIUM_SHA256=<hash of the archive>)
set -eu
PINNED_VERSION="chromium/8100"
VERSION="${PDFIUM_VERSION:-$PINNED_VERSION}"
DEST="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64)  PLATFORM=linux-x64;   LIB=lib/libpdfium.so;    SUM=1c1894a12d4d96ee01400c3874444126254180c5c7d2ab628576b67f8120320a ;;
  Linux-aarch64) PLATFORM=linux-arm64; LIB=lib/libpdfium.so;    SUM=58c87a95a40a24c914a9c755fae798843f46f06adf9e5e9f2c893302d4bdb040 ;;
  Darwin-x86_64) PLATFORM=mac-x64;     LIB=lib/libpdfium.dylib; SUM=d513a0530a0f0c1ca9929ded06660e27edb9a108a7d18d9df55a5fb060b091c8 ;;
  Darwin-arm64)  PLATFORM=mac-arm64;   LIB=lib/libpdfium.dylib; SUM=7ea647c7a7a9c1169be3f947000bc071ece2fade2cd6bd7e401e3675c6560cce ;;
  *) echo "Unsupported platform: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac
if [ "$VERSION" != "$PINNED_VERSION" ]; then
  SUM="${PDFIUM_SHA256:-}"
  [ -n "$SUM" ] || { echo "PDFIUM_VERSION=$VERSION is not pinned: also set PDFIUM_SHA256 to the SHA-256 of the archive." >&2; exit 1; }
fi
URL="https://github.com/bblanchon/pdfium-binaries/releases/download/$(printf '%s' "$VERSION" | sed 's|/|%2F|')/pdfium-$PLATFORM.tgz"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
echo "Downloading $URL"
curl -fsSL "$URL" -o "$TMP/pdfium.tgz"
if command -v sha256sum >/dev/null 2>&1; then GOT="$(sha256sum "$TMP/pdfium.tgz" | cut -d' ' -f1)"; else GOT="$(shasum -a 256 "$TMP/pdfium.tgz" | cut -d' ' -f1)"; fi
if [ "$GOT" != "$SUM" ]; then
  echo "Checksum mismatch for $PLATFORM: expected $SUM, got $GOT. Refusing to use this download." >&2
  exit 1
fi
tar -xzf "$TMP/pdfium.tgz" -C "$TMP"
cp "$TMP/$LIB" "$DEST/"
echo "Installed $(basename "$LIB") in $DEST (checksum verified)"
