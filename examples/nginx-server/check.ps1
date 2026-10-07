# Phase One success criteria 3-5 against a running instance. Needs PowerShell 7 (pwsh).
# From the repo root, first run in another terminal:
#   cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml
# Env: KAANAL_URL (default: port from .kaanal/run.json, else 8080), KAANAL (binary; default `cargo run -q -p kaanal --`).
$ErrorActionPreference = 'Stop'
if ($env:KAANAL_URL) {
    $base = $env:KAANAL_URL
} else {
    $port = 8080
    if (Test-Path .kaanal/run.json) { $port = (Get-Content .kaanal/run.json -Raw | ConvertFrom-Json).port }
    $base = "http://127.0.0.1:$port"
}
function Kaanal {
    if ($env:KAANAL) { & $env:KAANAL @args | Out-Null } else { cargo run -q -p kaanal -- @args | Out-Null }
    if ($LASTEXITCODE) { throw "kaanal $args failed" }
}
function Code($u) { (Invoke-WebRequest $u -SkipHttpErrorCheck).StatusCode }

try {
    $first = (Invoke-WebRequest "$base/users/42").Content
    $second = (Invoke-WebRequest "$base/users/42").Content
    if ($first -ne $second) { throw 'FAIL: /users/42 not stable' }
    if (-not (Invoke-WebRequest "$base/users/42").Headers['x-kaanal-trace']) { throw 'FAIL: no trace header' }

    Kaanal chaos set db --error-rate 100% --error 503
    if ((Code "$base/users/42") -ne 503) { throw 'FAIL: expected 503 with chaos' }

    Kaanal chaos clear
    if ((Code "$base/users/42") -ne 200) { throw 'FAIL: expected 200 after clear' }
    'PASS'
} finally {
    # Never leave chaos behind, even when a check fails.
    try { Kaanal chaos clear } catch { }
}
