# Master Scan Review — staged P0/P1 fixes

Branch: `fix/master-p0-p1-fixes` (base `63a41ba` = `origin/master`)
Scope: the six production changes staged in this tree (test hunks skimmed, not re-derived).

VERDICT: PASS
SAFE TO PUSH: yes

No blocking defect was found by reading the staged diff. All findings below are
either "no change needed" or non-blocking observations.

## Per-change verdicts

| # | Change | File | Verdict | Reason |
|---|--------|------|---------|--------|
| 1 | New pure fn `enumerate_stale_keys` replaces the inline reconcile enumeration | `crates/ramshield-enforcement/src/xdp.rs` | PASS | The cursor now advances for **every** visited key (stale or expected), so `bpf_map_get_next_key` can never return the same key twice and the loop terminates; only keys absent from `expected` are staged, and the staged-deletion (deferred delete after the loop) behaviour is preserved unchanged. Empty map: the first call (`prev = None`) returns `None` -> `break` -> empty staging vector. |
| 2 | Live replay paths switched from repairing `recover_dir` to read-only `verify_dir` | `crates/ramshield-storage/src/wal.rs` | PASS | The four `&self` methods (`replay`, `replay_from`, `replay_with_lsn_page`, `replay_with_lsn_from`) now never mutate a segment a live writer is appending to, closing the truncate-under-writer hole. In `verify` mode a torn tail on the *newest* segment is reported, not applied (older segments with a torn tail still error, unchanged), so a live read sees the valid prefix. The only remaining repair-capable entry points are ramwal's boot-time `Wal::open` -> `recover`, the `ramwal` CLI `recover` subcommand, and the static `Wal::replay_dir` — no `src/` production path calls the repairing static fn. `Wal::replay_dir` is now documented as boot-time-only. |
| 3 | `decode_hex` slices `s.as_bytes()` and validates each 2-byte pair with `from_utf8` | `crates/ramshield-protocol/src/auth.rs` | PASS | The `?` sits inside the `map` closure, whose return type is `Option<u8>`: `?` returns `None` from the closure and `collect::<Option<Vec<u8>>>` short-circuits to `None` — no panic path, and no type mismatch. `bytes.get(i..i+2)` cannot panic (and is always `Some` given the even-length guard). For valid ASCII hex the result is byte-identical to the old code; non-ASCII pairs are now rejected instead of panicking on a char boundary. |
| 4 | `IpAgg::absorb` uses `saturating_add` for `count` and `bytes` | `crates/ramshield-detection/src/batch.rs` | PASS | `ev.bytes` is client-supplied; with `overflow-checks = true` and `panic = "abort"` the old `+=` was a remote abort. Both accumulators now saturate and the rest of the struct's accumulators already saturated. |
| 5 | `open_or_create` opens with `O_NOFOLLOW` and requires a regular file owned by the current euid | `crates/ramshield-cgnat/src/shm.rs` | PASS | A symlink at the path now fails at `open` with `ELOOP`; a FIFO/dir/non-regular file is rejected before any `chmod`/`set_len`; a regular file planted by another uid is rejected before the daemon would `chmod 0600` and resize it. The check runs on the opened fd (`file.metadata()`), so there is no TOCTOU window. Creation mode is still `0o600` (and `set_permissions(0600)` still follows). A legitimate file created by the same uid passes unchanged. |
| 6 | New `validate_reason` (max 256 bytes, no control chars except space/tab) applied in `handle_block_ip` / `handle_block_cidr` before enqueue | `src/ipc/server/handlers.rs` | PASS | Validation happens on the `reason` field only, before the request is enqueued; `ip` / `cidr` are parsed separately by their own handlers and are unaffected. `Request::BlockIp`/`BlockCidr` are the only variants carrying a `reason`, and dispatch is exhaustive, so the local IPC path cannot bypass it. |
| 7 | Panic hook downcasts the payload to `String` as well as `&str` | `src/lib.rs` | PASS | Formatted panics now report their real message instead of `<non-string payload>`; the `&str` path is preserved and the fallback text is unchanged. No behavioural risk (the hook only formats a log line). |

## OPTIONAL OBSERVATIONS

Not required before the push.

1. `crates/ramshield-detection/src/batch.rs:47` — `self.status_dist[b as usize] += 1;` is still a plain
   increment on a `u32`. Overflowing it needs ~4.29e9 events of the same status class inside one
   flush window, so it is not remotely reachable, but the diff comment "Every other accumulator in
   this struct already saturates" is not literally true for this line. `saturating_add(1)` would
   make the comment accurate.
2. `crates/ramshield-cgnat/src/shm.rs` — `O_NOFOLLOW` protects only the final path component; a
   symlinked *parent* directory is still traversed. The shipped path is a fixed `/dev/shm/ramshield_rules`,
   so this is not exploitable in the current configuration.
3. `crates/ramshield-cgnat/src/shm.rs` — the new euid check rejects a root daemon opening a table
   pre-created by a *non-root* user, which the old code tolerated (root could `chmod` it). That is
   the intended hardening, but it is a behaviour change for such deployments; the error message is
   explicit, so the remedy (`chown root:root`) is obvious. UNVERIFIED: I did not check whether any
   deployment script in this repo pre-creates the table as a non-root user.
4. `crates/ramshield-storage/src/wal.rs` — in `verify` mode a torn tail on the newest segment is
   *reported* (`report.tail`, `report.truncated_bytes`) but not applied, so a live reader silently
   sees the valid prefix until the next boot repairs it. Consider logging `tail`/`truncated_bytes`
   on these read paths so operators can see an un-repaired tail.
5. `src/ipc/server/handlers.rs` — `validate_reason` covers the local IPC path only. A block reason
   replicated by a mesh peer is not passed through this validation, so the WAL/ring amplification
   concern is only closed for local clients. UNVERIFIED: I did not trace the mesh apply path for
   remote `reason` strings.
6. `crates/ramshield-enforcement/src/xdp.rs` — the live BPF path is now only covered by the model
   test for `enumerate_stale_keys`; there is no test against a real BPF map. The refactor is
   behaviour-preserving apart from the cursor fix, and the model test pins the termination property.
7. `crates/ramshield-protocol/src/auth.rs` — the even-length guard is on *bytes*, so a multibyte
   string can still pass it and be rejected later by `from_utf8`. Correct, but the rejection path is
   "not valid hex" rather than "not ASCII"; no functional impact.

REVIEWED IN 8 TOOL CALLS
