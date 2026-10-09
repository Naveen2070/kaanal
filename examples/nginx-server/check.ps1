# Phase One success criteria 3-5 against a running instance. Runs on Windows PowerShell 5.1 and pwsh 7.
# Start the emulator in another terminal (from the repo root):
#   cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml
# This script may be run from any directory: it resolves the repo root from its own
# path and runs every kaanal command there, because `.kaanal/run.json` and Cargo.toml
# are both relative to the directory `up` was started in (rule R2.6).
# Env: KAANAL_URL (default: port from .kaanal/run.json, else 8080), KAANAL (binary;
# default `cargo run -q -p kaanal --` in the repo root).
$ErrorActionPreference = 'Stop'
$here = Split-Path $PSCommandPath
$root = (Resolve-Path (Join-Path $here '..\..')).Path
$runJson = Join-Path $root '.kaanal\run.json'
$base = if ($env:KAANAL_URL) {
    $env:KAANAL_URL
} elseif (Test-Path $runJson) {
    "http://127.0.0.1:$((Get-Content $runJson -Raw | ConvertFrom-Json).port)"
} else {
    'http://127.0.0.1:8080'
}
function Kaanal {
    # PowerShell 5.1 turns a native command's stderr into a terminating error while
    # $ErrorActionPreference is Stop, which would mask the exit code we check below
    # (kaanal writes warnings to stderr). Relax it for the call, then restore.
    # Always run from $root, even when KAANAL is set: the CLI finds the instance via
    # .kaanal/run.json relative to its cwd, not via the binary's location.
    $prev = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    Push-Location $root
    try {
        if ($env:KAANAL) { & $env:KAANAL @args | Out-Null } else { cargo run -q -p kaanal -- @args | Out-Null }
        $code = $LASTEXITCODE
    } finally { Pop-Location; $ErrorActionPreference = $prev }
    if ($code) { throw "kaanal $args failed" }
}
function Code($u) {
    # `-SkipHttpErrorCheck` exists only in PowerShell 7; 5.1 throws on a non-2xx
    # instead. Reading the status off the exception gives one path for both.
    # `-UseBasicParsing` stops 5.1 from trying to start the IE engine.
    try {
        (Invoke-WebRequest $u -UseBasicParsing).StatusCode
    } catch {
        $r = $_.Exception.Response
        if ($null -eq $r) { throw }  # not an HTTP error: connection refused, DNS, ...
        [int]$r.StatusCode
    }
}

# Preconditions: an instance is reachable and no chaos is left over. Leftover chaos
# fails the 503 check below for the wrong reason, so start from a clean slate.
# The reachability probe is plain HTTP so it cannot be confused by CLI exit codes.
try { $health = Code "$base/__kaanal/health" } catch { $health = 0 }
if ($health -ne 200) {
    throw "no running instance at ${base}: start it with 'cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml' from the repo root"
}
Kaanal chaos clear

try {
    $first = (Invoke-WebRequest "$base/users/42" -UseBasicParsing).Content
    $second = (Invoke-WebRequest "$base/users/42" -UseBasicParsing).Content
    if ($first -ne $second) { throw 'FAIL: /users/42 not stable' }
    if (-not (Invoke-WebRequest "$base/users/42" -UseBasicParsing).Headers['x-kaanal-trace']) { throw 'FAIL: no trace header' }

    Kaanal chaos set db --error-rate 100% --error 503
    if ((Code "$base/users/42") -ne 503) { throw 'FAIL: expected 503 with chaos' }

    Kaanal chaos clear
    if ((Code "$base/users/42") -ne 200) { throw 'FAIL: expected 200 after clear' }
    'PASS'
} finally {
    # Never leave chaos behind, even when a check fails.
    try { Kaanal chaos clear } catch { }
}