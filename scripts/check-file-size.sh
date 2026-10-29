#!/usr/bin/env bash
# File-size gate (rule R3.8): fails when a tracked Rust or UI file is over its limit.
#   source files   (anything not under a tests/ folder): 800 lines
#   test files     (under a tests/ folder):              1000 lines
#   UI files       (ui/**/*.ts, ui/**/*.tsx):            600 lines
# Rust files between 600 and 800 lines, and UI files between 400 and 600, are listed as notices only (look for a seam).
# A file may exceed its limit only if it is in scripts/file-size-allow.txt with a reason.
# Limits can be overridden for a trial run: SRC_MAX=700 TEST_MAX=900 UI_MAX=500 scripts/check-file-size.sh
set -euo pipefail

src_max=${SRC_MAX:-800}
test_max=${TEST_MAX:-1000}
look=${LOOK:-600}
ui_max=${UI_MAX:-600}
ui_look=${UI_LOOK:-400}
allow_file="$(dirname "$0")/file-size-allow.txt"

cd "$(git rev-parse --show-toplevel)"
failed=0
# ui/ lists tracked source only: node_modules, dist and the built bundle are git-ignored.
while IFS= read -r f; do
  [ -f "$f" ] || continue
  lines=$(wc -l < "$f" | tr -d ' ')
  seam=$look
  case "$f" in
    ui/*) max=$ui_max; seam=$ui_look ;;
    */tests/*) max=$test_max ;;
    *) max=$src_max ;;
  esac
  if [ "$lines" -gt "$max" ]; then
    if grep -qE "^$f( |$)" "$allow_file" 2>/dev/null; then
      echo "allowed: $f has $lines lines (limit $max): $(grep -E "^$f( |$)" "$allow_file" | cut -d' ' -f2-)"
    else
      echo "::error file=$f::$f has $lines lines, over the limit of $max (R3.8): split it by responsibility, or add it to scripts/file-size-allow.txt with a reason"
      failed=1
    fi
  elif [ "$lines" -gt "$seam" ]; then
    echo "::notice file=$f::$f has $lines lines (over $seam): look for a seam"
  fi
done < <(git ls-files '*.rs' 'ui/*.ts' 'ui/*.tsx')

if [ "$failed" -ne 0 ]; then
  echo "file-size check failed"
  exit 1
fi
echo "file-size check passed (source <= $src_max, tests <= $test_max, ui <= $ui_max)"
