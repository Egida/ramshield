# Condition-driven mesh sync prototype

**Status:** isolated prototype; not wired into the TCP transport and not production-ready.

**Branch:** `prototype/condition-driven-mesh-sync-20261011`

## Goal

Replace unconditional full-state anti-entropy every two seconds with a per-peer scheduler that syncs when evidence suggests it is needed, while retaining a bounded periodic repair pass. This is an optimization of *when* to send anti-entropy, not a change to CRDT merge semantics.

## Conditions

| Condition | Scheduling behavior |
|---|---|
| Local mutation | Coalesce nearby changes for 50 ms, then schedule repair |
| Remote state applied | Mark state dirty and coalesce |
| Peer reconnect | Request immediate repair |
| Delivery failure | Retain dirty state and retry with bounded exponential backoff |
| Incoming queue overflow | Request repair because a delta may have been dropped |
| Periodic repair deadline | Run a safety-net sync even if no trigger was observed |

Defaults are prototype values: 50 ms coalescing, 30 s repair interval, 250 ms initial retry, and 10 s maximum retry delay. Tests use short deterministic durations. Scheduling uses monotonic elapsed time, not wall-clock time.

## State-machine contract

Each configured peer owns one `ConditionSyncPlanner`.

1. The caller reports a condition with a monotonic timestamp.
2. `take_due(now)` claims at most one pass; overlapping passes are prevented.
3. The caller performs the existing bounded snapshot/chunk/send work for that peer.
4. The caller invokes `complete(now, success)` exactly once for each claimed pass.
5. Failure leaves the peer dirty and applies capped exponential backoff.
6. A condition arriving during an in-flight pass remains pending; completion of the older pass cannot erase it.
7. Periodic repair remains mandatory because notifications can be missed and a successful socket write is not proof that remote state was applied.

## Critical integration work still required

This module is only a tested policy state machine. The current transport still runs unconditional global sync and has no per-peer acknowledgements or durable delivery cursors. Do not claim network-level conditional sync until a follow-up integration:

- gives every peer its own planner and sync task;
- wires local mutation, applied remote state, reconnect, send failure, and queue-overflow signals into that peer's planner;
- makes snapshot pages fair per peer rather than advancing one shared cursor for all peers;
- distinguishes bytes-written from authenticated/applied acknowledgement, if acknowledgements are introduced;
- keeps state changes that happen during a snapshot dirty for the next pass;
- proves recovery after packet loss, disconnection, process restart, more than 4,096 entries/tombstones, and a saturated inbound queue;
- verifies bounded memory, bounded task count, cancellation, and shutdown while sends are stalled.

## Prototype tests

The module has deterministic unit tests for mutation coalescing, reconnect fast-path, periodic fallback, capped retry behavior, conditions during an in-flight pass, queue-overflow repair, no overlapping passes, and invalid policy settings. These are source-level tests only until exact-SHA CI runs.

## Acceptance bar

Do not replace the current transport scheduler until the integrated implementation demonstrates:
- convergence with dropped, duplicated, delayed, and reordered messages;
- every page and tombstone eventually visited for each peer independently;
- bounded recovery latency after reconnect/overflow/failure;
- no hot retry loop for an unreachable peer;
- no lost dirty signal when mutation races with sync completion;
- core feature-off suite unchanged and green;
- mesh-only and mesh-integrated CI, strict Clippy, formatting, and independent review all green on one exact SHA.

Until those gates pass, keep mesh optional and disabled in production.
