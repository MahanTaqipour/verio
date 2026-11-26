#Requires -Version 5.1
param(
    [switch]$UseRsproxy,
    [switch]$Offline
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$RepoRoot = Split-Path -Parent $PSScriptRoot
$KeyPath = Join-Path $env:USERPROFILE ".tauri\verio.key"

if (-not (Test-Path $KeyPath)) {
    throw "Signing key not found at $KeyPath. Generate it first with: pnpm tauri signer generate --ci -w `"$KeyPath`""
}

$env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -Raw $KeyPath).Trim()
$env:CARGO_BUILD_JOBS = "1"

$ConfPath = Join-Path $RepoRoot "ui\src-tauri\tauri.conf.json"
$Conf = Get-Content -Raw $ConfPath | ConvertFrom-Json
$Version = [string]$Conf.version
$Endpoint = [string]$Conf.plugins.updater.endpoints[0]

if ($Endpoint -match "github\.com/([^/]+/[^/]+)/releases/") {
    $OwnerRepo = $Matches[1]
} else {
    throw "Could not parse GitHub OWNER/REPO from updater endpoint: $Endpoint"
}

Write-Host "Building signed Windows NSIS release for Verio v$Version ($OwnerRepo) directly on C: with CARGO_BUILD_JOBS=1..."

$CargoExtra = @()
if ($Offline) {
    $CargoExtra += "--offline"
}
if ($UseRsproxy) {
    $CargoExtra += "--config", "source.crates-io.replace-with='rsproxy'", "--config", "source.rsproxy.registry='sparse+https://rsproxy.cn/index/'"
}

Push-Location (Join-Path $RepoRoot "ui")
try {
    if ($CargoExtra.Count -gt 0) {
        pnpm exec tauri build --ci -- @CargoExtra
    } else {
        pnpm tauri build --ci
    }
    if ($LASTEXITCODE -ne 0) {
        throw "pnpm tauri build failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$NsisDir = Join-Path $RepoRoot "target\release\bundle\nsis"
$SetupName = "Verio_${Version}_x64-setup.exe"
$SetupPath = Join-Path $NsisDir $SetupName
$SigPath = "${SetupPath}.sig"

if (-not (Test-Path $SetupPath)) {
    throw "Expected NSIS installer not found: $SetupPath"
}
if (-not (Test-Path $SigPath)) {
    throw "Expected updater signature not found: $SigPath"
}

$Signature = (Get-Content -Raw $SigPath).Trim()
$PubDate = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")

$Manifest = [ordered]@{
    version   = $Version
    notes     = "Verio v$Version"
    pub_date  = $PubDate
    platforms = [ordered]@{
        "windows-x86_64" = [ordered]@{
            signature = $Signature
            url       = "https://github.com/$OwnerRepo/releases/download/v$Version/$SetupName"
        }
    }
}

$LatestJsonPath = Join-Path $NsisDir "latest.json"
$Json = $Manifest | ConvertTo-Json -Depth 5
[System.IO.File]::WriteAllText($LatestJsonPath, $Json, (New-Object System.Text.UTF8Encoding($false)))

Write-Host ""
Write-Host "Release artifacts built successfully!"
Write-Host "Upload these exact 2 files to GitHub Release v${Version}:"
Write-Host "  1) $SetupPath"
Write-Host "  2) $LatestJsonPath"
