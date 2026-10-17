# Prepares and publishes a release: bumps the version, runs the tests, commits, tags and pushes.
# GitHub Actions then builds the Windows, Linux and macOS packages and publishes the release.
#
# Usage:   scripts\release.ps1 -Version 0.3.0            (add -Yes to skip the confirmation question)
#          scripts\release.ps1 -Version 0.3.0-rc1        (a version with a dash becomes a pre-release)
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [switch]$Yes
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

if ($Version -notmatch '^\d+\.\d+\.\d+(-[0-9A-Za-z.]+)?$') { throw "Invalid version '$Version' (expected 1.2.3 or 1.2.3-rc1)" }
$crateVersion = $Version -replace '-.*$', ''
$tag = "v$Version"

if (git status --porcelain) { throw "The working tree is not clean: commit or stash your changes first." }
$branch = (git rev-parse --abbrev-ref HEAD).Trim()
if (git tag --list $tag) { throw "The tag $tag already exists." }
git fetch origin --tags 2>$null | Out-Null

# 1. version in Cargo.toml (the crate version has no suffix)
$toml = Get-Content Cargo.toml -Raw
$new = [regex]::Replace($toml, '(?m)^version = ".*"', "version = `"$crateVersion`"", 1)
if ($new -ne $toml) { Set-Content Cargo.toml $new -NoNewline -Encoding utf8 }

# 2. tests (this also refreshes Cargo.lock)
cargo test --locked 2>&1 | Select-Object -Last 3
if ($LASTEXITCODE -ne 0) {
    cargo test 2>&1 | Select-Object -Last 3
    if ($LASTEXITCODE -ne 0) { git checkout -- Cargo.toml Cargo.lock; throw "Tests failed: release cancelled." }
}

# 3. summary and confirmation
git --no-pager diff --stat
if (-not $Yes) {
    $answer = Read-Host "Commit, tag $tag and push to origin/$branch? (y/N)"
    if ($answer -notin @('y', 'Y', 'yes')) { git checkout -- Cargo.toml Cargo.lock; Write-Host "Cancelled, files restored."; return }
}

# 4. commit, tag, push
git add Cargo.toml Cargo.lock
git commit -m "Release $tag" 2>&1 | Out-Null
git tag -a $tag -m "RustPDF $Version"
git push origin $branch
git push origin $tag
$repo = (git remote get-url origin) -replace '\.git$', '' -replace '^git@github.com:', 'https://github.com/'
Write-Host "`nTag $tag pushed. Follow the build here: $repo/actions"
Write-Host "The release will appear here when the jobs finish: $repo/releases"
