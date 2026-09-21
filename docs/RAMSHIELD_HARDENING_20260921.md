# RamShield Hardening Integration — 2026-09-21

## Integrated changes

### H1 — CIDR-aware XDP reconciliation

`XdpApplier::reconcile()` now receives both:

- expected per-IP blocks;
- expected CIDR blocks.

`EnforcementService` obtains the CIDR source of truth from
`Store::active_cidrs` on initial and periodic reconciliation.

The Aya implementation independently reconciles:

- `BLOCKLIST`
- `BLOCKLIST6`
- `BLOCKCIDR`
- `BLOCKCIDR6`

The CIDR maps use Aya `LpmTrie::keys()` followed by exact-key removal and
reinsertion of the expected set. Aya documents `keys()` as a fallible iterator
and `remove()` as exact prefix/data removal, which matches this reconciliation
strategy.

### H2 — Fail-fast typed environment configuration

`Config::apply_env_overrides()` now returns `anyhow::Result<()>`.

All typed environment variables reject malformed values. This prevents a
configuration typo from silently falling back to TOML/default state.

The two configuration entry points propagate this error:

- `Config::load()`;
- no-config startup in `src/main.rs`.

A regression test verifies invalid `RAMSHIELD_ENGINE__RAM_LIMIT_MB` fails
startup.

### H3 — Operator runbook and diagnostics

Added:

- `docs/RAMSHIELD_RUNBOOK.md`
- `docs/RAMSHIELD_HARDENING_20260921.md`
- `scripts/ramshield-preflight.sh`
- `scripts/ramshield-xdp-inspect.sh`
- `scripts/ramshield-wal-inspect.sh`

All scripts are intentionally read-only.

### H4 — WAL semantics documented explicitly

The implementation remains append-before-mutate. The runbook now explicitly
states that this is durable command intent, not a complete ACID transaction.
The remaining storage-failure window is documented rather than hidden.

The next WAL format revision should add an explicit intent/commit protocol if
full transactional recovery is required.

## Validation performed in the packaging environment

The repository preflight script passes its source/layout checks.

Cargo/rustc and bpftool were not installed in the packaging environment, so no
fresh compiler, clippy, rustfmt, integration-test, or live XDP result is claimed.

On a development/CI host, run:

```bash
cargo test --workspace --all-targets --no-fail-fast
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
