# RAMWAL

A recovery-first append-only log for Rust.

> A WAL is easy to write. Recovery is the product.

## Why

Processes crash. Storage writes can be torn. State has to come back.

RAMWAL provides a small durable log and a conservative recovery engine
for reconstructing state after failure.

## Guarantees

- checksummed records (CRC-32C over the stored payload)
- explicit durability semantics (`Buffered`, `Explicit`, `SyncEach`, `GroupCommit`)
- deterministic recovery — the writer may only append at the boundary recovery proved
- torn-tail repair, but only at EOF
- corruption detection — a complete but invalid record is never silently truncated
- strictly-increasing LSN enforcement across segment boundaries
- segmented storage with checkpoint-aware retention

The two rules everything derives from:

```text
INCOMPLETE (only at EOF)  →  safe to repair
INVALID                   →  stop, report corruption
```

## Example

```rust
use ramwal::{Wal, config::Config, config::Durability, lsn::Lsn};

let mut cfg = Config::new("./wal");
cfg.durability = Durability::SyncEach;
let wal = Wal::open(cfg)?;

// Append is not durable until the barrier is crossed.
let lsn: Lsn = wal.append(b"increment")?;
wal.sync()?;                       // lsn is now durable

// Acknowledge state only up to a checkpoint the app has persisted.
wal.checkpoint(lsn)?;
wal.truncate_before(lsn)?;         // may now drop older segments
# Ok::<(), ramwal::error::Error>(())
```

## Recovery

Recovery runs inside `Wal::open`. There is no separate "scan for the max
LSN" path — the recovery engine owns the append boundary:

```rust
let wal = Wal::open(cfg)?;
let report = wal.recovery_report();

println!("segments      {}", report.segments);
println!("records       {}", report.records.len());
println!("first LSN     {:?}", report.first_lsn);
println!("last LSN      {:?}", report.last_lsn);
println!("tail          {:?}", report.tail);
println!("truncated     {}", report.truncated_bytes);
println!("repaired      {}", report.repaired);

// Every verified record, in order:
for rec in &report.records {
    replay(rec.lsn, &rec.payload);
}
```

Recovery either proves a prefix of the log valid or reports the exact
corruption. It never guesses, and it never uses `max()` to paper over an
out-of-order record.

## CLI

```text
ramwal-cli verify  <wal-dir>   # scan only — never mutates
ramwal-cli recover <wal-dir>   # explicitly allows torn-tail repair
```

`verify` reports a torn tail without touching the file; `recover` truncates
the incomplete tail back to the last verified record.

## Documentation

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — layering, flows, boundaries
- [`docs/FORMAT.md`](docs/FORMAT.md) — the on-disk format, **authoritative**
- [`docs/RECOVERY.md`](docs/RECOVERY.md) — scanner algorithm and rules
- [`docs/DURABILITY.md`](docs/DURABILITY.md) — the sync contract and modes
- [`docs/CHECKPOINTING.md`](docs/CHECKPOINTING.md) — snapshot ownership, retention
- [`docs/CORRUPTION.md`](docs/CORRUPTION.md) — damage classification matrix
- [`docs/API_FREEZE.md`](docs/API_FREEZE.md) — frozen public surface
- [`RELEASES.md`](RELEASES.md) — release notes
- [`SECURITY.md`](SECURITY.md) — what is and is not a security boundary

## License

MIT