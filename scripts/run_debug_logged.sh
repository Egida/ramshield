#!/usr/bin/env bash
set -uo pipefail

# ponytail: debug runtime logs retained for the latest 10 runs; use external log storage for longer history.
# RUST_LOG=debug  → batch summaries, decision events, 4 Hz XDP counter audit (no per-event lines).
# RUST_LOG=trace  → opt-in per-event low-level channel (Store::insert per key).
# Override: RUST_LOG=trace scripts/run_debug_logged.sh ...
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOG_DIR=${RAMSHIELD_RUNTIME_LOG_DIR:-/tmp/ramshield-runtime}
RETENTION=${RAMSHIELD_RUNTIME_LOG_RETENTION:-10}
DESCRIPTION=${1:-"debug runtime"}
shift || true
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
LOG="$LOG_DIR/${STAMP}_$$_runtime.log"
mkdir -p "$LOG_DIR"

{
  printf 'started_utc=%s\n' "$STAMP"
  printf 'description=%s\n' "$DESCRIPTION"
  printf 'git_sha=%s\n' "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || printf unknown)"
  printf 'rust_log=%s\n' "${RUST_LOG:-debug}"
  printf 'command=target/release/ramshield --config config-xdp.toml %s\n\n' "$*"
} >"$LOG"

(
  cd "$ROOT" || exit 1
  RUST_LOG=${RUST_LOG:-debug} RUST_BACKTRACE=${RUST_BACKTRACE:-1} \
    ./target/release/ramshield --config config-xdp.toml "$@"
) 2>&1 | tee -a "$LOG"
RC=${PIPESTATUS[0]}
printf '\nresult=%s\nfinished_utc=%s\n' "$RC" "$(date -u +%Y%m%dT%H%M%SZ)" >>"$LOG"

if [[ "$RETENTION" =~ ^[0-9]+$ ]]; then
  mapfile -t OLD_LOGS < <(find "$LOG_DIR" -maxdepth 1 -type f -name '*_runtime.log' -printf '%T@ %p\n' | sort -nr | awk "NR > $RETENTION {sub(/^[^ ]+ /, \"\"); print}")
  ((${#OLD_LOGS[@]})) && rm -f -- "${OLD_LOGS[@]}"
fi

printf 'runtime_log=%s\nresult=%s\n' "$LOG" "$RC"
exit "$RC"
