#!/usr/bin/env bash
# Run the local release-candidate boundary. Does not push, tag, or deploy.
set -Eeuo pipefail
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$ROOT"

branch=$(git branch --show-current)
case "$branch" in
  p1|master) ;;
  *) printf 'release candidate requires p1 or master; got %s\n' "$branch" >&2; exit 2 ;;
esac
[[ -z "$(git status --porcelain)" ]] || { echo 'working tree is not clean' >&2; exit 2; }

bash scripts/review_pipeline.sh
cargo build --release --locked --features full
test -x target/release/ramshield

if command -v setcap >/dev/null 2>&1; then
  setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield 2>/dev/null || \
    sudo -n setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' target/release/ramshield
fi
getcap target/release/ramshield

CFG=config.prod.toml.example \
IPC_PORT=17890 \
DASH_ADDR=127.0.0.1:19999 \
WAL_DIR="${WAL_DIR:-/tmp/ramshield-release-candidate-wal}" \
bash scripts/prod_smoke.sh

sha256sum target/release/ramshield Cargo.lock docs/metrics/metric-keystore.json docs/metrics/metric-keystore.jsonl
printf 'RELEASE CANDIDATE GREEN: %s %s\n' "$branch" "$(git rev-parse HEAD)"
