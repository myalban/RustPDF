#!/usr/bin/env sh
# Builds dist/rustpdf-linux-<arch>.tar.gz: executable + libpdfium.so + desktop entry + icon + install script.
# Usage: installer/build-linux.sh        (run from anywhere; needs the build dependencies listed in the README)
set -eu
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
[ -f libpdfium.so ] || sh scripts/fetch-pdfium.sh
cargo build --release
ARCH="$(uname -m)"
NAME="rustpdf-linux-$ARCH"
OUT="dist/$NAME"
rm -rf "$OUT"
mkdir -p "$OUT" dist
cp target/release/rustpdf libpdfium.so LICENSE README.md "$OUT/"
cp assets/icon.png "$OUT/rustpdf.png"
cp -r licenses "$OUT/licenses"
cat > "$OUT/rustpdf.desktop" <<'DESKTOP'
[Desktop Entry]
Type=Application
Name=RustPDF
Comment=PDF editor
Exec=RUSTPDF_EXEC %f
Icon=RUSTPDF_ICON
Terminal=false
Categories=Office;Viewer;
MimeType=application/pdf;
StartupWMClass=rustpdf
DESKTOP
cat > "$OUT/install.sh" <<'INSTALL'
#!/usr/bin/env sh
# Installs RustPDF for the current user (no root needed): ~/.local/opt/rustpdf + launcher + menu entry.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
mkdir -p "$PREFIX/opt/rustpdf" "$PREFIX/bin" "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/256x256/apps"
cp "$HERE/rustpdf" "$HERE/libpdfium.so" "$PREFIX/opt/rustpdf/"
ln -sf "$PREFIX/opt/rustpdf/rustpdf" "$PREFIX/bin/rustpdf"
cp "$HERE/rustpdf.png" "$PREFIX/share/icons/hicolor/256x256/apps/rustpdf.png"
sed -e "s|RUSTPDF_EXEC|$PREFIX/opt/rustpdf/rustpdf|" -e "s|RUSTPDF_ICON|rustpdf|" "$HERE/rustpdf.desktop" > "$PREFIX/share/applications/rustpdf.desktop"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" || true
echo "RustPDF installed in $PREFIX/opt/rustpdf (command: rustpdf)"
INSTALL
cat > "$OUT/uninstall.sh" <<'UNINSTALL'
#!/usr/bin/env sh
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
rm -rf "$PREFIX/opt/rustpdf" "$PREFIX/bin/rustpdf" "$PREFIX/share/applications/rustpdf.desktop" "$PREFIX/share/icons/hicolor/256x256/apps/rustpdf.png"
echo "RustPDF removed (your settings in ~/.config/rustpdf are kept)"
UNINSTALL
chmod +x "$OUT/rustpdf" "$OUT/install.sh" "$OUT/uninstall.sh"
tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
echo "Created dist/$NAME.tar.gz"
