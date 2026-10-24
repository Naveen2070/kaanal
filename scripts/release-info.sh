#!/usr/bin/env bash
# Read RELEASE.md for the release workflow (.github/workflows/release.yml).
#
# RELEASE.md starts with one line, `Tag: vX.Y.Z` (an optional `-pre.1` suffix is allowed); everything after
# that line is the release notes. This script checks the tag against the version in Cargo.toml and prints
# `tag=`, `version=` and `prerelease=` lines (the form GitHub Actions wants in $GITHUB_OUTPUT). With
# `--notes FILE` it also writes the notes to FILE.
#
# Usage: scripts/release-info.sh [--notes FILE]
# Exit codes: 0 ok, 1 RELEASE.md is wrong or does not match Cargo.toml, 64 bad usage.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
notes_out=
while [ $# -gt 0 ]; do
  case "$1" in
    --notes) notes_out=${2:?--notes needs a file}; shift 2 ;;
    -h | --help) sed -n '2,11p' "$0"; exit 0 ;;
    *) echo "unknown option: $1 (see --help)" >&2; exit 64 ;;
  esac
done

fail() { echo "release-info: $*" >&2; exit 1; }
[ -f RELEASE.md ] || fail "RELEASE.md is missing"

# CRLF-safe: drop carriage returns before parsing.
tr -d '\r' < RELEASE.md > "${TMPDIR:-/tmp}/release-md.$$"
trap 'rm -f "${TMPDIR:-/tmp}/release-md.$$"' EXIT
file=${TMPDIR:-/tmp}/release-md.$$

first=$(head -n 1 "$file")
tag=$(printf '%s\n' "$first" | sed -n 's/^Tag:[[:space:]]*\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\(-[0-9A-Za-z.][0-9A-Za-z.]*\)\{0,1\}\)[[:space:]]*$/\1/p')
[ -n "$tag" ] || fail "the first line of RELEASE.md must be 'Tag: vX.Y.Z' (got: $first)"
version=${tag#v}

cargo_version=$(sed -n '/^\[workspace\.package\]/,/^\[/{s/^version[[:space:]]*=[[:space:]]*"\(.*\)"[[:space:]]*$/\1/p;}' Cargo.toml)
[ -n "$cargo_version" ] || fail "no [workspace.package] version in Cargo.toml"
[ "$version" = "$cargo_version" ] || fail "RELEASE.md says $tag but Cargo.toml says $cargo_version"

notes=$(tail -n +2 "$file" | sed '/./,$!d')
[ -n "$notes" ] || fail "RELEASE.md has no release notes after the Tag line"
if [ -n "$notes_out" ]; then
  printf '%s\n' "$notes" > "$notes_out"
fi

# Versions below 1.0 and any `-suffix` are marked as pre-releases.
prerelease=false
case "$version" in 0.* | *-*) prerelease=true ;; esac

echo "tag=$tag"
echo "version=$version"
echo "prerelease=$prerelease"
