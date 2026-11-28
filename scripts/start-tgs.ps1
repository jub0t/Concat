param(
    [string]$TgsRoot = $env:TGS_ROOT
)

$ErrorActionPreference = "Stop"

# ==========================================
# TGS CONFIGURATION
# ==========================================

if ([string]::IsNullOrWhiteSpace($TgsRoot)) {
    $TgsRoot = "D:\Work\TGS"
}

$tgsBackend = Join-Path $TgsRoot "backend"
$tgsPython = Join-Path $tgsBackend ".venv\Scripts\python.exe"

$apiUrl = "http://127.0.0.1:8000"
$healthUrl = "$apiUrl/api/v1/health"

$startedBackend = $false
$tgsProcess = $null

# ==========================================
# HEALTH CHECK
# ==========================================

function Test-TgsBackend {
    try {
        $health = Invoke-RestMethod `
            -Uri $healthUrl `
            -TimeoutSec 2

        return $health.status -eq "ok"
    }
    catch {
        return $false
    }
}

# ==========================================
# VALIDATION
# ==========================================

if (-not (Test-Path -LiteralPath $tgsPython -PathType Leaf)) {
    throw "TGS Python environment not found: $tgsPython"
}

# ==========================================
# START BACKEND
# ==========================================

if (Test-TgsBackend) {
    Write-Host "TGS backend is already running at $apiUrl" -ForegroundColor Green
}
else {
    $logStamp = Get-Date -Format "yyyyMMdd-HHmmss"

    $backendStdout = Join-Path $env:TEMP "tgs-$logStamp.out.log"
    $backendStderr = Join-Path $env:TEMP "tgs-$logStamp.err.log"

    Write-Host "Starting TGS backend..." -ForegroundColor Cyan

    $tgsProcess = Start-Process `
        -FilePath $tgsPython `
        -ArgumentList @(
            "-m",
            "uvicorn",
            "app.main:app",
            "--host",
            "127.0.0.1",
            "--port",
            "8000"
        ) `
        -WorkingDirectory $tgsBackend `
        -WindowStyle Hidden `
        -RedirectStandardOutput $backendStdout `
        -RedirectStandardError $backendStderr `
        -PassThru

    $startedBackend = $true

    # Wait for backend
    $backendReady = $false

    for ($attempt = 0; $attempt -lt 60; $attempt++) {

        if (Test-TgsBackend) {
            $backendReady = $true
            break
        }

        if ($tgsProcess.HasExited) {
            $details = if (Test-Path $backendStderr) {
                Get-Content $backendStderr -Tail 30 | Out-String
            }
            else {
                "No backend log was created."
            }

            throw "TGS backend exited unexpectedly.`n$details"
        }

        Start-Sleep -Seconds 1
    }

    if (-not $backendReady) {
        if (-not $tgsProcess.HasExited) {
            Stop-Process -Id $tgsProcess.Id -Force
        }

        throw "TGS backend startup timed out after 60 seconds. Check: $backendStderr"
    }

    Write-Host "TGS backend started successfully." -ForegroundColor Green
}

# ==========================================
# OUTPUT
# ==========================================

Write-Host ""
Write-Host "TGS Backend Status : RUNNING" -ForegroundColor Green
Write-Host "API URL            : $apiUrl"
Write-Host "Health URL         : $healthUrl"

if ($startedBackend) {
    Write-Host "Process ID         : $($tgsProcess.Id)"
    Write-Host "Backend ownership  : Started by this script"
}
else {
    Write-Host "Backend ownership  : Existing process"
}