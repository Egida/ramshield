# Architecture

RamShield is a single-node userspace detection service with optional kernel-level enforcement.

## System flow

```text
proxy / telemetry source
          |
          v
    JSON/TCP IPC
          |
          v
 bounded ingest + pre-aggregation
          |
          v
      detection
          |
          v
   enforcement queue
          |
          +------> WAL (when enabled)
          |
          v
     enforcement
          |
          v
      XDP maps
          |
          v
       network
```

Detection runs in userspace. XDP is the enforcement boundary, not the detector.

## Main components

| Component | Responsibility |
|---|---|
| `src/main.rs` | daemon startup, config selection, signals, shutdown |
| `src/cli.rs` | `ramshield-cli` client |
| `src/engine/` | pipeline startup, runtime state, dashboard snapshot |
| `src/ipc/` | TCP IPC server and request authentication/authorization |
| `src/dashboard/` | HTTP dashboard, health, metrics, APIs |
| `ramshield-config` | TOML schema, environment overrides, validation |
| `ramshield-detection` | rate and anomaly detection |
| `ramshield-forecasting` | forecasting/anomaly support |
| `ramshield-enforcement` | block lifecycle, TTL, WAL integration |
| `ramshield-storage` | in-memory state and TTL/subnet indexes |
| `ramshield-metrics` | counters, snapshots, Prometheus export |
| `ramshield-protocol` | IPC request/response/auth contract |
| `ramshield-xdp` | userspace XDP loader and eBPF program |
| `ramshield-cgnat` | CGNAT telemetry support |
| `ramshield-analytics` | batch analytics |
| `ramshield-mesh` | multi-instance coordination code; not the primary single-node deployment model |

## Enforcement order

For a block command, the current enforcement service is designed around:

```text
block command
    ↓
WAL append (when WAL is enabled)
    ↓
store mutation
    ↓
TTL scheduling
    ↓
XDP map update
    ↓
runtime result
```

The exact `xdp_applied` result is observable separately from the committed block result. A block record in userspace should not be interpreted as proof that a kernel map update succeeded.

**Architectural invariant (0.3.1):** WAL/userspace state is authoritative. XDP is a projection.
A failure of the projection must never destroy the authoritative security decision.
A configured security boundary must never report itself healthy when that boundary is inactive.

## XDP

When `[xdp].enabled = true`, the engine attempts to load and attach the XDP program.

If attachment fails:
- `allow_inband_fallback = false` (default) → pipeline startup fails, `/healthz` returns 503.
- `allow_inband_fallback = true` → daemon runs degraded (in-band enforcement), `/healthz` returns 200 with `protection_state: degraded`.

See `protection_state` on `/healthz` and `/api/snapshot` for the current enforcement level.

BLOCKLIST and BLOCKLIST6 are LRU hash maps. Kernel eviction of a live block is expected under map pressure. Userspace remains authoritative: the enforcement tick (~10s) re-inserts every expected IP and CIDR via `reconcile()`. Observe `ramshield_xdp_reconcile_successes_total` and `ramshield_xdp_reconcile_failures_total`. There is no separate critical/bulk map split.

## WAL and recovery

When WAL is enabled:

1. the WAL is opened during startup;
2. prior block/unblock entries are replayed;
3. expired blocks are skipped;
4. live block expirations are re-armed;
5. the running enforcement service can reconcile recovered state.

When WAL is enabled and `allow_volatile_fallback=false` (default), open or replay failure is a startup failure — the daemon never becomes ready with an empty reconstructed block set. Volatile continuation requires an explicit `allow_volatile_fallback=true`.

## Shutdown

SIGINT, SIGTERM, and SIGHUP enter the same shutdown path.

The daemon asks the engine to stop, joins worker threads with a bounded grace period, and allows the enforcement pipeline to drain.

The XDP applier is owned by the enforcement service, so normal process shutdown releases its kernel attachment.

## Dashboard

The dashboard runs in its own OS thread/runtime and exposes:

- `/healthz`
- `/metrics`
- `/api/snapshot`
- `/api/stream`
- `/api/history/batches`
- `/api/history/blocks`
- `/api/blocks/active`
- `/api/traffic/subnets`
- `/api/status/modules`
- `/api/config`

Authenticated administrative routes use the dashboard auth middleware.

## Important limitations

The current architecture is intentionally single-node.

The repository contains mesh/analytics/CGNAT components, but their presence in the workspace should not be read as a claim that a production multi-node deployment is supported.

Likewise, userspace fallback is not equivalent to XDP enforcement: it preserves the decision/block path but does not provide the same kernel-level packet-drop boundary.