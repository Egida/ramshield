# Independent review — native ingest drop accounting (audit finding F3)

Reviewer scope: the UNCOMMITTED working-tree change in
`/home/m/.cache/rs-master` (branch `fix/master-p0-p1-fixes`, HEAD `54ee1b7`).
Read-only review; no mutating command was run. All claims below are backed by
file:line in the working tree as read.

VERDICT: PASS
SAFE TO PUSH: yes

No blocking defect was confirmed. One non-blocking lint nit and three
coverage/doc-precision observations are listed under OPTIONAL OBSERVATIONS.

## Question-by-question

| # | Question | Answer | Evidence / reason |
|---|---|---|---|
| a | Is the counting correct? Can a drop escape uncounted, or be double counted? Is the Full/Disconnected split right? | Yes for the channel-send path. Exactly one `try_send` exists in the native producer and both rejection arms increment `native_ingest_drops` exactly once; `native_ingest_drops_disconnected` is a strict subset (only the Disconnected arm) and is documented as such, so it is not double counting but a documented subset. Drops that are NOT channel rejections (rate-limit budget rejection, non-IP frames, trusted-overlay skip) are not counted by this metric — by design of the name, but see OPTIONAL OBSERVATIONS 2. | `src/engine/native.rs:743` (the single `try_send`), `:747` (Full → `inc_native_ingest_drop(1)`), `:755-756` (Disconnected → drop + disconnect), `crates/ramshield-metrics/src/counters.rs:68-84`; subset semantics documented at `crates/ramshield-metrics/README.md:113-118` and `crates/ramshield-metrics/src/lib.rs:54-63` |
| b | Is the one-shot log correct under concurrency? Is `compare_exchange(0,1)` the right primitive? Can two workers both log, or can the log be lost? Does the counter still work as an observable "emitted once" proof? | Correct. A single `compare_exchange(0,1)` RMW admits exactly one caller, so at most one worker can enter the `error!` branch; the latch is created once per `spawn()` (per daemon) and shared by all workers, so the log fires at most once per daemon. Relaxed ordering is sufficient — the CAS publishes no data, it only arbitrates. The counter is a boolean latch (0/1) that proves "at most one thread entered the log branch", which is exactly the property under test. Residual: a crash between the winning CAS and the `error!` call would leave the counter at 1 with no log line (cosmetic only), and the latch is not exported to Prometheus, so the operator-visible artifact is the log line itself, not the counter. | `src/engine/native.rs:114-123` (latch created in the supervisor, shared by all workers), `:761-767` (CAS + `error!`), `:136-137` (latch cloned per worker), `crates/ramshield-metrics/src/prometheus.rs:88-101` (counter not exported) |
| c | Is the plumbing correct and complete? Is there any path that reaches a `try_send` without the metrics handle? Are the Arc clones done in the right place? | Complete. `spawn()` takes `Arc<Metrics>`; the supervisor clones it per worker *outside* the `move` closure (so each worker owns its own clone and the closure does not move the handle on the first iteration); `run_linux`/`run_socket`/`process_packet` thread it through; `boot.rs` passes `engine.metrics.clone()`. The only `try_send` in the producer is at `native.rs:743`, and every call path to it (`run_socket` at `:520`, plus the test call sites) passes a metrics handle — no un-instrumented path. | `src/engine/native.rs:103` (`spawn` signature), `:133-137` (clones outside the closure), `:142-155` (worker spawn), `:243`, `:288`, `:310`, `:528`, `:683` (plumbing), `src/engine/boot.rs:626` (`metrics.clone()`), `src/engine/native.rs:743` (sole `try_send`) |
| d | Hot-path impact: is the success path affected? Is `&Metrics` + relaxed `fetch_add` acceptable? | Negligible on the success path. The old code already computed the `try_send` result and threw it away; the new code matches on that same result. The success arm is an empty `Ok(()) => {}` — no atomic, no allocation, no branch beyond the match that the previous code's `let _ =` already implied. The metrics handle is borrowed (`&Metrics`), not cloned, per packet, and the only atomic RMWs happen on the rejection path. The rejected-path `fetch_add` on a shared cache line can be contended across up to 8 workers, but only while the downstream is saturated — a condition that is already pathological. | `src/engine/native.rs:743-768` (match arms), `:683` (`&Metrics` param, not `Arc` clone per packet), `crates/ramshield-metrics/src/counters.rs:75` / `:82` (`Relaxed` fetch_add) |
| e | Is the metric naming/export consistent with the crate's conventions, and is the README accurate? | Consistent. `ramshield_native_ingest_drops_total` / `ramshield_native_ingest_drops_disconnected_total` follow the crate's `ramshield_<name>_total` counter convention (compare `ramshield_events_shed_total`, `ramshield_ipc_event_drops_total`), are exported through the same `emit!` macro with `# HELP` + `# TYPE counter`, and are documented in the crate README in the same table style. The README's operational statement (full = backpressure, no log; closed = pipeline gone, logged once) matches the code exactly. | `crates/ramshield-metrics/src/prometheus.rs:83-101`, `crates/ramshield-metrics/README.md:108-125`, `crates/ramshield-metrics/src/lib.rs:54-63` |
| f | Any panic, deadlock, overflow, or correctness risk introduced? | No panic path introduced (no `unwrap`/`expect`/indexing added; `compare_exchange` returns a `Result` that is only `is_ok()`-tested). No deadlock: the change adds no lock, only relaxed atomics; nothing is held across the `try_send`. No overflow concern: `u64` `fetch_add` wraps rather than panicking, and 2^64 native drops is unreachable in practice. The one behavioural change is the single `error!` line per daemon, which is bounded. The `Full`/`Disconnected` arms are exhaustive over `TrySendError`, so a future third variant would be a compile error rather than a silent miss. | `src/engine/native.rs:743-771` (exhaustive match, no unwrap), `crates/ramshield-metrics/src/counters.rs:68-84` |
| g | Is the test set adequate and non-vacuous? Are the assertions meaningful? Anything untested that matters? | The three tests are non-vacuous and meaningful: they assert exact counter deltas (1/0/0, 1/1/1, 5/5/1) rather than "greater than zero", and the "log exactly once" test proves the one-shot property (5 rejections → 5 drops, 5 disconnects, still exactly 1 emission). Gaps (non-blocking): (i) no test asserts the Prometheus export actually contains the two new metrics; (ii) no test asserts the *success* path leaves both counters at 0; (iii) no multi-threaded test exercises the CAS under contention. | `src/engine/native.rs:1471` (full-channel test), `:1528` (closed-channel test), `:1573` (logged-once test); `process_packet` call sites at `:1491`, `:1546`, `:1592`; counter assertions at `:1504`/`:1509`/`:1514`, `:1551`/`:1556`/`:1561`, `:1598`/`:1603`/`:1608` |

## OPTIONAL OBSERVATIONS

1. **Unused parameter on non-Linux builds (confirmed, warning-level).**
   `spawn()` now binds `metrics: Arc<Metrics>` unconditionally
   (`src/engine/native.rs:103`), but the `#[cfg(not(target_os = "linux"))]`
   branch's discard tuple (`src/engine/native.rs:167-174`) does not include
   `metrics`, so a non-Linux build emits `unused variable: metrics`. The
   current CI clippy gate (`-D warnings`) runs on ubuntu only
   (`.github/workflows/ci.yml:62-64`, `advanced.yml:24`), and the
   macOS/Windows jobs only run `cargo build`/`cargo test`
   (`advanced.yml:42-57`), so this does not break CI today — but it will the
   moment clippy is run on a non-Linux runner. One-line fix: add `metrics` to
   that tuple. Not blocking, because the metric is Linux-only by construction.

2. **`native_ingest_drops` covers channel rejections only, but the README
   wording is broader.** The README says "Every observation event the native
   producer could not deliver" and the struct doc says "the native capture
   producer could not deliver downstream", yet a packet rejected by the
   `max_events_per_sec` budget returns before the event is ever built
   (`src/engine/native.rs:709-716`), as do non-IP frames
   (`src/engine/native.rs:696-698`) and trusted-overlay skips
   (`:701-708`). Those are also silent losses of would-be observation
   events. Either narrow the wording to "could not deliver *downstream*"
   (the Prometheus HELP text already says this correctly,
   `crates/ramshield-metrics/src/prometheus.rs:91`) or add a separate
   rate-limit-shed counter. This is a documentation-accuracy nit, not a
   counting bug — the counter name and HELP text are consistent with each
   other.

3. **The disconnect latch is a test-only observable.**
   `disconnect_log_emissions` is deliberately not exported
   (`crates/ramshield-metrics/src/prometheus.rs:88-101` exports only the two
   drop counters), so the operator-visible artifact of the one-shot is the
   `error!` log line, and the counter proves the property only in-process.
   That is a reasonable split, but it means the "logged exactly once"
   guarantee is not independently checkable from metrics. It is also a
   per-`spawn()` latch: if the detection pipeline were ever re-created with a
   new receiver on the same sender (currently impossible — a crossbeam
   channel stays disconnected once all receivers are dropped), the log would
   not fire again. Acceptable as-is.

4. **Prometheus-only export is defensible.** The dashboard JSON snapshot
   (`crates/ramshield-metrics/src/types.rs:41-90`) is a typed struct
   consumed by the dashboard UI; extending it means a public-struct field
   addition plus UI work, for a path that is opt-in and off by default
   (`native_ingest.enabled` defaults to false, `src/engine/boot.rs:619`).
   Prometheus is the right surface for a scrape-and-alert counter. The
   consequence to note explicitly: the dashboard's own drop figures
   (`src/dashboard/mod.rs:480-482`) will show zero native drops even while
   Prometheus shows non-zero — that is consistent with the documented
   decision, but an operator reading only the dashboard will not see this
   loss. No change required.

5. **UNVERIFIED — not independently confirmed by this review:** that the
   three tests were observed RED then GREEN with the final code (taken as
   given per the task brief); that the change compiles cleanly under
   `cargo clippy -D warnings` on Linux (no build/clippy was run — the review
   is read-only by instruction); and the actual measured throughput effect
   of the change on the native ingest path (no benchmark was run). The
   hot-path reasoning in row (d) is from code reading, not measurement.

REVIEWED IN 18 TOOL CALLS
