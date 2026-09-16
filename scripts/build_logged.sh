#!/usr/bin/env bash
set -uo pipefail

# ponytail: JSONL index = build DB; switch to SQLite only if cross-build queries get complex.
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
LOG_DIR=${RAMSHIELD_BUILD_LOG_DIR:-/tmp/ramshield-builds}
RETENTION=${RAMSHIELD_BUILD_LOG_RETENTION:-10}
DESCRIPTION=${1:-"release build"}
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
LOG="$LOG_DIR/${STAMP}_$$_build.log"
DB="$LOG_DIR/index.jsonl"
mkdir -p "$LOG_DIR"

START_EPOCH=$(date +%s)
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
FINISH_EPOCH=$(date +%s)
DURATION=$((FINISH_EPOCH - START_EPOCH))
printf '\nresult=%s\nduration_secs=%s\nfinished_utc=%s\n' \
  "$RC" "$DURATION" "$(date -u -d @"$FINISH_EPOCH" +%Y%m%dT%H%M%SZ 2>/dev/null || date -u +%Y%m%dT%H%M%SZ)" >>"$LOG"

python3 - "$DB" "$STAMP" "$DESCRIPTION" "$RC" "$DURATION" "$LOG" <<'PY'
import json, os, sys
db, stamp, desc, rc, dur, log = sys.argv[1:]
path = os.path.basename(log)
rec = {
    "start_utc": stamp,
    "description": desc,
    "git_sha": open(log).read().split("git_sha=",1)[1].splitlines()[0],
    "command": "cargo build --release --locked --features full",
    "result": int(rc),
    "duration_secs": int(dur),
    "log": path,
}
recs = []
if os.path.exists(db):
    with open(db) as f:
        recs = [json.loads(l) for l in f if l.strip()]
recs.append(rec)
with open(db, "w") as f:
    for r in recs:
        f.write(json.dumps(r) + "\n")
PY

if [[ "$RETENTION" =~ ^[0-9]+$ ]]; then
  mapfile -t OLD_LOGS < <(find "$LOG_DIR" -maxdepth 1 -type f -name '*_build.log' -printf '%T@ %p\n' | sort -nr | awk "NR > $RETENTION {sub(/^[^ ]+ /, \"\"); print}")
  ((${#OLD_LOGS[@]})) && rm -f -- "${OLD_LOGS[@]}"
  python3 - "$DB" "$LOG_DIR" <<'PY'
import json, os, sys
db, logdir = sys.argv[1:]
with open(db) as f:
    recs = [json.loads(l) for l in f if l.strip()]
kept = [r for r in recs if os.path.exists(os.path.join(logdir, r["log"]))]
with open(db, "w") as f:
    for r in kept:
        f.write(json.dumps(r) + "\n")
PY
fi

printf 'build_db=%s\nbuild_log=%s\nresult=%s\n' "$DB" "$LOG" "$RC"
exit "$RC"