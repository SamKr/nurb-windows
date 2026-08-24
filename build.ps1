# Builds the Windows app in release mode and packages the NSIS installer.
# The result is unsigned (SmartScreen will warn on machines that have not seen
# it); signing belongs to the release process, not to a local build.
$ErrorActionPreference = "Stop"

foreach ($tool in "node", "npm", "cargo", "uv") {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Write-Error @"
'$tool' is not on PATH. The build needs Node (https://nodejs.org), Rust (https://rustup.rs), and uv (https://docs.astral.sh/uv). Install the missing one and reopen the terminal.
"@
    }
}

$desktop = Join-Path $PSScriptRoot "desktop"
if (-not (Test-Path (Join-Path $desktop "node_modules"))) {
    Write-Host "First run: installing frontend dependencies..."
    Push-Location $desktop
    try { npm ci } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { Write-Error "npm ci failed" }
}

Push-Location $desktop
try { npm run tauri build } finally { Pop-Location }
if ($LASTEXITCODE -ne 0) { Write-Error "the build failed; see the output above" }

$bundle = Join-Path $desktop "src-tauri\target\release\bundle\nsis"
$installer = Get-ChildItem $bundle -Filter "*-setup.exe" -ErrorAction SilentlyContinue |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $installer) { Write-Error "the build finished but no installer was found in $bundle" }

Write-Host ""
Write-Host "Installer: $($installer.FullName)"
Write-Host "Standalone exe: $(Join-Path $desktop 'src-tauri\target\release\nurb.exe') (needs an install for first-launch setup; prefer the installer)"
