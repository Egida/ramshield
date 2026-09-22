# Attack gauntlet, round 2 (2026-09-22)

Round 2 shifted from round 1's "does anything crash" to "are the mechanisms
correct under adversarial shapes": synchronized expirations, kernel map
saturation, distributed evasion, auth-wall starvation, dashboard load, and
sustained mixed contention with a concurrent soak. Same discipline: live prod
(HMAC, ports 7890/9999, XDP wlp2s0), sources confined to RFC5737 ranges,
queue-driven driver rescanned between slots, 3 s sampler throughout.

## Setup

A soak process ran for the entire ~18-minute window underneath every round:
8 block writers (5000-IP pool, ttl 600) + 4 pipelined readers + 3 detection
feeders (~12k eps). Every round then ran on top of that background load, so
all numbers below are under-contention numbers, not clean-room ones.

## Rounds

B1 — TTL deadline avalanche. 20k blocks, one shared 4 s TTL, get_stats
probes at 500 ms through the expiry wave. Wave was invisible: write
p50 1.4 ms / p99 8.6 ms, read p50 1.1 ms / p99 5.0 ms. The expiry scheduler
is amortized off the request path; synchronized deadlines are a non-event.

B2 — XDP LPM map saturation. 176k distinct blocks added cumulatively across
slots (56k + 3×30k, bases 160–190), map growing throughout. Batch times
stayed flat at ~2.0 s per 2k adds under soak contention (~1.0–1.2k ops/s;
uncontended round-1 rate was 1.75k/s). xdp_apply_failures_total and
attribution_gaps stayed 0 for the entire growth. The map's ceiling was not
reached — the kernel layer absorbed every add.

B3 — distributed evasion + memory pressure. 875,520 events at 15.9k eps
from a 50k-unique-IP reserved pool, ~17 events per IP (under every gate).
Result: 16 per-IP blocks + 2 subnet blocks out of 875k events (0.002%
false-positive rate), and the CGNAT challenge tier answered 4,097 times —
the tier system responds to sub-threshold floods with challenges instead of
blocks, which is the intended shape. RSS grew 3.4 MB for 50k new unique IPs
(~68 B/IP) and held after settle; HLL +106k inserts, CMS +2.2M increments.

B4 — auth-wall starvation probe. 48 connections spamming valid JSON with
bad signatures at 4,127 rejects/s; a legitimate probe every 250 ms. Probe
p50 19.1 ms / p99 33.3 ms, zero probe failures (idle baseline ~1–4 ms; the
elevation matches the concurrent soak, not the wall). frames_rejected
delta 0 — 401s are decided before the frame parser sees anything. First
attempt hit a harness deadlock (executor join vs stop flag); re-run clean.

B5 — dashboard hammer. 23,920 HTTP requests at ~430 req/s (metrics, api
snapshot, healthz, index) — every one 200, p50 5.1 ms, p99 23.1 ms. 8,964
login attempts with 1 KB passwords, all 404, no lockout pathology, no
degradation of the HTTP path. The dashboard's own IPC client ran 42k calls
during the window.

B6 — stats read storm + audit. 60,145 full-store get_stats reads in 40 s
(~1.5k/s) while soak writers hammered: p50 19.1 ms, p99 62.9 ms, idle
post-storm 1.2 ms — full-store reads cost ~19 ms under write load and the
service settles instantly. WAL: 2 segments, 68 MB against a 1 GB retention
cap. (Harness note: the writer half of this round died on a Python closure
bug — read half is the valid data; write coverage came from the soak.)

## The round's real finding

events_rejected_total went 26 → 416,394 during the concurrent phases
(1.0% of 42.5M ingested). That counter is the bounded event channel's
try_send hitting its watermark — not malformed input, not shed (shed stayed
0; they are different mechanisms). Rejections occurred only while a block
storm (~1.1k enforcement ops/s with kernel LPM work) ran simultaneously
with 16k eps evasion and the soak's 12k eps; the moment contention ended,
rejections froze (soak alone at 12k eps produced zero). ingest_channel_depth
read 0 at the sampler's 3 s granularity, so the spikes were sub-sample.

Interpretation: under extreme mixed load the enforcement path and the
detection workers contend, the event channel fills faster than it drains,
and events drop rather than block the caller. That is a degrade-not-die
behavior and the F2 instrumentation is what made it visible at all — but
it is the one parameter on the unsound side of the ledger after two rounds:
channel capacity vs detection throughput under enforcement contention has
no current knob and no alert. Candidate fixes, in order of cost: expose
channel depth more finely (sub-second sampler or a gauge under the same
name), widen the channel, or give detection workers priority isolation
from enforcement work.

## End state

pid 3595995 alive throughout; errs unchanged at 1 (the pre-setcap line);
RSS 113 → 144 MB (state growth: ~176k block entries + 50k-IP pool +
promotions, flat during idle stretches — no leak signature); pending
expirations 29,386 draining normally; XDP wire_pass 106k with drops,
attribution gaps, apply failures all 0; HMAC, metrics, dashboard 200.

Cumulative over both rounds: 846k block ops, 43M events ingested, 1.73M
requests served, zero crashes, zero leaks, zero XDP failures.

## Correction (post-round-22, C1–C3)

The "one unsound parameter" finding above is **retracted**.
`events_rejected_total` is a CONFLATED counter: it sums auth 401s,
connection refusals, AND channel-full event drops (three call sites in
`src/ipc/server.rs` all bump `inc_rejected`). My B4 auth-wall rounds alone
sent ~300k 401s into that counter. C1 (17k eps + up to 1k blk/s, single-digit
rejections per 10 s phase) and C3 (15k → 45k eps, no writers, **zero**
rejections at every rate) show the 64k bounded channel + pre-aggregation
drains well beyond 45k eps sustained — the channel-full path is effectively
unreachable at realistic IPC rates. The 416k "drop storm" was auth rejections
counted as event rejections.

Fix: commit be51ef1 splits the counter into
`ramshield_ipc_event_drops_total`, `ramshield_ipc_auth_rejections_total`,
`ramshield_ipc_rejected_connections_total`; `events_rejected_total` remains
the aggregate for dashboard/SSE compat. Related: `IpcServerStats`
(`dropped_events`, `channel_capacity`) exists in `src/ipc/server.rs` but is
never called — the clean per-class stats had no consumer at all.

The surviving, smaller finding: detection latency degrades ~10× under
concurrent enforcement load (C2: 199 ms clean vs 2,049 ms under 32 writers +
noise feed) — head-of-line, not a ceiling.

## Harness bugs (mine, fixed or noted)

B2's generated scripts had mangled f-string braces (data valid, prints
cosmetic); B4's first run deadlocked on executor-join-before-stop; B6's
writer closure lacked nonlocal. All lessons folded into
`ramshield-development/scripts/gauntlet_*.py` conventions: no executor
context managers around stop-flagged loops, explicit nonlocal, and
generated code must be read back before queuing.
