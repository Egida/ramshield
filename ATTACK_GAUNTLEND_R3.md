# Attack Gauntlet — Round 3 (genuine-classes phase, D1–D11, 2026-09-22)

After rounds 1–2 (A/B, throughput + hardening) and round 3-C (C1–C3,
drain-ceiling), this phase runs **genuinely new attack classes** — one
hand-authored round at a time, improvised from live metrics — until the
new classes dry. No re-runs, no padding. Prod restarted on `be51ef1`
(the metrics-split binary) at the start; campaign infra: driver +
sampler + 60s monitor under `/tmp/atk/`.

All sources are RFC5737 / RFC3849 reserved (198.51.100/24,
203.0.113/24, 192.0.2/24, 2001:db8::/32). Attacks hit live prod over
HMAC-authenticated IPC end-to-end. `get_stats` is a *lightweight*
snapshot (`blocked`, `ips_tracked`, `ram_bytes`); the per-IP truth is
`check_ip` + the prometheus counters (`/metrics`, **1s render cache**).

## D-rounds

| # | class | headline result |
|---|-------|-----------------|
| D1 | CIDR LPM nesting | 330 cycles, nested-prefix overlap both orders, `apply_fail=0`, longest-prefix correct, `active_cidr 5→3`, +12 KB state |
| D2 | IPv6 path | parser+LPM+attribution all work, 1,276 v6 blk/s, `/64` containment correct (cosmetic: reason names `/32` ancestor) |
| D3 | WAL durability cost | 74,501 blocks @ 1,863/s (campaign high), **44 B/block**, flat curve, no rotation — durability not the bottleneck |
| D4 | slowloris | 400 half-open conns held 8s — probe 3ms, fresh conns 50/50; threshold = `max_connections`, unreachable from one box |
| D5 | detection timing | pulsing (1s on/off) + ramp both caught by `high_rps`; `blocks_forecast` did not move under these shapes |
| D6/D6b | config-mutation abuse | CSRF 403 on cross-origin, placeholder-rejection (P2) 400 on every shape, merge-key-free bodies = idempotent no-op stores, config intact; 3 stacked harness bugs created a false P0 (no product defect) |
| D7 | unblock storm + TTL semantics | `ttl=0` → **100/100 permanent** ✓; `ttl>1yr` (`2^32-1`) → **typed rejection at IPC boundary** (`MAX_TTL_SECS=31,536,000`, fail-closed, no side effects) ✓; manual block/unblock ~250 ops/s @16 workers |
| D8 | forecast path (concentrated) | 625k events in **one /24** — the subnet-sweep tier fired (`cidr_block(192.0.2.0/24)`) in <2s; forecast stayed 0; `blocks_detection=0` (separate `blocks_subnet` counter) |
| D8b | forecast slow-ramp | **forecast fires for the first time**: 8 preemptive blocks, H2 SLOW-RAMP conf 0.82→0.97 then H1 VOLUMETRIC conf 0.88, `pre-emptive blocks: 2/4/2`; 55 also via `high_rps`; 1 sample reason=`forecast_anomaly` |
| D9 | IPC desync / replay | 160 half-frame + 80 CRLF + 160 pipelined, **0 wrong**; probes p50 0.7/p99 5.4ms; **replay protection live** (identical re-signed frame → 401 `unauthorized: replay`, `ipc_auth_rejections` +80) |
| D10 | event resubmission | **no event-level dedup** — 10× re-signed identical batch counted 10× → all 20 IPs blocked `high_rps` (~20,000 eps inflated vs ~2,000 logical) |
| D11 | restart under load | SIGKILL mid-storm → relaunch: kill 0.76s, **18.3s to ready**; **8/8 last storm blocks survived** (WAL replay), signed auth live, unsigned→401, XDP re-attached, counters reset, errs=0 |

## Forecast path — the "unmatched design" made visible (D8/D8b)

The Bayesian forecaster is a real, layered, preemptive actor, not
cosmetic. It classifies the attack and acts *before* the deterministic
tiers:

- **Input axes:** global RPS (EWMA + Holt-Winters forecast → residual →
  z-score, CUSUM), per-IP threat sample, entropy Δ across 256 subnet
  windows, peak reservoir (self-calibrated extreme quantile).
- **Threat formula** (per IP): `threat = 0.7*rps_score + 0.3*err5xx`
  (the 5xx status bucket only). Clean feeds cap at `threat ≤ 0.7` — so
  the preemptive block effectively needs elevated 5xx fraction.
- **Hypothesis test:** Bayesian update over H0 baseline / H1 Volumetric
  / H2 SlowRamp with priors; the matching hypothesis gates
  `preemptive_block(threat_sample)` (block IPs with `threat > 0.7`,
  source `forecasting`, reason `forecast_anomaly`), plus a legacy
  z>3.0 + extreme-quantile fallback.
- **Why my early bursts never hit it:** (a) a single-IP attack fails
  the `n ≥ 10 unique IPs` tick gate; (b) a concentrated /24 burst is
  caught by the faster subnet-sweep tier; (c) a ramping single-IP is
  caught by `high_rps` first. The forecast's lane is a **distributed,
  multi-IP, moderate-rate ramp** — exactly what D8b shaped (60 IPs over
  20 /24s, 150→700 eps, 40% 5xx). It fired: H2 confidence climbed
  0.82→0.97 across the ramp, flipped to H1 at the top, then
  `pre-emptive blocks` fired in 3 ticks.
- **Layered result (D8b):** `blocks_forecast_delta=8`,
  `blocks_detection_delta=55`, `blocks_subnet_delta=0`. The two
  independent layers each caught its own share — the forecaster is the
  slow-preemptive backstop, `high_rps` is the fast per-IP gate.

## D10 finding (documented, no code change)

Detection is purely **count-based; there is no event-level dedup**. A
re-signed identical batch counts 10× → inflated per-IP rate → false
`high_rps` blocks on the replayed IPs. Scoping: any key-holder can
already fabricate events, so the marginal harm of *replay* is nil for
the single-trust-domain model; the note stands as a
**warn-when-you-extend-trust** — if multi-tenant / cross-trust reporting
is ever added, event-level dedup becomes required.

## D11 operational fact

Startup goes offline ~18s: WAL replay of ~1.65M entries ≈ 9s, then XDP
re-attach + config. `WAL replay: restored 380 live blocks (21 with TTL)`
— the large campaign state is mostly expired/superseded by replay time;
replay is idempotent (ran twice, same result). Counters reset to 0 on a
fresh process by design.

## Honest close

Genuinely-new classes covered: CIDR nesting, IPv6, WAL cost, slowloris,
detection timing, config-mutation, unblock+TTL semantics, forecast
path, IPC desync/replay, resubmission dedup, restart-under-load.
Remaining thin candidates (low information value, not run): bloom decay
under long churn, detection head-of-line re-test, XDP map saturation.
The decisive open item surfaced by this phase is the D10 trust-boundary
note, not a capacity defect — consistent with the campaign's overall
verdict: RamShield's dataplane is non-blocking by construction and every
failure class is metered; the only defects found across ~30 rounds were
in the harness's own counter/message semantics, now split
(`be51ef1`) and documented.