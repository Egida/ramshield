# NATIVE INGEST DROP ACCOUNTING — the capture path must not lose events silently

**Finding:** audit F3 (21 October 2026 master audit), independently confirmed in
`docs/development/MASTER_AUDIT_2026_10_21_REVIEW.md`.
**Branch:** `fix/master-p0-p1-fixes` — base `54ee1b7` (`origin/master`).
**Scope:** the native capture producer's accounting of events it cannot
deliver. No other subsystem is touched.

---

## 1. The defect

`src/engine/native.rs` ended its per-packet path with:

```rust
let _ = tx.try_send(ev);
```

The native capture path sends observation events downstream **without
blocking**. That is the correct overload policy for a real-time ingest path:
the capture thread must never stall because the detection pipeline is busy.
A rejected event is therefore dropped, by design.

But the result was discarded, so **no counter moved**. The loss was invisible
in both directions:

- to the operator — the Prometheus export had no metric for it, and
- to the codebase itself — a repo-wide search showed `Metrics::inc_shed` is
  never called anywhere, so the pre-existing `ramshield_events_shed_total`
  was permanently `0` as well.

So the policy was sound and the accounting was absent. Under a flood — the
exact condition the design is built for — the operator could not tell that
observation events were being lost.

`native_ingest.enabled` defaults to `false`, so this path is opt-in; the
blast radius is limited to deployments that enable it.

---

## 2. The change

### 2.1 Two counters, with the serious case broken out

`crates/ramshield-metrics/src/lib.rs` gains:

| Counter | Meaning |
|---|---|
| `native_ingest_drops` | Every observation event the native producer built but could not hand to the downstream channel. |
| `native_ingest_drops_disconnected` | The subset where the channel was **closed** rather than full — the detection pipeline is gone, not busy. |

A full channel is backpressure; a closed channel is a dead pipeline. They are
different failures and an operator must be able to tell them apart, so the
disconnect case is broken out rather than folded into a single number.

Exported as `ramshield_native_ingest_drops_total` and
`ramshield_native_ingest_drops_disconnected_total`.

The pre-existing `events_shed` counter was deliberately **not** reused: it is
documented as "Low-signal events shed at the **IPC** high-water mark", which
is a different producer and a different policy.

### 2.2 The count happens where the loss happens

`process_packet` now matches on the `try_send` result: `Full` increments the
drop counter; `Disconnected` increments both counters and emits the error.

### 2.3 The error is emitted exactly once

A closed channel rejects every subsequent event, so an error log per event
would be a log flood. The guard is a `compare_exchange(0, 1)` on an
`AtomicU64` that counts log **emissions** — not a load-then-store, so exactly
one thread may emit the error even if several workers notice the closed
channel at the same moment. The counter also makes the one-shot property
*observable*, so a test can prove it rather than infer it from the code.

The counter is shared by every worker, so the daemon logs the condition once
overall.

### 2.4 Queue pressure is visible, not just the loss

A drop counter says *that* events were lost. It does not say whether the
queue was genuinely too small for the load or whether a brief spike caught it
out. Two gauges now answer that:

- `ramshield_ingest_channel_depth` — the current occupancy, which already
  existed and is written by the engine snapshot path;
- `ramshield_ingest_channel_depth_peak` — the high-water mark since process
  start, added here.

The peak is maintained in `set_channel_depth`, which is the only place the
depth is observed, so the peak cannot drift out of step with the value it
summarises. It only ever rises (`fetch_max`), so it needs no lock and no
compare-and-swap loop.

### 2.5 The event-rate cap is documented as what it is

`native_ingest.max_events_per_sec` is an **admission cap, not a delivery
guarantee**. It bounds how many packets may *become* observation events per
second; it says nothing about whether an admitted event reaches the
detection pipeline, because the downstream queue is bounded and a saturated
queue drops the event rather than blocking the capture thread. The config
field's own doc comment, the config crate README and the metrics crate README
now say so, and point at the drop counter and the queue-pressure gauges.

### 2.6 The scope is stated precisely

The counter covers **channel rejections only**. A packet that never becomes
an event — a non-IP frame, a trusted-overlay skip, or one refused by the
per-second admission budget — is not counted, because no event was built to
deliver. The struct documentation, the Prometheus `HELP` text and the crate
README all say so, and the README also records that these counters are
Prometheus-only: the dashboard's own drop figures do not include native
ingest drops.

---

## 3. The evidence

Every cycle below was observed with the **final** code.

### 3.1 RED — the drops were invisible

With the counting removed and the final test code in place:

```
assertion `left == right` failed: a dropped observation event must be counted
  left: 0
 right: 1
```

and, for the five-rejection case, `left: 0, right: 5`.

### 3.2 GREEN

```
engine::native::tests::native_ingest_counts_a_drop_when_the_channel_is_full ... ok
engine::native::tests::native_ingest_counts_a_disconnect_separately_from_a_full_channel ... ok
engine::native::tests::native_ingest_does_not_count_a_delivered_event_as_a_drop ... ok
engine::native::tests::native_ingest_logs_a_disconnect_only_once ... ok
engine::native::tests::native_ingest_emits_the_disconnect_error_once_under_contention ... ok
tests::native_ingest_drop_counters_are_exported ... ok
```

### 3.3 The tests are discriminating

Removing the one-shot guard fails three of them:

```
assertion `left == right` failed: exactly one thread may emit the error,
however many notice the closed channel
```

### 3.4 What each test pins

| Test | Pins |
|---|---|
| `..._counts_a_drop_when_the_channel_is_full` | a full channel: 1 drop, 0 disconnects, no log |
| `..._counts_a_disconnect_separately_from_a_full_channel` | a closed channel: 1 drop, 1 disconnect, 1 emission |
| `..._does_not_count_a_delivered_event_as_a_drop` | the success path moves no counter at all |
| `..._logs_a_disconnect_only_once` | 5 rejections → 5 drops, 5 disconnects, **1** emission |
| `..._emits_the_disconnect_error_once_under_contention` | 8 threads × 100 events → 800 drops, 800 disconnects, **1** emission |
| `native_ingest_drop_counters_are_exported` | both metrics reach the Prometheus export with the right values |

The assertions are exact deltas, not "greater than zero".

### 3.5 The hot path is unaffected

The success arm is `Ok(()) => {}`. The old code already computed the
`try_send` result and threw it away; the new code matches on that same
result. No atomic, no allocation and no lock is added to the success path —
the handle is borrowed (`&Metrics`), not cloned per packet, and the only
atomic read-modify-writes happen on the rejection path.

### 3.6 The gates

All 14 CI-equivalent gates pass on the final tree — see §5.

---

## 4. What was deliberately NOT changed

- **The overload policy.** The capture thread still does not block on a full
  queue. Blocking would trade a lost observation for a stalled capture, which
  is the worse failure.
- **The admission budget.** A packet refused by `max_events_per_sec` is not
  counted as a drop, because no event was built. That is a separate policy
  with its own semantics.
- **The dashboard's JSON snapshot.** Extending the typed snapshot struct
  means a public-field addition plus UI work, for an opt-in path; Prometheus
  is the right surface for a scrape-and-alert counter. The limitation is
  recorded in the crate README rather than left implicit.
- **The detection pipeline's shedding.** `events_shed` is IPC-specific and
  was left alone; this change does not claim to account for it.

---

## 5. Verification

```
cargo fmt --all --check                          clean
cargo clippy --workspace --all-targets --all-features -- -D warnings
                                                   clean
cargo test --lib native_ingest                    5 passed, 0 failed
cargo test -p ramshield-metrics                  15 passed, 0 failed
14-gate CI-equivalent suite                      14/14 EXIT=0
```

Gate log: `/tmp/f01/native3_gates.log` (local scratch, not committed).

---

## 6. Review

`docs/development/NATIVE_INGEST_DROPS_REVIEW.md` — independent review,
**VERDICT: PASS / SAFE TO PUSH: yes**. Its findings were addressed in this
batch:

| Observation | Response |
|---|---|
| 1 — `metrics` was not in the non-Linux discard tuple, so a non-Linux build would warn `unused variable: metrics` | Added to the tuple (`src/engine/native.rs`). It does not break CI today only because the clippy gate runs on ubuntu; the next clippy run on another runner would have. |
| 2 — the counter's documented scope was broader than what it counts | The struct doc, the Prometheus `HELP` text and the README now say "built but could not hand to the downstream channel", and state that a packet which never becomes an event is not counted |
| 3 — the emission counter is not exported | Accepted as-is: the operator-visible artefact is the error line, and the counter proves the one-shot property in-process. Recorded here. |
| 4 — Prometheus-only export | Accepted as-is, and now stated in the README, including that the dashboard's own drop figures do not include native ingest drops |
| g(i) — no test asserts the Prometheus export contains the new metrics | Added: `native_ingest_drop_counters_are_exported` |
| g(ii) — no test asserts the success path leaves the counters at 0 | Added: `native_ingest_does_not_count_a_delivered_event_as_a_drop` |
| g(iii) — no multi-threaded test exercises the compare-exchange under contention | Added: `native_ingest_emits_the_disconnect_error_once_under_contention` (8 threads × 100 events) |
| the review's F3 action also asked to "export counters **and queue pressure**" and to "document that `max_events_per_sec` is not a delivery guarantee" | Added in the follow-up commit: `ramshield_ingest_channel_depth_peak` (high-water mark, monotonic, maintained in the one place the depth is observed) and the admission-cap wording in the config field, the config README and the metrics README |
