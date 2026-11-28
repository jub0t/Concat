param()

$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
$concatSource = Join-Path $workspaceRoot "src"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust Cargo is not on PATH. Install Rust 1.93 or newer, then reopen PowerShell."
}

# Stop any running Concat instance from this workspace so Cargo can overwrite concat.exe on Windows.
$runningConcat = Get-Process -Name "concat" -ErrorAction SilentlyContinue
if ($runningConcat) {
    Write-Host "Stopping existing Concat instance before rebuilding..." -ForegroundColor Yellow
    $runningConcat | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

Write-Host "Launching Concat (standalone Threads scraper + native Edge TTS + karaoke subtitles)..." -ForegroundColor Cyan

Push-Location $concatSource
try {
    & cargo run -p concat --profile quick
    $runExitCode = $LASTEXITCODE
} finally {
    Pop-Location
}

exit $runExitCode
