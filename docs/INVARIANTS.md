# Architectural invariants (audit §32)

Every production‑mutation path maintains these. A test asserting them after
every complex operation catches regressions the same way `assert_store_invariants`
has been catching secondary‑index leaks.

---

## Store

```
blocked_count  ==  |{ IP | IpRecord.block_state ∈ {Blocked} }|
```

```
blocked_set    ==  { IP | IpRecord.block_state ∈ {Blocked} }
```

```
ttl_entries    ==  |{ Entry | Entry.expires_at ≠ None }|
```

```
ram_bytes      ==  Σ (size_of::<Entry> + value.heap_bytes() + size_of::<IpAddr>)
```

```
ram_bytes + entry_size ≤ ram_limit_mb * 1_048_576   (fresh‑insert path;
                                                       replacement is unbounded)
```

---

## WAL

```
written_lsn  ≥  durable_lsn       (all committed writes)
```

```
ckpt_lsn     ≤  durable_lsn       (checkpoint must be durable before
                                    we can discard older segments)
```

```
durable_lsn  ≤  written_lsn       (observed after a crash: some
                                    written bytes may not have crossed
                                    the durability barrier)
```

After `sync()` returns `Ok`:

```
durable_lsn  ≥  LSN of last `append` before the `sync()` call.
```

---

## Checkpoint

```
snapshot_lsn       ≤  durable_lsn
```

`checkpoint(lsn)` rejects when:

```
lsn  >  current_lsn               (cannot checkpoint the future)
lsn  >  durable_lsn               (CheckpointNotDurable)
lsn  <  previous checkpoint LSN   (CheckpointRegression  —  regression)
```

---

## Replay

```
recovered state  =  snapshot(lsn₀)
                    ∪  replay(WAL records > lsn₀ up to recovered_last_lsn)
```

Every recovered IP that exists in the snapshot AND has matching records in the
replayed tail sees its `block_state` reconciled: the WAL (not the snapshot)
wins, because the WAL is the authoritative record of the latest security
decision.

---

## Enforcement

```
userspace truth  →  XDP desired state  →  periodic reconcile()
```

A block that was committed to userspace and the WAL is authoritative even when
XDP map eviction has dropped it. `reconcile()` re‑inserts expected IPs/CIDRs
into the XDP maps every ~10 s.

No per‑IP `block_state` exists for CIDR‑blocked hosts. Query must check
`active_cidrs` explicitly (`is_blocked_by_cidr`).

---

## XDP

```
parse failure  →  PASS
known blocked  →  DROP
expired rule   →  PASS
```

Malformed / unrecognised packets must pass — XDP is a projection of userspace
decisions, not a second detection engine.

---

## Instrumentation

Each crate that maintains secondary indexes over an authoritative source should
expose a test‑only `assert_invariants()` function (postfixed with `_invariants`
for consistency) that reads both the authority and every derived structure
under the relevant locks / atomic variables.

Existing:
- `ramshield-storage::Store` — `assert_store_invariants()` covers `blocked_set`,
  `blocked_count`, `ttl_entries`, `ram_bytes`‑vs‑live‑entry sum.
- This document —  `assert_wal_invariants()` targets `durable_lsn`,
  `written_lsn`, `ckpt_lsn` ordering.

`ponytail:` enforcement‑side (TTL schedule vs XDP map membership) currently
has no cheap O(1) assertion — the reconciliation tick is the proof, and it runs
continuously. If the schedule → map synchronisation ever fails it is caught by
integration/diagnostic tests, not by a unit‑level assertion.

### Relative detection

- The relative detector is opt-in and default-off.
- Relative thresholds use the prior slow baseline; the current sample cannot
  move its own threshold.
- Relative breach state lives on `IpRecord` and is serialized with it; no
  second per-IP detector map is authoritative.
- A relative block requires both maturity and consecutive breach criteria.

### XDP projection freshness

- The engine owns the projection freshness threshold.
- Dashboard and Prometheus expose the same engine-derived stale value.
- A configured active XDP dataplane with stale projection cannot report fully
  healthy; fallback policy determines Degraded versus Failed.
