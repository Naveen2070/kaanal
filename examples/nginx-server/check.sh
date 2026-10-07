#!/usr/bin/env bash
# Phase One success criteria 3-5 against a running instance. From the repo root, first run in another terminal:
#   cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml
# Env: KAANAL_URL (default: port from .kaanal/run.json, else 8080), KAANAL (binary; default `cargo run -q -p kaanal --`).
set -euo pipefail
K=${KAANAL:-"cargo run -q -p kaanal --"}
if [ -z "${KAANAL_URL:-}" ]; then
  port=$(sed -n 's/.*"port": *\([0-9][0-9]*\).*/\1/p' .kaanal/run.json 2>/dev/null || true)
  KAANAL_URL="http://127.0.0.1:${port:-8080}"
fi
B=$KAANAL_URL
# Never leave chaos behind, even when a check fails.
trap '$K chaos clear >/dev/null 2>&1 || true' EXIT

a=$(curl -fsS "$B/users/42"); b=$(curl -fsS "$B/users/42")
[ "$a" = "$b" ] || { echo "FAIL: /users/42 not stable"; exit 1; }
headers=$(curl -fsS -D - -o /dev/null "$B/users/42")
grep -qi '^x-kaanal-trace:' <<<"$headers" || { echo "FAIL: no trace header"; exit 1; }

$K chaos set db --error-rate 100% --error 503 >/dev/null
code=$(curl -s -o /dev/null -w '%{http_code}' "$B/users/42")
[ "$code" = 503 ] || { echo "FAIL: expected 503 with chaos, got $code"; exit 1; }

$K chaos clear >/dev/null
code=$(curl -s -o /dev/null -w '%{http_code}' "$B/users/42")
[ "$code" = 200 ] || { echo "FAIL: expected 200 after clear, got $code"; exit 1; }
echo PASS
