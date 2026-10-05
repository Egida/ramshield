#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
VERSION="$(sed -n 's/^version = "\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)"$/\1/p' "$ROOT/Cargo.toml" | head -1)"
[[ -n "$VERSION" ]] || { echo 'unable to resolve Cargo.toml version' >&2; exit 1; }
printf '%s\n' "$VERSION"
