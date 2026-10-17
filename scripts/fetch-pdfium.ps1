# Downloads pdfium.dll (Windows x64) from bblanchon/pdfium-binaries into the project root.
# Usage: scripts\fetch-pdfium.ps1 [-Destination <folder>] [-Version chromium/8100] [-Arch x64|arm64|x86]
param(
    [string]$Destination = (Split-Path $PSScriptRoot),
    [string]$Version = "chromium/8100",
    [string]$Arch = "x64"
)
$ErrorActionPreference = 'Stop'
$tag = $Version -replace '/', '%2F'
$url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$tag/pdfium-win-$Arch.tgz"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("pdfium-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "Downloading $url"
    Invoke-WebRequest $url -OutFile "$tmp\pdfium.tgz"
    tar -xzf "$tmp\pdfium.tgz" -C $tmp
    Copy-Item "$tmp\bin\pdfium.dll" $Destination -Force
    Write-Host "Installed pdfium.dll in $Destination"
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
