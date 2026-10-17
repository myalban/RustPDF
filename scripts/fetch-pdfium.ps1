# Downloads pdfium.dll (Windows) from bblanchon/pdfium-binaries into the project root.
# The archive is checked against a pinned SHA-256 so that a tampered download is refused.
# Usage: scripts\fetch-pdfium.ps1 [-Destination <folder>] [-Arch x64|arm64|x86]
#        [-Version chromium/XXXX -Sha256 <hash of the archive>]   (another build must come with its hash)
param(
    [string]$Destination = (Split-Path $PSScriptRoot),
    [string]$Version = "chromium/8100",
    [string]$Arch = "x64",
    [string]$Sha256 = ""
)
$ErrorActionPreference = 'Stop'
$pinnedVersion = "chromium/8100"
$pinned = @{
    'x64'   = '51621bcc1d9be9ea448f4b84b0516862b928e3968d3fca899af1a6d6e2848d6d'
    'arm64' = 'cca9b28d406ae2a902b0b710a92ab699c4c2931564ab6efe44630014d492a9ca'
    'x86'   = 'bdab604368d0116adf84eaab452521a5f200dfa3f97001193d63edb35e75378e'
}
if ($Version -eq $pinnedVersion) {
    if (-not $pinned.ContainsKey($Arch)) { throw "Unknown architecture '$Arch' (x64, arm64 or x86)" }
    $Sha256 = $pinned[$Arch]
} elseif (-not $Sha256) {
    throw "Version $Version is not pinned: also pass -Sha256 with the SHA-256 of the archive."
}
$tag = $Version -replace '/', '%2F'
$url = "https://github.com/bblanchon/pdfium-binaries/releases/download/$tag/pdfium-win-$Arch.tgz"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("pdfium-" + [Guid]::NewGuid())
New-Item -ItemType Directory $tmp | Out-Null
try {
    Write-Host "Downloading $url"
    Invoke-WebRequest $url -OutFile "$tmp\pdfium.tgz"
    $got = (Get-FileHash "$tmp\pdfium.tgz" -Algorithm SHA256).Hash.ToLower()
    if ($got -ne $Sha256.ToLower()) { throw "Checksum mismatch: expected $Sha256, got $got. Refusing to use this download." }
    tar -xzf "$tmp\pdfium.tgz" -C $tmp
    Copy-Item "$tmp\bin\pdfium.dll" $Destination -Force
    Write-Host "Installed pdfium.dll in $Destination (checksum verified)"
} finally {
    Remove-Item $tmp -Recurse -Force -ErrorAction SilentlyContinue
}
