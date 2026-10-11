# Condition-driven mesh sync prototype

**Status:** integrated transport prototype; exact-SHA acceptance and network-level convergence tests are still required. Not production-ready.

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

Defaults are prototype values: 50 ms coalescing, 30 s repair interval, 250 ms initial retry, and 10 s maximum retry delay. The transport scheduler wakes every 25 ms with delayed missed-tick behavior; tests use short deterministic durations. Scheduling uses monotonic elapsed time, not wall-clock time.

## State-machine contract

Each configured peer owns one `ConditionSyncPlanner`.

1. The caller reports a condition with a monotonic timestamp.
2. `take_due(now)` claims at most one pass; overlapping passes are prevented.
3. The caller performs the existing bounded snapshot/chunk/send work for that peer.
4. The caller invokes `complete(now, success)` exactly once for each claimed pass.
5. Failure leaves the peer dirty and applies capped exponential backoff.
6. A condition arriving during an in-flight pass remains pending; completion of the older pass cannot erase it.
7. Periodic repair remains mandatory because notifications can be missed and a successful socket write is not proof that remote state was applied.

## Current transport integration

The prototype is wired into `MeshHandle` on this branch:

- each configured peer has its own planner, delivery-failure flag, and snapshot cursor;
- local `broadcast` calls mark peers dirty; send failures request repair; a successful send after a recorded failure triggers reconnect repair;
- inbound queue overflow is surfaced to the scheduler after releasing the queue lock;
- the owner must call `notify_remote_state_applied()` **after** drained messages have actually been merged into the CRDT. Draining alone is deliberately not treated as application success;
- a 30-second periodic repair remains as a safety net, while large snapshots continue page-by-page;
- each page is capped at 4,096 entries and each wire message at 128 entries; active page sends use the shared writer semaphore and bounded per-frame deadlines;
- caller-owned `snapshot_page(cursor, limit)` cursors keep one peer from advancing another peer's pagination position.

## Remaining acceptance work

This is still a prototype, not a production-ready convergence protocol:

- a successful TCP write is not an authenticated/applied acknowledgement; there are no durable delivery acknowledgements or durable cursors;
- the current shared HMAC trust model does not provide independent per-origin signatures;
- the higher-level service must call `notify_remote_state_applied()` after merge; this callback is not automatically inferred by the transport;
- prove recovery after packet loss, disconnection, process restart, more than 4,096 live entries/tombstones, and a saturated inbound queue;
- test concurrent mutation while pages are emitted, cursor behavior as entries are inserted/removed, peer starvation, bounded memory/task count, cancellation, and shutdown while sends are stalled;
- run mesh-only and workspace gates and obtain independent review on one exact SHA.

Keep mesh optional and disabled in production until these gates pass.

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
