# ============================================================================
# prepare-pdfium.ps1 - Place pdfium.dll under src-tauri/resources/pdfium/ for bundling
# ----------------------------------------------------------------------------
# Purpose: run before pnpm tauri:build so the chromium/6721 pdfium.dll is bundled
#   into the installer (pdfium-render loads the DLL at runtime — NOT statically
#   linked like libsodium). Target: packages/app/src-tauri/resources/pdfium/pdfium.dll
#
# Source priority:
#   1) Local sodium-prebuilt/pdfium/bin/pdfium.dll  (already downloaded by dev)
#   2) Download chromium/6721 win-x64 from bblanchon/pdfium_binaries (self-bootstrap)
#
# Idempotent: skips if target exists (use -Force to re-place).
#
# NOTE: ASCII-only on purpose. Windows PowerShell 5.1 reads .ps1 using the system
# codepage (GBK on zh-CN), so non-ASCII bytes would corrupt string parsing
# (same convention as prepare-mcp-runtime.ps1).
# ============================================================================

[CmdletBinding()]
param(
    [string] $ChromiumTag = "6721",                       # must match Cargo.toml pdfium_6721 feature
    [switch] $Force
)

$ErrorActionPreference = "Stop"

$RepoRoot  = Split-Path $PSScriptRoot                     # scripts/ -> repo root
$TargetDir = Join-Path $RepoRoot "packages/app/src-tauri/resources/pdfium"
$TargetDll = Join-Path $TargetDir "pdfium.dll"
$LocalSrc  = Join-Path $RepoRoot "sodium-prebuilt/pdfium/bin/pdfium.dll"

if (-not (Test-Path $TargetDir)) {
    New-Item -ItemType Directory -Force -Path $TargetDir | Out-Null
}

# Already placed (idempotent)
if ((-not $Force) -and (Test-Path $TargetDll)) {
    Write-Host "[prepare-pdfium] pdfium.dll already present, skip: $TargetDll" -ForegroundColor DarkGray
    return
}

# Source 1: local sodium-prebuilt (dev machine already has it)
if (Test-Path $LocalSrc) {
    Copy-Item $LocalSrc $TargetDll -Force
    Write-Host "[prepare-pdfium] Copied from local source: $LocalSrc" -ForegroundColor Green
    Write-Host "[prepare-pdfium] -> $TargetDll" -ForegroundColor Green
    return
}

# Source 2: download chromium/<tag> win.x64 from bblanchon/pdfium_binaries
$zipName = "pdfium-win.x64.zip"
$zip     = Join-Path $env:TEMP $zipName
$url     = "https://github.com/bblanchon/pdfium_binaries/releases/download/chromium/$ChromiumTag/$zipName"

Write-Host "[prepare-pdfium] Local source not found ($LocalSrc)." -ForegroundColor Yellow
Write-Host "[prepare-pdfium] Downloading chromium/$ChromiumTag win-x64: $url"
try {
    Invoke-WebRequest -Uri $url -OutFile $zip -UseBasicParsing
} catch {
    throw "Download failed: $($_.Exception.Message). Manually place a chromium/$ChromiumTag pdfium.dll at $LocalSrc or $TargetDll."
}

# Extract — bblanchon zip layout: bin/pdfium.dll
$extract = Join-Path $env:TEMP "pdfium-extract-$ChromiumTag"
if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
Expand-Archive -Path $zip -DestinationPath $extract -Force
$extracted = Join-Path $extract "bin/pdfium.dll"
if (-not (Test-Path $extracted)) {
    throw "Extracted pdfium.dll not found at: $extracted (unexpected zip layout)"
}
Copy-Item $extracted $TargetDll -Force
Write-Host "[prepare-pdfium] Downloaded + extracted: $TargetDll" -ForegroundColor Green

Write-Host "[prepare-pdfium] Done." -ForegroundColor Green
