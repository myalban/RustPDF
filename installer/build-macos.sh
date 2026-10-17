#!/usr/bin/env sh
# Builds dist/RustPDF-macos-<arch>.dmg containing RustPDF.app (executable + libpdfium.dylib + icon).
# Usage: installer/build-macos.sh        (run on a Mac with Xcode command line tools)
# The app is ad-hoc signed, not notarized: on first launch use right-click > Open.
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[ -f libpdfium.dylib ] || sh scripts/fetch-pdfium.sh
cargo build --release
ARCH="$(uname -m)"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
APP="dist/RustPDF.app"
rm -rf "$APP" dist/icon.iconset
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp target/release/rustpdf libpdfium.dylib "$APP/Contents/MacOS/"
# icon: assets/icon.png -> icns
mkdir -p dist/icon.iconset
for s in 16 32 64 128 256; do
  sips -z $s $s assets/icon.png --out "dist/icon.iconset/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/icon.png --out "dist/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns dist/icon.iconset -o "$APP/Contents/Resources/RustPDF.icns"
rm -rf dist/icon.iconset
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>RustPDF</string>
  <key>CFBundleDisplayName</key><string>RustPDF</string>
  <key>CFBundleIdentifier</key><string>org.rustpdf.app</string>
  <key>CFBundleExecutable</key><string>rustpdf</string>
  <key>CFBundleIconFile</key><string>RustPDF</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>PDF Document</string>
      <key>CFBundleTypeRole</key><string>Editor</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key><array><string>com.adobe.pdf</string></array>
    </dict>
  </array>
</dict>
</plist>
PLIST
codesign --force --deep --sign - "$APP"
DMG="dist/RustPDF-macos-$ARCH.dmg"
rm -f "$DMG"
hdiutil create -volname RustPDF -srcfolder "$APP" -ov -format UDZO "$DMG"
echo "Created $DMG"
