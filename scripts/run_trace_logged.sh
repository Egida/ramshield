#!/usr/bin/env bash
set -uo pipefail

# ponytail: debug runtime logs retained for the latest 10 runs; use external log storage for longer history.
# RUST_LOG=trace  → per-event low-level channel (Store::insert per key, XDP calls).
# VOLUME: ~300 bytes/event; keep trace windows bounded (100k events ≈ 30 MB).
# Override: RUST_LOG=debug scripts/run_trace_logged.sh ...
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOG_DIR=${RAMSHIELD_RUNTIME_LOG_DIR:-/tmp/ramshield-runtime}
RETENTION=${RAMSHIELD_RUNTIME_LOG_RETENTION:-10}
DESCRIPTION=${1:-"trace runtime"}
if [[ "$DESCRIPTION" == -* ]]; then DESCRIPTION="trace runtime"; else shift || true; fi
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
LOG="$LOG_DIR/${STAMP}_$$_trace.log"
mkdir -p "$LOG_DIR"

# File capabilities live on the inode: every cargo build drops them, and the
# kernel then reports the loss as an opaque BPF map creation failure. Say it
# here instead of letting the XDP arm fail at runtime.
if command -v getcap >/dev/null && [[ -z "$(getcap "$ROOT/target/release/ramshield" 2>/dev/null)" ]]; then
  printf 'warn: no file caps on target/release/ramshield — XDP will fall back to in-band.\n'
  printf "      fix: sudo setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield\n"
fi

{
  printf 'started_utc=%s\n' "$STAMP"
  printf 'description=%s\n' "$DESCRIPTION"
  printf 'git_sha=%s\n' "$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || printf unknown)"
  printf 'rust_log=%s\n' "${RUST_LOG:-trace}"
  printf 'command=target/release/ramshield --config config-xdp.toml %s\n\n' "$*"
} >"$LOG"

(
  cd "$ROOT" || exit 1
  RUST_LOG=${RUST_LOG:-trace} RUST_BACKTRACE=${RUST_BACKTRACE:-1} \
    ./target/release/ramshield --config config-xdp.toml "$@"
) 2>&1 | stdbuf -oL tee -a "$LOG"
RC=${PIPESTATUS[0]}
printf '\nresult=%s\nfinished_utc=%s\n' "$RC" "$(date -u +%Y%m%dT%H%M%SZ)" >>"$LOG"

if [[ "$RETENTION" =~ ^[0-9]+$ ]]; then
  mapfile -t OLD_LOGS < <(find "$LOG_DIR" -maxdepth 1 -type f \( -name '*_runtime.log' -o -name '*_trace.log' \) -printf '%T@ %p\n' | sort -nr | awk "NR > $RETENTION {sub(/^[^ ]+ /, \"\"); print}")
  ((${#OLD_LOGS[@]})) && rm -f -- "${OLD_LOGS[@]}"
fi

printf 'runtime_log=%s\nresult=%s\n' "$LOG" "$RC"
exit "$RC"
