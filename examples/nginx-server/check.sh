#!/usr/bin/env bash
# Phase One success criteria 3-5 against a running instance. Start Kaanal in
# another terminal (from the repo root):
#   cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml
# This script may be run from any directory: it resolves the repo root from its own
# path and runs every kaanal command there, because `.kaanal/run.json` and Cargo.toml
# are both relative to the directory `up` was started in (rule R2.6).
# Env: KAANAL_URL (default: port from .kaanal/run.json, else 8080), KAANAL (binary;
# default `cargo run -q -p kaanal --` in the repo root).
set -euo pipefail
here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root=$(cd -- "$here/../.." && pwd)

# KAANAL may hold a whole command ("cargo run -q -p kaanal --"), so it stays unquoted.
k() { (cd "$root" && ${KAANAL:-cargo run -q -p kaanal --} "$@"); }

if [ -z "${KAANAL_URL:-}" ]; then
  port=$(sed -n 's/.*"port": *\([0-9][0-9]*\).*/\1/p' "$root/.kaanal/run.json" 2>/dev/null || true)
  KAANAL_URL="http://127.0.0.1:${port:-8080}"
fi
B=$KAANAL_URL
# Never leave chaos behind, even when a check fails or the script is interrupted.
trap 'k chaos clear >/dev/null 2>&1 || true' EXIT

# Preconditions: an instance is reachable and no chaos is left over. Leftover chaos
# fails the 503 check below for the wrong reason, so start from a clean slate.
if ! curl -fsS -o /dev/null "$B/__kaanal/health" 2>/dev/null; then
  echo "no running instance at $B: start it with 'cargo run -p kaanal -- up -c examples/nginx-server/kaanal.yaml' from the repo root" >&2
  exit 1
fi
k chaos clear >/dev/null

a=$(curl -fsS "$B/users/42"); b=$(curl -fsS "$B/users/42")
[ "$a" = "$b" ] || { echo "FAIL: /users/42 not stable"; exit 1; }
headers=$(curl -fsS -D - -o /dev/null "$B/users/42")
grep -qi '^x-kaanal-trace:' <<<"$headers" || { echo "FAIL: no trace header"; exit 1; }

k chaos set db --error-rate 100% --error 503 >/dev/null
code=$(curl -s -o /dev/null -w '%{http_code}' "$B/users/42")
[ "$code" = 503 ] || { echo "FAIL: expected 503 with chaos, got $code"; exit 1; }

k chaos clear >/dev/null
code=$(curl -s -o /dev/null -w '%{http_code}' "$B/users/42")
[ "$code" = 200 ] || { echo "FAIL: expected 200 after clear, got $code"; exit 1; }
echo PASS