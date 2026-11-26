param()

$ErrorActionPreference = "Stop"

$workspaceRoot = Split-Path -Parent $PSScriptRoot
$tgsRoot = if ([string]::IsNullOrWhiteSpace($env:TGS_ROOT)) {
    "D:\Work\TGS"
} else {
    $env:TGS_ROOT
}
$tgsBackend = Join-Path $tgsRoot "backend"
$tgsPython = Join-Path $tgsBackend ".venv\Scripts\python.exe"
$concatSource = Join-Path $workspaceRoot "src"
$apiUrl = "http://127.0.0.1:8000"
$healthUrl = "$apiUrl/api/v1/health"
$startedBackend = $false
$tgsProcess = $null
$oldApiUrl = $env:TGS_API_URL
$runExitCode = 0

function Test-TgsBackend {
    try {
        $health = Invoke-RestMethod -Uri $healthUrl -TimeoutSec 2
        return $health.status -eq "ok"
    } catch {
        return $false
    }
}

if (-not (Test-Path -LiteralPath $tgsPython -PathType Leaf)) {
    throw "TGS Python environment was not found at '$tgsPython'. Set TGS_ROOT if your TGS checkout is elsewhere."
}
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust Cargo is not on PATH. Install Rust 1.93 or newer, then reopen PowerShell."
}

if (-not (Test-TgsBackend)) {
    $logStamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $backendStdout = Join-Path $env:TEMP "concat-tgs-$logStamp.out.log"
    $backendStderr = Join-Path $env:TEMP "concat-tgs-$logStamp.err.log"
    $tgsProcess = Start-Process `
        -FilePath $tgsPython `
        -ArgumentList @("-m", "uvicorn", "app.main:app", "--host", "127.0.0.1", "--port", "8000") `
        -WorkingDirectory $tgsBackend `
        -WindowStyle Hidden `
        -RedirectStandardOutput $backendStdout `
        -RedirectStandardError $backendStderr `
        -PassThru
    $startedBackend = $true

    $backendReady = $false
    for ($attempt = 0; $attempt -lt 60; $attempt++) {
        if (Test-TgsBackend) {
            $backendReady = $true
            break
        }
        if ($tgsProcess.HasExited) {
            $details = if (Test-Path -LiteralPath $backendStderr) {
                Get-Content -LiteralPath $backendStderr -Tail 20 | Out-String
            } else {
                "No backend log was created."
            }
            throw "TGS backend stopped before it became ready.`n$details"
        }
        Start-Sleep -Seconds 1
    }
    if (-not $backendReady) {
        if ($startedBackend -and -not $tgsProcess.HasExited) {
            Stop-Process -Id $tgsProcess.Id -Force -ErrorAction SilentlyContinue
        }
        throw "TGS backend did not become ready within 60 seconds. Check '$backendStderr'."
    }
    Write-Host "TGS backend is ready at $apiUrl"
} else {
    Write-Host "Using the TGS backend already running at $apiUrl"
}

try {
    $env:TGS_API_URL = $apiUrl
    Push-Location $concatSource
    try {
        & cargo run -p concat
        $runExitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }
} finally {
    if ($null -eq $oldApiUrl) {
        Remove-Item Env:TGS_API_URL -ErrorAction SilentlyContinue
    } else {
        $env:TGS_API_URL = $oldApiUrl
    }
    if ($startedBackend -and $null -ne $tgsProcess -and -not $tgsProcess.HasExited) {
        Stop-Process -Id $tgsProcess.Id -Force -ErrorAction SilentlyContinue
    }
}

exit $runExitCode
