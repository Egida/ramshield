#!/usr/bin/env bash
set -uo pipefail

# ponytail: retain the latest 10 build records; use CI artifact storage for longer history.
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOG_DIR=${RAMSHIELD_BUILD_LOG_DIR:-/tmp/ramshield-builds}
RETENTION=${RAMSHIELD_BUILD_LOG_RETENTION:-10}
DESCRIPTION=${1:-"release build"}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
LOG="$LOG_DIR/${STAMP}_$$_build.log"
mkdir -p "$LOG_DIR"

{
  printf 'started_utc=%s\n' "$STAMP"
  printf 'description=%s\n' "$DESCRIPTION"
  printf 'git_sha=%s\n' "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || printf unknown)"
  printf 'command=cargo build --release --locked --features full\n\n'
} >"$LOG"

(
  cd "$ROOT" || exit 1
  cargo build --release --locked --features full
) 2>&1 | tee -a "$LOG"
RC=${PIPESTATUS[0]}
printf '\nresult=%s\nfinished_utc=%s\n' "$RC" "$(date -u +%Y%m%dT%H%M%SZ)" >>"$LOG"

if [[ "$RETENTION" =~ ^[0-9]+$ ]]; then
  mapfile -t OLD_LOGS < <(find "$LOG_DIR" -maxdepth 1 -type f -name '*_build.log' -printf '%T@ %p\n' | sort -nr | awk "NR > $RETENTION {sub(/^[^ ]+ /, \"\"); print}")
  ((${#OLD_LOGS[@]})) && rm -f -- "${OLD_LOGS[@]}"
fi

printf 'build_log=%s\nresult=%s\n' "$LOG" "$RC"
exit "$RC"
