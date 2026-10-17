# Launches the staged Windows bundle the way a person would, and fails if
# it does not start. Nothing else in the build ever ran what it packaged,
# and 0.2.6 shipped installers whose program would not open: a DLL that
# was not in the folder, an old DirectML.dll found in System32 instead of
# a new one (#303), an instruction an Ivy Bridge does not have (#313).
#
#   scripts/ci/smoke-windows.ps1 -Bundle stage\Concat-<version>-windows-<arch> [-Sde path\to\sde.exe]
#
# 1. Every DLL any program or library in the folder imports is either in
#    the folder or one Windows itself provides. A DLL that only happens to
#    be on this machine - the Visual C++ runtime, DirectML, ONNX Runtime,
#    which Windows 11 carries a copy of its own - counts as missing.
# 2. `concat.exe --version` exits 0 with PATH cut down to Windows' own
#    folders, so every DLL comes from the folder or from System32 - as on
#    a machine with nothing else installed. It touches no GPU: it proves
#    the program and every DLL it loads at start load and initialise.
# 3. `concat.exe --smoke` exits 0 the same way: the start-up runs through
#    the first frame of the window, then the program leaves.
# 4. With -Sde, `--version` again under Intel's emulator posing as an Ivy
#    Bridge (AVX, nothing newer), which stops the program at the first
#    instruction that chip does not have; then `--whisper-cpu`, loading
#    whisper's CPU kernels, posing as a Sandy Bridge. x86_64 only.
#
# concat.exe is a GUI program: it has no console, so it is judged by its
# exit code and not by what it prints. Windows' error dialogs are turned
# off for the programs this starts, so a missing DLL is an exit code here
# rather than a dialog waiting for a click on a runner nobody is watching.
#Requires -Version 7
param(
  [Parameter(Mandatory = $true)][string]$Bundle,
  [string]$Sde = ""
)
$ErrorActionPreference = "Stop"

$bundle = (Resolve-Path $Bundle).Path
$exe = Join-Path $bundle "concat.exe"
if (-not (Test-Path $exe)) { throw "$exe is not there" }

function Say($text) { Write-Host ""; Write-Host "== $text" }
function Fail($text) {
  if ($env:GITHUB_ACTIONS) { Write-Host "::error::$text" }
  throw $text
}

# No error dialogs for this process or the ones it starts: a child takes
# its error mode from its parent. SEM_FAILCRITICALERRORS (the loader's
# "the code execution cannot proceed") and SEM_NOGPFAULTERRORBOX (the
# crash dialog).
Add-Type -Namespace Smoke -Name Native -MemberDefinition @"
[DllImport("kernel32.dll")] public static extern uint SetErrorMode(uint mode);
"@
[void][Smoke.Native]::SetErrorMode(0x0001 -bor 0x0002)

# What a status a program died with means, for the ones a broken bundle
# dies with.
$meanings = @{
  [int]0xC0000135 = "STATUS_DLL_NOT_FOUND: a DLL it imports is nowhere to be found"
  [int]0xC0000139 = "STATUS_ENTRYPOINT_NOT_FOUND: a DLL it found is not the one it was built against"
  [int]0xC000007B = "STATUS_INVALID_IMAGE_FORMAT: a DLL for another architecture, or a damaged one"
  [int]0xC000001D = "STATUS_ILLEGAL_INSTRUCTION: an instruction this CPU does not have"
  [int]0xC0000005 = "STATUS_ACCESS_VIOLATION: a crash"
  [int]0xC0000409 = "STATUS_STACK_BUFFER_OVERRUN: a fail-fast, which is how a Rust panic aborts"
  [int]0xC0E90002 = "a code integrity block (Smart App Control) on an unsigned or untrusted file"
}

# What Windows wrote down about a program that crashed since $since: the
# Application Error event (1000) names the faulting module, its version
# and the offset into it, which is the one thing an exit code cannot say.
# It covers a crash before main as well as after, where the app's own
# handler (concat-host's crash.rs) is not installed yet. Windows writes the
# event a moment after the process is gone, so it is waited for briefly.
function Show-Crash($file, $since) {
  $exe = [IO.Path]::GetFileName($file)
  for ($try = 0; $try -lt 10; $try++) {
    $event = Get-WinEvent -FilterHashtable @{ LogName = "Application"; Id = 1000; StartTime = $since } -ErrorAction SilentlyContinue |
      Where-Object { $_.Message -match [regex]::Escape($exe) } | Select-Object -First 1
    if ($event) {
      Say "what Windows recorded about the crash"
      $event.Message -split "`n" | ForEach-Object { Write-Host "  $($_.TrimEnd())" }
      return
    }
    Start-Sleep -Milliseconds 500
  }
  Write-Host "  (Windows recorded no Application Error event for $exe)"
}

# Starts a program with the given PATH and waits for it, up to a limit;
# fails unless it exits 0. Its output is shown when there is any.
function Expect-Ok($what, $file, [string[]]$arguments, $path, $seconds) {
  $info = New-Object System.Diagnostics.ProcessStartInfo
  $info.FileName = $file
  foreach ($a in $arguments) { [void]$info.ArgumentList.Add($a) }
  $info.UseShellExecute = $false
  $info.RedirectStandardOutput = $true
  $info.RedirectStandardError = $true
  # Not the bundle: the program has to find its DLLs beside itself, not
  # in the folder it was started from.
  $info.WorkingDirectory = [IO.Path]::GetTempPath()
  $info.Environment["PATH"] = $path
  $started = Get-Date
  $process = [System.Diagnostics.Process]::Start($info)
  $stdout = $process.StandardOutput.ReadToEndAsync()
  $stderr = $process.StandardError.ReadToEndAsync()
  if (-not $process.WaitForExit($seconds * 1000)) {
    $process.Kill($true)
    Fail "${what}: still running after $seconds s"
  }
  $process.WaitForExit()
  $out = ($stdout.Result + $stderr.Result).Trim()
  if ($out) { $out -split "`n" | Select-Object -Last 60 | ForEach-Object { Write-Host "  $_" } }
  $code = $process.ExitCode
  if ($code -ne 0) {
    Show-Crash $file $started
    $meaning = $meanings[[int]$code]
    Fail ("${what}: exited with 0x{0:X8}{1}" -f $code, $(if ($meaning) { " - $meaning" } else { "" }))
  }
  Write-Host "  exited 0"
}

# ── 1. Every import is in the folder or Windows' own ───────────────────
Say "every DLL the bundle imports is in it, or is Windows' own"
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$dumpbin = & $vswhere -latest -products * -find "VC\Tools\MSVC\**\bin\Host*\*\dumpbin.exe" | Select-Object -First 1
if (-not $dumpbin) { throw "dumpbin.exe is not in the Visual Studio installation" }

# DLLs a clean Windows does not have, or has in a version of its own that
# is not the one the bundle was built against. One of these resolved from
# System32 on the runner is a DLL missing on somebody's machine.
$mustShip = @(
  "vcruntime140.dll", "vcruntime140_1.dll", "msvcp140.dll", "msvcp140_1.dll",
  "msvcp140_2.dll", "msvcp140_atomic_wait.dll", "msvcp140_codecvt_ids.dll",
  "concrt140.dll", "vccorlib140.dll", "vcomp140.dll",
  "directml.dll", "onnxruntime.dll", "onnxruntime_providers_shared.dll"
)
$system = Join-Path $env:SystemRoot "System32"
$inBundle = @{}
Get-ChildItem $bundle -Recurse -File | ForEach-Object { $inBundle[$_.Name.ToLowerInvariant()] = $true }
$missing = @()
foreach ($pe in Get-ChildItem $bundle -Recurse -File -Include *.exe, *.dll) {
  $lines = & $dumpbin /nologo /dependents $pe.FullName
  if ($LASTEXITCODE -ne 0) { throw "dumpbin failed on $($pe.Name)" }
  # The names are the indented lines under "Image has the following
  # (delay load) dependencies:", up to the summary.
  $reading = $false
  foreach ($line in $lines) {
    if ($line -match "Image has the following .*dependencies") { $reading = $true; continue }
    if ($line -match "^\s*Summary") { $reading = $false; continue }
    if (-not $reading) { continue }
    $dll = $line.Trim()
    if (-not $dll -or $dll -notmatch "\.dll$") { continue }
    $name = $dll.ToLowerInvariant()
    if ($inBundle.ContainsKey($name)) { continue }
    # API sets: names the loader resolves to whatever implements them.
    if ($name -match "^(api|ext)-ms-") { continue }
    if ($mustShip -contains $name) { $missing += "$($pe.Name) -> $dll (must ship with the program)"; continue }
    if (-not (Test-Path (Join-Path $system $dll))) { $missing += "$($pe.Name) -> $dll (not in the folder or in System32)" }
  }
}
if ($missing.Count -gt 0) {
  $missing | ForEach-Object { Write-Host "  $_" }
  Fail "the bundle imports DLLs it does not carry"
}
Write-Host "  every import resolves to the folder or to Windows"

# Windows' own folders and nothing else.
$bare = @($system, $env:SystemRoot, (Join-Path $system "Wbem"), (Join-Path $system "WindowsPowerShell\v1.0")) -join ";"

# ── 2. --version ───────────────────────────────────────────────────────
Say "concat.exe --version, with only Windows on the path"
Expect-Ok "concat.exe --version" $exe @("--version") $bare 60

# ── 3. --smoke ─────────────────────────────────────────────────────────
Say "concat.exe --smoke, with only Windows on the path"
Expect-Ok "concat.exe --smoke" $exe @("--smoke") $bare 180

# ── 4. On an emulated Ivy Bridge ───────────────────────────────────────
if ($Sde) {
  Say "concat.exe --version on an emulated Ivy Bridge"
  $sdeExe = (Resolve-Path $Sde).Path
  # The emulator's own folder too: it loads its pieces from beside itself.
  $path = (Split-Path $sdeExe) + ";" + $bare
  Expect-Ok "concat.exe --version under SDE -ivb" $sdeExe @("-ivb", "--", $exe, "--version") $path 600

  # whisper's CPU kernels, one DLL per CPU generation: ggml loads every
  # one to score it against the CPU before it keeps the best, so the
  # Haswell and Ice Lake DLLs are loaded here too, on a Sandy Bridge - AVX
  # without even F16C, like #318's i7-2600 - which must come out with the
  # sandybridge DLL and no illegal instruction.
  Say "concat.exe --whisper-cpu on an emulated Sandy Bridge"
  Expect-Ok "concat.exe --whisper-cpu under SDE -snb" $sdeExe @("-snb", "--", $exe, "--whisper-cpu") $path 600
}

Say "ok"
