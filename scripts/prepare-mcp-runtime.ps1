# ============================================================================
# prepare-mcp-runtime.ps1 - Prepare IcePaw bundled MCP runtime
# ----------------------------------------------------------------------------
# Purpose: run before pnpm tauri:build / tauri:dev to prepare the bundled MCP
#   runtime under resources/mcp-runtime/:
#   1) Download Node win-x64 portable (pinned version + SHA256 verify), extract
#      into resources/mcp-runtime/node/ (keeps bundled npm for step 2).
#   2) Use that node + its bundled npm to install the 3 server packages + all
#      transitive deps (incl. zod) into resources/mcp-runtime/node_modules/.
# Idempotent: skips if targets exist (use -Force to reinstall).
#
# Only node/node.exe + node_modules/** get bundled (see tauri.conf.json
# bundle.resources). The bundled npm under node/ is prepare-time only.
#
# GFW-friendly: Node download prefers the npmmirror mirror, falls back to
# nodejs.org. npm install reuses the local npm registry config.
#
# NOTE: This script is intentionally ASCII-only. Windows PowerShell 5.1 reads
# .ps1 files using the system codepage (GBK on zh-CN), so non-ASCII bytes would
# corrupt string parsing.
# ============================================================================

[CmdletBinding()]
param(
    [string] $NodeVersion = "22.11.0",                                  # LTS, pinned (tested in prod)
    [string] $Mirror = "https://npmmirror.com/mirrors/node",            # GFW-friendly mirror
    [string] $OfficialBase = "https://nodejs.org/dist",                 # fallback if mirror fails
    [switch] $Force
)

$ErrorActionPreference = "Stop"

$RepoRoot    = Split-Path $PSScriptRoot                  # scripts/ -> repo root
$RuntimeDir  = Join-Path $RepoRoot "packages/app/src-tauri/resources/mcp-runtime"
$NodeDir     = Join-Path $RuntimeDir "node"
$NodeExe     = Join-Path $NodeDir "node.exe"
$ModulesDir  = Join-Path $RuntimeDir "node_modules"
$PackageJson = Join-Path $RuntimeDir "package.json"

if (-not (Test-Path $PackageJson)) {
    throw "staging package.json not found: $PackageJson"
}

# -- Step 1: Node runtime --------------------------------------------------
if ((-not $Force) -and (Test-Path $NodeExe)) {
    Write-Host "[prepare-mcp] node.exe already present, skip download: $NodeExe" -ForegroundColor DarkGray
} else {
    $zipName = "node-v$NodeVersion-win-x64.zip"
    $zip = Join-Path $env:TEMP $zipName
    $mirrorUrl   = "$Mirror/v$NodeVersion/$zipName"
    $officialUrl = "$OfficialBase/v$NodeVersion/$zipName"

    # Download (mirror first, fallback to official)
    try {
        Write-Host "[prepare-mcp] Downloading Node v$NodeVersion (mirror): $mirrorUrl"
        Invoke-WebRequest -Uri $mirrorUrl -OutFile $zip -UseBasicParsing
    } catch {
        Write-Warning "[prepare-mcp] Mirror download failed, falling back to official: $officialUrl"
        Invoke-WebRequest -Uri $officialUrl -OutFile $zip -UseBasicParsing
    }

    # SHA256 verify (mirror and official SHASUMS256.txt are identical)
    $sumsUrl = "$Mirror/v$NodeVersion/SHASUMS256.txt"
    try {
        $sums = (Invoke-WebRequest -Uri $sumsUrl -UseBasicParsing).Content
        $line = ($sums -split "`n" | Where-Object { $_ -match $zipName } | Select-Object -First 1)
        $expected = ($line -split '\s+' | Select-Object -First 1)
        if ($expected) {
            $actual = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
            if ($actual -ne $expected.ToLower()) {
                throw "SHA256 mismatch: expected=$expected actual=$actual"
            }
            Write-Host "[prepare-mcp] SHA256 verified" -ForegroundColor Green
        } else {
            Write-Warning "[prepare-mcp] No $zipName entry in SHASUMS256.txt, skipping verify"
        }
    } catch {
        Write-Warning "[prepare-mcp] Failed to fetch SHASUMS256 ($($_.Exception.Message)), skipping verify"
    }

    # Extract full portable into node/ (keep bundled npm for step 2)
    $extractRoot = Join-Path $env:TEMP "node-extract-$NodeVersion"
    if (Test-Path $extractRoot) { Remove-Item $extractRoot -Recurse -Force }
    Expand-Archive -Path $zip -DestinationPath $extractRoot -Force
    $inner = Join-Path $extractRoot "node-v$NodeVersion-win-x64"
    if (-not (Test-Path $inner)) { throw "Extracted dir not found: $inner" }
    if (Test-Path $NodeDir) { Remove-Item $NodeDir -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $NodeDir | Out-Null
    Copy-Item (Join-Path $inner "*") $NodeDir -Recurse -Force
    if (-not (Test-Path $NodeExe)) { throw "node.exe not found after extract: $NodeExe" }
    Write-Host "[prepare-mcp] Node ready: $NodeExe" -ForegroundColor Green
}

# -- Step 2: npm install (production) --------------------------------------
# Use the freshly downloaded node + its bundled npm-cli to install the 3 server
# packages + all transitive deps (incl. zod) into node_modules/.
# npm (not pnpm): produces a flat node_modules that bundles and resolves cleanly.
$NpmCli = Join-Path $NodeDir "node_modules\npm\bin\npm-cli.js"
if (-not (Test-Path $NpmCli)) {
    throw "Bundled npm not found: $NpmCli (incomplete node extract? delete node/ and rerun)"
}

if ((-not $Force) -and (Test-Path $ModulesDir)) {
    Write-Host "[prepare-mcp] node_modules already present, skip install (-Force to reinstall)" -ForegroundColor DarkGray
} else {
    Write-Host "[prepare-mcp] npm install --omit=dev (3 servers + transitive deps)"
    Push-Location $RuntimeDir
    try {
        & $NodeExe $NpmCli install --omit=dev --no-audit --no-fund
        if ($LASTEXITCODE -ne 0) { throw "npm install failed (exit $LASTEXITCODE)" }
    } finally { Pop-Location }
    Write-Host "[prepare-mcp] node_modules ready" -ForegroundColor Green
}

# -- Self-check ------------------------------------------------------------
Write-Host ""
Write-Host "[prepare-mcp] Self-check:" -ForegroundColor Cyan
& $NodeExe --version
foreach ($pkg in "@modelcontextprotocol/server-sequential-thinking",
                 "@modelcontextprotocol/server-memory") {
    $entry = Join-Path $ModulesDir "$pkg/dist/index.js"
    if (Test-Path $entry) {
        Write-Host "  [OK]   $pkg/dist/index.js" -ForegroundColor Green
    } else {
        Write-Host "  [MISS] $pkg/dist/index.js" -ForegroundColor Red
    }
}
$zodDir = Join-Path $ModulesDir "zod"
if (Test-Path $zodDir) {
    Write-Host "  [OK]   zod (critical transitive dep)" -ForegroundColor Green
} else {
    Write-Host "  [WARN] zod missing - sequential-thinking will fail at runtime" -ForegroundColor Red
}

Write-Host ""
Write-Host "[prepare-mcp] Done." -ForegroundColor Green
