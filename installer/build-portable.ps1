# Builds dist\RustPDF-portable.zip (exe + pdfium.dll + licenses) from the release build.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Push-Location $root
cargo build --release
New-Item -ItemType Directory -Force dist\RustPDF | Out-Null
Copy-Item target\release\rustpdf.exe, pdfium.dll, LICENSE, README.md dist\RustPDF -Force
Copy-Item licenses dist\RustPDF -Recurse -Force
Compress-Archive -Path dist\RustPDF\* -DestinationPath dist\RustPDF-portable.zip -Force
Pop-Location
