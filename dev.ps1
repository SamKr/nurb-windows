# Launches the nurb desktop app in development mode: stages the bundle inputs,
# starts vite, compiles the Rust shell, and opens the app against this checkout's
# engine. First run compiles the whole dependency tree and takes a few minutes;
# after that it is quick.
$ErrorActionPreference = "Stop"

foreach ($tool in "node", "npm", "cargo", "uv") {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
        Write-Error @"
'$tool' is not on PATH. The dev build needs Node (https://nodejs.org), Rust (https://rustup.rs), and uv (https://docs.astral.sh/uv). Install the missing one and reopen the terminal.
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
try { npm run tauri dev } finally { Pop-Location }
