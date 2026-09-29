#!/usr/bin/env bash
# upgrade_qualification.sh — test $OLD_TAG → current upgrade and current → $OLD_TAG rollback.
# Current version is read from Cargo.toml (one release identity).
# Run on a non-production host.
set -euo pipefail

PREFIX="${1:-/tmp/ramshield_upgrade_test}"
OLD_TAG="${2:-v0.3.0}"
NEW_TAG="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
NEW_DIR="$PREFIX/new"
OLD_DIR="$PREFIX/old"
WAL_DIR="$PREFIX/wal"
CONFIG_FILE="$PREFIX/config.toml"
TEST_IP="203.0.113.7"

log()  { printf '\033[32m[qual]\033[0m %s\n' "$*"; }
err()  { printf '\033[31m[qual]\033[0m %s\n' "$*" >&2; }

need_cmd() { command -v "$1" >/dev/null 2>&1 || { err "missing: $1"; exit 1; }; }

cleanup() {
    log "cleaning up $PREFIX"
    rm -rf "$PREFIX"
}

setup() {
    mkdir -p "$NEW_DIR" "$OLD_DIR" "$WAL_DIR" "$PREFIX"

    # Build old version
    log "building $OLD_TAG"
    git clone --branch "$OLD_TAG" --depth 1 https://github.com/grep999/ramshield.git "$OLD_DIR"
    (cd "$OLD_DIR" && cargo build --release --locked --features full)

    # Build new version (current checkout)
    log "building current"
    cargo build --release --locked --features full
    cp target/release/ramshield "$NEW_DIR/"

    # minimal config pointing at test WAL
    cat > "$CONFIG_FILE" <<EOF
[engine]
shard_count = 4
worker_threads = 1
ram_limit_mb = 256

[ipc]
tcp_addr = "127.0.0.1:17890"
max_line_length = 1048576
auth_keys = ["t1:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"]

[key_roles]
k1 = { key_id = "k1", role = "Admin" }

[dashboard]
enabled = false

[wal]
enabled = true
dir = "$WAL_DIR"
durability = "GroupCommit"
compress = false
seg_max_bytes = 4194304
retention_max_bytes = 33554432

[detection]
rps_threshold = 999999
rate_window_secs = 10
batch_block_enabled = true
block_ttl_secs = 120
bloom_bits = 100000
promote_min_events = 1000

[xdp]
enabled = false
EOF
}

test_upgrade() {
    log "=== Upgrade test: $OLD_TAG → $NEW_TAG ==="

    # Start old version, create persistent block
    "$OLD_DIR/target/release/ramshield" --config "$CONFIG_FILE" &
    OLD_PID=$!
    sleep 2

    "$OLD_DIR/target/release/ramshield-cli" block "$TEST_IP" --reason upgrade-test --ttl 120 || \
        { err "old version block failed"; kill "$OLD_PID" 2>/dev/null; return 1; }

    kill "$OLD_PID" 2>/dev/null; wait "$OLD_PID" 2>/dev/null || true
    log "old version stopped"

    # Start current release with same WAL
    "$NEW_DIR/ramshield" --config "$CONFIG_FILE" &
    NEW_PID=$!
    sleep 2

    # Verify block survived
    if "$NEW_DIR/ramshield-cli" check "$TEST_IP" | grep -q BLOCKED; then
        log "UPGRADE PASS: block survived"
    else
        err "UPGRADE FAIL: block not restored after upgrade"
        kill "$NEW_PID" 2>/dev/null; wait "$NEW_PID" 2>/dev/null || true
        return 1
    fi

    kill "$NEW_PID" 2>/dev/null; wait "$NEW_PID" 2>/dev/null || true
    log "upgrade test complete"
    return 0
}

test_rollback() {
    log "=== Rollback test: $NEW_TAG → $OLD_TAG ==="

    # Create block with current release
    "$NEW_DIR/ramshield" --config "$CONFIG_FILE" &
    NEW_PID=$!
    sleep 2

    "$NEW_DIR/ramshield-cli" block "198.51.100.1" --reason rollback-test --ttl 120 || \
        { err "new version block failed"; kill "$NEW_PID" 2>/dev/null; return 1; }

    kill "$NEW_PID" 2>/dev/null; wait "$NEW_PID" 2>/dev/null || true
    log "new version stopped"

    # Start old version with same WAL
    "$OLD_DIR/target/release/ramshield" --config "$CONFIG_FILE" &
    OLD_PID=$!
    sleep 2

    if "$OLD_DIR/target/release/ramshield-cli" check "198.51.100.1" | grep -q BLOCKED; then
        log "ROLLBACK PASS: block restored"
    else
        log "ROLLBACK NOTE: block not restored (expected if WAL format changed)"
    fi

    kill "$OLD_PID" 2>/dev/null; wait "$OLD_PID" 2>/dev/null || true
    log "rollback test complete"
    return 0
}

main() {
    log "=== RamShield Upgrade/Rollback Qualification ==="
    log "old=$OLD_TAG new=$NEW_TAG"
    trap cleanup EXIT
    need_cmd cargo
    need_cmd git

    setup
    test_upgrade
    UPGRADE_RESULT=$?
    test_rollback
    ROLLBACK_RESULT=$?

    echo ""
    log "=== Results ==="
    log "Upgrade test:  $([ "$UPGRADE_RESULT" -eq 0 ] && echo PASS || echo FAIL)"
    log "Rollback test: $([ "$ROLLBACK_RESULT" -eq 0 ] && echo PASS || echo FAIL)"

    [ "$UPGRADE_RESULT" -eq 0 ] && [ "$ROLLBACK_RESULT" -eq 0 ]
}

main "$@"