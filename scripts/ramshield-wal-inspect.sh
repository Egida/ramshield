#!/usr/bin/env bash
set -euo pipefail

# Read-only WAL inspection. It intentionally avoids parsing payloads because
# payloads may contain operationally sensitive information. Use RamShield's
# own replay code for semantic validation.

DIR="${1:-}"
if [[ -z "$DIR" ]]; then
  echo "usage: $0 /path/to/wal" >&2
  exit 2
fi
if [[ ! -d "$DIR" ]]; then
  echo "WAL directory does not exist: $DIR" >&2
  exit 2
fi

printf 'WAL directory: %s\n' "$DIR"
printf 'Segments:\n'
find "$DIR" -maxdepth 1 -type f -printf '%f %s bytes\n' | sort
printf '\nPermissions:\n'
stat -c '%A %U:%G %n' "$DIR"/* 2>/dev/null || true
printf '\nNo WAL files were modified.\n'
