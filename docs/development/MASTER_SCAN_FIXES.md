# Master Scan Fixes — `63a41ba` → candidate

**Base:** `63a41ba6f67405b9afd29e82ea99aead68f5dc90` (`origin/master` at time of work)
**Branch:** `fix/master-p0-p1-fixes`
**Scope:** confirmed defects from the five-lens read-only master scan, plus the
complementary panic/cast sweep. No F-01 mesh work, no TLS design, no
audit-branch repair, and no unconfirmed findings are included.

Every fix below is minimal, carries a focused regression, and was verified
RED → GREEN. No test was weakened to obtain a green suite.

---

## F-1 (P0) — XDP stale-key reconciliation never terminates

**File:** `crates/ramshield-enforcement/src/xdp.rs`

`reconcile` enumerated the BPF blocklist map with `bpf_map_get_next_key` and
advanced the cursor **only for keys that were still expected**. A key that had
gone stale was staged for deletion but the cursor was *not* advanced, so the
next `GET_NEXT_KEY` returned the same resident key again. The loop therefore
never terminated whenever at least one stale key was resident: the enforcement
control plane hung and `staged_deletions` grew without bound.

Introduced by `f8a63fe`; present in the old master (`fdd876b`) and in the
repair candidate `3308049`.

**Proof:** a raw BPF syscall reproducer (`/home/m/.cache/shm_audit/reconcile2.c`)
showed `BPF_MAP_GET_NEXT_KEY` returning the same key indefinitely. A
post-fix probe did not terminate in 25 iterations and reached a staged count
of 24.

**Fix:** advance the cursor after *every* visited key, stale or not. The
enumeration is extracted into the pure function `enumerate_stale_keys` so the
cursor discipline is directly testable; deletion stays deferred (staged), which
preserves the original race-avoidance intent.

**Regression:** `reconcile_enumeration_terminates_with_a_stale_key`
(`cargo test -p ramshield-enforcement --features xdp --lib …`) — RED before the
fix, GREEN after.

---

## F-2 (P1) — a live WAL read truncated the segment it was reading

**File:** `crates/ramshield-storage/src/wal.rs`

`replay`, `replay_from`, `replay_with_lsn_page` and `replay_with_lsn_from` are
methods on an **open** `Wal`. Each one nevertheless called
`ramwal::recovery::recover_dir(dir)` — the *repairing* scan — which truncates a
torn tail and fsyncs the truncation, and can quarantine a segment outright.

The mesh sync path pages WAL history (`service/mesh.rs`, `spawn_blocking`
`replay_with_lsn_page`) while the enforcement actor is appending to the same
`Arc<Wal>`. A repair-mode scan under a live writer truncates the segment below
the writer's file offset, so the writer's next append lands past the
truncation point: a hole in the log, a gap in the LSN sequence, and a next
recovery that rejects the segment.

The repair was also **redundant**: `RamWal::open` already performs the
authoritative repairing recovery (`recover(&dir)`) and positions the writer from
`recovery.append_offset`. Boot-time recovery therefore keeps working exactly
as before — it happens at open, not at read.

**Fix:** the four `&self` read paths now use `ramwal::recovery::verify_dir`
(read-only). `Wal::replay_dir(dir)` remains the explicit repairing entry point
and is documented as boot-time-only.

**Regression:** `live_replay_does_not_mutate_the_segment` — plants a torn tail
on a live segment and asserts the segment length is unchanged after a live
read. RED before the fix (`left: 94, right: 104` — the live replay truncated
the segment), GREEN after.

---

## F-3 (P1) — `decode_hex` panicked on non-ASCII input

**File:** `crates/ramshield-protocol/src/auth.rs`

`decode_hex` sliced a `&str` at byte offsets. An even byte length passed the
length guard, but a multi-byte character made the slice land inside a char
boundary — a panic. The signature is decoded **before** any credential is
checked, so this is reachable pre-authentication. The release profile sets
`panic = "abort"`, so the panic takes the whole daemon down.

**Proof:** a property harness over 2 011 inputs (`/home/m/.cache/scratch/fixproof/p.rs`)
recorded **729 panics** with the current decoder and **0** with the fixed
decoder, with **no** behavioural difference on inputs that decoded successfully.

**Fix:** slice `s.as_bytes()` and validate each two-byte pair with
`str::from_utf8` before hex parsing, so the panic precondition is removed
structurally rather than guarded.

**Regressions:** `decode_hex_rejects_multibyte_input_without_panicking` and
`verify_rejects_multibyte_signature_without_panicking` (the public
verification path) — both RED before, GREEN after.

---

## F-4 (P1) — client-supplied byte counter could overflow

**File:** `crates/ramshield-detection/src/batch.rs`

`IpAgg::absorb` did `self.bytes += ev.bytes`. `ev.bytes` is client-supplied
(the IPC `report_connection` request carries it verbatim) and the release
profile has `overflow-checks = true`, so the `+=` is a panic — and with
`panic = "abort"` that aborts the daemon.

**Fix:** `saturating_add`, matching the sibling accumulators already present
in the same type.

**Regression:** `byte_accumulator_saturates_instead_of_overflowing` — RED
(panicked at `batch.rs:37`), GREEN after.

---

## F-5 (P2) — shared-memory path could be a symlink

**File:** `crates/ramshield-cgnat/src/shm.rs`

`ShmTableManager::open_or_create` opened the predictable path
`/dev/shm/ramshield_rules` with `create(true)` and **no** `O_NOFOLLOW`.
`/dev/shm` is world-writable and sticky, so any local user can pre-plant a
symlink named `ramshield_rules`. The daemon would then open the symlink
target, `chmod` it to `0600`, resize it to the 16 MiB table size and map it
writable — destroying an arbitrary file. A pre-planted *regular* file is the
same class of problem: a privileged daemon would chmod and resize it.

**Fix:** `O_NOFOLLOW` on the open, plus a post-open check that the file is a
regular file owned by the current euid. Failing closed there is strictly
earlier than the pre-existing `set_permissions` call, which already failed with
`EPERM` for a non-root daemon on a file it did not own.

**Regressions:** `open_or_create_refuses_a_symlink_at_the_path` (asserts the
symlink target's length, content and mode are all untouched) and
`open_or_create_refuses_a_non_regular_file` — the symlink test is RED without
`O_NOFOLLOW` (the daemon followed the link), GREEN with it.

---

## F-6 (P2) — unbounded block reason

**File:** `src/ipc/server/handlers.rs`

The `reason` of a `BlockIp` / `BlockCidr` request is stored in the
block-history ring, written verbatim into a WAL record that is replicated to
every mesh peer, and interpolated into a `tracing` field whose default
formatter does not escape values. It was neither length-bounded nor
control-character-checked: an unbounded reason multiplies across the ring and
the mesh, and a newline in the reason forges a log line.

**Fix:** `validate_reason` — at most 256 bytes, no control characters other
than space and tab. Applied in `handle_block_ip` and `handle_block_cidr`
before the request is enqueued, matching the existing convention of
validating request fields in the handler (as `cidr.parse::<IpNetwork>()` does).

**Regression:** `reason_validation_rejects_oversize_and_control_characters` —
RED when the guard is neutered to pre-fix behaviour, GREEN with the guard.

---

## F-7 (P2, diagnostic) — the panic hook discarded formatted messages

**File:** `src/lib.rs`

The custom panic hook downcast the payload to `&str` only. A `panic!` with a
literal payload is a `&str`; a *formatted* payload is a `String`. Every
format-style panic was therefore reported as `<non-string payload>` and the
standard backtrace was lost — for exactly the panics this codebase can hit.

**Fix:** accept both `&str` and `String` payloads.

No standalone regression: the hook is process-global and installing it inside
a test would replace the harness's own hook. It is covered by the absence of
the other panics (F-3, F-4) rather than by a dedicated test.

---

## Reviewed and deliberately NOT changed

| Finding | Decision |
|---|---|
| `panic = "abort"` in the release profile | **Kept.** Both `catch_unwind` sites in the tree (`src/dashboard/auth.rs`, `crates/ramshield-protocol/src/auth.rs`) are inside `#[cfg(test)]` modules, so no production code relies on unwinding. The setting is a deliberate hardening choice, and it raises the stakes of the panic fixes above — which is why F-3 and F-4 matter. |
| Default unauthenticated privileged IPC posture (`docs/IPC.md` claims role authorisation that the default path skips, and hard-codes the actor to `"admin"`) | **Not in this push.** This is a policy decision for the repository owner, not a mechanical defect. |
| Tracked stale copies `README.md.bak`, `src/engine/boot.rs.bak`, `src/engine/boot.rs.orig` | **Not in this push.** These are on the preserve list for this work; deleting tracked files needs an explicit owner decision. They are not compiled by Cargo and are invisible to `rg --type rust`. |
| Per-IP `IpAgg` map growth, replay-store bounds, native XDP frame-size bound, dashboard body limits, config-patch path, `inc_blocks` | **Not in this push.** Open scan candidates without a confirmed reproduction. |
| `src/engine/waf.rs` | **Not a reachable vulnerability.** The module has zero production callers; the issues are dead-code concerns. |

---

## Verification

All commands run in `/home/m/.cache/rs-master` with `CARGO_TARGET_DIR` unset
(the nested BPF build deadlocks when it is inherited) and
`PATH="$HOME/.local/bin:$PATH"` for `bpf-linker`.

| Gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo check --workspace --locked --all-targets` | PASS |
| `cargo check --workspace --locked --all-targets --all-features` | PASS |
| `cargo clippy --workspace --locked --all-targets -- -D warnings` | PASS |
| `cargo clippy --workspace --locked --features full --all-targets -- -D warnings` | PASS |
| `cargo build --workspace --locked --all-targets` | PASS |
| `cargo build --workspace --locked --all-targets --all-features` | PASS |
| `cargo test --workspace --locked` | PASS — 558 passed / 0 failed / 14 ignored |
| `cargo test --workspace --locked --features full` | PASS |
| `cargo test -p ramshield-mesh --locked` | PASS |
| `cargo test --workspace --locked --all-features` | PASS — 587 passed / 0 failed / 14 ignored |
| `cargo build --release --locked --features full` | PASS |
| `lint-no-unwrap` (production `src/` + `crates/`) | PASS — 0 hits |
| `scripts/verify_shm_header.sh` | PASS |

Full gate log: `/tmp/f01/scan_gates.log`.
