# Architecture

System design, major components, data flow, and failure boundaries.

## System overview

```text
reverse proxy / client telemetry
         |
         v
 JSON/TCP IPC server (HMAC-SHA256 authed)
         |
         v
 bounded ingest + sharded pre-aggregation (256 shards)
         |
         v
 detection: EWMA / Holt-Winters / pulse-wave / dual-gate swarm
         |
    +----+----+
    |         |
    v         v
  WAL    enforcement
              |
    +---------+---------+
    |                   |
    v                   v
shared runtime      eBPF/XDP maps
and dashboard       IP/CIDR drops
```

## Major crates

| Crate | Purpose |
|---|---|
| `ramshield-config` | TOML configuration, validation, env-override parsing |
| `ramshield-detection` | EWMA, Holt-Winters, pulse-wave, dual-gate swarm detection |
| `ramshield-enforcement` | Block lifecycle, single-writer enforcement actor, WAL persistence |
| `ramshield-forecasting` | Holt-Winters + SPOT-lite alarm |
| `ramshield-metrics` | Atomic counters, metric export, Prometheus exposition |
| `ramshield-protocol` | IPC wire contract (Request/Response types, auth frame) |
| `ramshield-storage` | Sharded DashMap store, TTL wheel, subnet index |
| `ramshield-types` | Shared types (IpRecord, BlockState, CIDR) |
| `ramshield-xdp` | Userspace XDP loader + aya eBPF program |
| `ramshield-cgnat` | Shared-memory telemetry on CGNAT path |
| `ramshield-analytics` | Batch analytics |
| `ramshield-mesh` | Multi-instance coordination |

## Data flow

### Telemetry ingestion
1. Proxy sends connection event via JSON/TCP IPC (single or batched).
2. IPC server parses, authenticates, authorizes the frame.
3. Event enters bounded ingest channel → sharded pre-aggregator.

### Detection flow
1. Pre-aggregator batches events every `batch_window_ms` (default 25 ms).
2. Detection pipeline: EWMA rate gate → Holt-Winters forecast → entropy scan → pulse-wave detection.
3. Dual-gate swarm checker keys on distinct source IPs per /24.
4. Threshold crossing → `EnforceCommand` sent to enforcement actor.

### Enforcement flow
1. Enforcement actor receives `EnforceCommand`.
2. WAL append (fsync-grouped for durability).
3. Store mutation (mark blocked in DashMap).
4. TTL ring scheduling.
5. XDP map push (IP + CIDR prefix to kernel LPM trie).
6. Block confirmed in runtime state.

### XDP enforcement
- XDP program runs in kernel, drops packets before socket layer.
- Per-IP and per-CIDR prefix matches via longest-prefix trie.
- Per-CPU drop counters exported via RingBuf.
- XDP reconciliation runs periodically to detect and fix map drift.

## Failure boundaries

| What fails | Consequence |
|---|---|
| Detection thread panics | No new blocks. Existing blocks remain active. |
| Enforcement channel full | Block command dropped (`enforcement_dropped`). Attacker unblocked. |
| WAL append fails | Block decision aborted before store mutation. |
| XDP map full (ENOSPC) | Block stored in memory only. Kernel not updated. |
| Dashboard thread panics | Engine continues. Dashboard/API unavailable. |
| Process crash | WAL replay on restart restores block state. |

## Performance envelope

Measured on 2023 laptop (8C/16T, 32 GB, NVMe):

| Metric | Value |
|---|---|
| IPC ingest | 135k events/s (peak) |
| Detect → mitigate | 108 ms (warm), 8 s (cold — one full window) |
| RSS idle | 44 MB |
| RAM growth | 0.0004% per million events |
| Benign FPR | 0/200 IPs (0.0000%) |
| Background EPS under attack | −93.5% (known limitation, tracked) |

See [DDOS BENCHMARK REPORT](DDOS_BENCHMARK_REPORT.md) for full methodology and caveats.

## Key design decisions

- **Detection in userspace, enforcement in kernel.** XDP is the enforcement boundary, not the detector. Without XDP, daemon degrades to userspace blocking.
- **Single-writer enforcement.** One actor owns block state. No concurrent writers → no races.
- **WAL-first ordering.** Append to WAL before store mutation. Failed append aborts the entire decision.
- **Bounded memory.** Hard RAM ceiling with cold-IP eviction. Blocked IPs never evicted.
- **Dual-gate subnet blocking.** A /24 is blocked only when both distinct-IP threshold AND event-volume threshold are met.
