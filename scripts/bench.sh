#!/usr/bin/env bash
# Run the workspace benchmark suite (crates/kaanal/tests/bench.rs) in release mode.
#
# Usage: scripts/bench.sh [options]
#   --runs N            repeat the whole suite N times and merge (median of the medians); default 1.
#                       Each run is itself 5 repetitions per row. More runs means less noise.
#   --compare FILE      add a "% change" column against a saved run (default: target/bench/baseline.json
#                       when it exists)
#   --save-baseline     keep this result as target/bench/baseline.json for later comparisons
#   --out FILE          also copy the (merged) result JSON to FILE
#   -h, --help          this text
#
# Every run is kept in target/bench/ (ignored by git). Typical local use:
#   scripts/bench.sh --runs 3 --save-baseline     # before a change
#   scripts/bench.sh --runs 3                     # after it: compares with the baseline
#
# Numbers from a busy machine swing by 2x between runs, so close the heavy programs first and
# compare medians of several runs. Needs cargo; `--runs` above 1 and the comparison table need
# node (any recent version). The only hard check in the suite is NFR-05 (1M simulated requests
# under 10 s); a regression never fails the script.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
root=$PWD
dir=$root/target/bench
runs=1
compare=
save=0
out=

while [ $# -gt 0 ]; do
  case "$1" in
    --runs) runs=${2:?--runs needs a number}; shift 2 ;;
    --compare) compare=${2:?--compare needs a file}; shift 2 ;;
    --save-baseline) save=1; shift ;;
    --out) out=${2:?--out needs a file}; shift 2 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 64 ;;
  esac
done

case "$runs" in
  '' | *[!0-9]* | 0) echo "--runs must be a positive number" >&2; exit 64 ;;
esac

mkdir -p "$dir"
if [ -z "$compare" ] && [ -f "$dir/baseline.json" ]; then
  compare=$dir/baseline.json
fi
if [ -n "$compare" ]; then
  # The test runs from the crate folder, so hand it an absolute path.
  compare=$(cd "$(dirname "$compare")" && pwd)/$(basename "$compare")
  [ -f "$compare" ] || { echo "no such baseline: $compare" >&2; exit 66; }
  export KAANAL_BENCH_COMPARE=$compare
fi

stamp=$(date +%Y%m%d-%H%M%S)
files=()
for i in $(seq 1 "$runs"); do
  f=$dir/run-$stamp-$i.json
  echo "== bench run $i of $runs =="
  KAANAL_BENCH_OUT=$f cargo test -p kaanal --release --test bench -- --ignored --nocapture
  files+=("$f")
done

result=${files[0]}
if [ "$runs" -gt 1 ]; then
  result=$dir/merged-$stamp.json
  node "$root/scripts/bench-report.mjs" merge "${files[@]}" > "$result"
fi

if [ -n "${KAANAL_BENCH_COMPARE:-}" ] && command -v node > /dev/null; then
  echo
  echo "== $runs run(s) against $(basename "$KAANAL_BENCH_COMPARE") =="
  node "$root/scripts/bench-report.mjs" compare "$KAANAL_BENCH_COMPARE" "$result"
fi
[ "$save" -eq 1 ] && cp "$result" "$dir/baseline.json" && echo "saved baseline: $dir/baseline.json"
[ -n "$out" ] && cp "$result" "$out" && echo "wrote $out"
echo "result: $result"
exit 0
