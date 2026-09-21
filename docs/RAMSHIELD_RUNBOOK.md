# RamShield Operations & Recovery Runbook

> This document is intentionally colocated with the implementation. The code
> comments in `ramshield-enforcement` and `ramshield-config` describe the
> invariants; this runbook describes how an operator verifies those invariants
> in production and what to do when they are violated.

## 1. Scope

This runbook covers the failure modes identified by the 2026-09-20 ground-up
source audit:

1. XDP per-IP map drift.
2. XDP CIDR/LPM map drift.
3. WAL -> storage ordering and recovery semantics.
4. Invalid environment overrides.
5. CI/toolchain and security-audit checks.
6. GitHub automation trust boundaries.

The enforcement actor remains the sole writer of block state and dataplane
state. Do not edit BPF maps manually during an active incident unless the
incident procedure explicitly requires it.

## 2. Normal verification

From the repository root:

```bash
./scripts/ramshield-preflight.sh
```

The preflight script is deliberately read-only. It checks for the expected
source layout, required tools, workspace metadata, and the presence of the
security/recovery tests.

When Rust/Cargo is installed, run:

```bash
cargo test --workspace --all-targets --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```

Cargo's `test` command compiles and runs unit, integration, and documentation
tests, while `--workspace` selects the workspace packages. See the Cargo
reference for the exact behavior. citeturn0search0turn0search2

## 3. Environment configuration failure

### Symptom

Startup fails with an error containing the environment variable name, for
example:

```text
invalid value for RAMSHIELD_ENGINE__RAM_LIMIT_MB: invalid digit found in string
```

### Meaning

This is intentional. Typed environment overrides are configuration input, not
best-effort hints. An invalid value must prevent startup rather than silently
leaving the TOML/default value active.

### Recovery

1. Inspect the service environment:

```bash
systemctl show ramshield --property=Environment
```

2. Correct the variable.
3. Run the preflight script.
4. Restart RamShield.

Never print password, HMAC, or Argon2 secret values into incident logs.

## 4. XDP map drift

### Symptom

Dashboard/metrics indicate active blocks but kernel enforcement counters do
not reflect the expected traffic, or a kernel/BPF reload has occurred.

### First response

Run:

```bash
./scripts/ramshield-xdp-inspect.sh
```

The script is diagnostic only. It does not mutate maps.

### Recovery

The enforcement actor performs periodic store -> XDP reconciliation. Both
per-IP maps and CIDR LPM maps are now included in the reconciliation contract.

For CIDRs specifically, the expected set comes from
`Store::active_cidrs`; it is not inferred from the per-IP blocked index.

If the maps remain inconsistent after the normal reconciliation interval:

1. Capture logs and `bpftool` output.
2. Do not manually delete active entries.
3. Restart the daemon only if the incident procedure allows it; WAL replay
   reconstructs durable enforcement state before normal reconciliation.

## 5. CIDR/LPM-specific failure

A CIDR block is represented in:

```text
Store::active_cidrs
        |
        +--> BLOCKCIDR   (IPv4)
        |
        +--> BLOCKCIDR6  (IPv6)
```

A per-IP block is represented in:

```text
Store::blocked_set
        |
        +--> BLOCKLIST
        +--> BLOCKLIST6
```

These are separate state domains. A successful per-IP reconciliation does not
prove that CIDR state is present in the kernel.

The implementation therefore:

1. partitions expected CIDRs by address family;
2. converts each expected network to the exact Aya LPM key;
3. removes stale kernel prefixes;
4. inserts missing prefixes;
5. restores CIDRs as permanent kernel entries;
6. leaves TTL authority with the enforcement actor.

Aya exposes `LpmTrie::keys()` as a fallible iterator and exact-key removal via
`remove`, which is why reconciliation collects the current keys before
mutating the trie. citeturn1search0turn1search3

## 6. WAL recovery

### Current contract

The current WAL implementation uses **append-before-mutate**:

```text
WAL append
   -> Store mutation
      -> TTL scheduling
         -> XDP
```

This guarantees that a successfully journaled command exists before the
corresponding in-memory transition is attempted.

### Important limitation

A storage mutation can still fail after the WAL record has been appended. The
existing WAL format does not contain a two-phase commit/abort record, so this
is a known recovery-semantics limitation rather than a claim of full ACID
transactionality.

Do not describe the current WAL as a database transaction log.

### Recovery procedure

1. Preserve the WAL directory.
2. Do not delete the newest segment.
3. Inspect logs for `WAL replay` messages.
4. Confirm recovered block counts.
5. Confirm CIDR recovery separately.
6. Confirm XDP reconciliation after startup.

Use:

```bash
./scripts/ramshield-wal-inspect.sh /path/to/wal
```

The script only reads segment metadata and never rewrites WAL files.

## 7. Security audit / CI

The security audit must be treated as a deliberate policy gate. Do not hide a
failing dependency audit with `|| true` in a release workflow.

Recommended release checks:

```bash
cargo test --workspace --all-targets --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
cargo audit
```

If stable compatibility is required in addition to the repository's nightly
toolchain, make it an explicitly named compatibility job rather than silently
mixing toolchains.

## 8. GitHub automation security

The `.github/scripts` automation layer is privileged infrastructure.

Rules:

- Do not execute untrusted strings through `shell=True`.
- Prefer argument arrays / direct process APIs.
- Keep workflow token permissions minimal.
- Never expose repository secrets to untrusted pull-request code.
- Separate read-only analysis agents from write/commit agents.
- Treat issue/PR text as untrusted input.

Rust's `std::process::Command::arg/args` passes arguments literally rather than
through a shell, which is the preferred process-spawning model when shell
syntax is not required. citeturn0search1turn0search3

## 9. Incident evidence checklist

Collect:

```text
- RamShield version
- git revision
- kernel version
- network interface
- XDP attach mode
- active block count
- active CIDR count
- WAL directory and segment list
- last WAL LSN
- XDP counters
- enforcement/XDP error logs
- configuration validation errors
- relevant systemd journal window
```

Do not collect or paste:

```text
- IPC HMAC secrets
- dashboard passwords
- Argon2 plaintext passwords
- session cookies
```

## 10. Change procedure for enforcement code

Any change touching:

```text
XdpApplier
Store::blocked_set
Store::active_cidrs
WalEntry
replay_wal_*
EnforcementService::enforce
EnforcementService::expire_due
```

must include:

1. a unit regression test;
2. a restart/replay test where applicable;
3. an XDP reconciliation test where applicable;
4. a comment documenting ownership and failure semantics;
5. a runbook update if operator behavior changes.

## 11. Future WAL transaction work

The remaining architectural improvement is a true command transaction protocol:

```text
Intent LSN
   |
   +--> state mutation
   |
   +--> Commit LSN

replay:
   apply only intents with a durable Commit
```

A format/version migration is required before implementing this. Until then,
append-before-mutate is the supported behavior and the storage-failure window
must remain visible in operational documentation.
