# Attack gauntlet, round 1 (2026-09-22)

Sustained improvised assault on the LIVE production instance: 10 strategies
over ~10 minutes, each round designed while the previous one ran, attack
sources confined to reserved ranges (198.51.0.0/16, 203.0.113.0/24,
192.0.2.0/24) so nothing legitimate could be blocked. XDP was attached to
the real uplink (wlp2s0, skb mode) for the whole run; HMAC auth was on.

Method: a queue-driven driver runs each strategy under a 62 s cap and
rescans the queue before every step, so the next attack is authored while
the current one runs. A 3 s sampler records the full counter set to
`/tmp/atk/metrics.tsv`. Harness + discipline live in the
`ramshield-development` skill (`references/attack-gauntlet.md`,
`scripts/gauntlet_*.py`).

## Results

| # | strategy | load | outcome |
|---|----------|------|---------|
| A1 | write storm, 64 conns | 52,930 block_ip | 962/s · p50 59.7 ms · p99 193 ms · 0 failed |
| A2 | pipelined reads, 16 conns | 76,800 check_ip | 18,827/s · 0 lost · 0 failed |
| A3 | detection flood, malformed events | 10,432 frames | all rejected as typed parse errors at ~1k/s (`frames_rejected_total` climbed); service unaffected |
| A4 | XDP map churn, 32 conns | 48,064 block+unblock cycles | 874 cycles/s = 1,748 ops/s · `apply_failures_total` 0 · `attribution_gaps` 0 |
| A5 | unsigned protocol abuse | 13 cases | every case 401 at 0 ms; 40 MB line → connection reset in 77 ms |
| A6 | real detection flood | 3.28M events @ 59,695 eps | 240 auto-blocks (238 per-IP + 2 subnet) · CGNAT tiers 26 block / 73 challenge / 0 allow · 11,412 promotions · `events_shed_total` 0 · `ingest_channel_depth` 0 |
| A7 | signed-but-broken envelopes | 12 cases | 12/12 rejected at the auth gate |
| A8 | semantic abuse, authed | 13 cases + 200k-event frame | 13/13 typed errors (code 1 parse / 400 semantic), 0 panics; 10 MB frame → reset in 2.2 s, RSS +164 KB |
| A9 | all three paths at once | 26.5k writes + 13.2k reads + 2.2M events | 0 failures anywhere · read p50 27.9 ms / p99 117.6 ms · +251 blocks · pristine-connection probe answered in 12–67 ms every 5 s |
| A10 | low-and-slow, 200 IPs over 4 /24 | 382k events @ 6,959 eps | **zero blocks** — no false positives |

## Cross-round state

- RSS: 89 → 113 MB over the run. The entire delta maps to new state
  (+75k blocked IPs, 5.9M ingested events). During rounds that add no new
  state (A5–A8) RSS is flat — no leak signal.
- `events_shed_total` never moved (0), `enforcement_dropped_total` 0,
  `xdp_apply_failures_total` 0, `attribution_gaps` 0, log ERROR count
  unchanged (1 pre-existing line from before setcap).
- Post-run: pid alive, idle `check_ip` back to 3.9 ms.

## Parameter findings

1. **Concurrency sweet spot ≤ 32 clients on the enforcement path.**
   A1 (64 conns): 962 ops/s, p50 59.7 ms. A4 (32 conns): 1,748 ops/s,
   ~10 ms/op. Beyond ~32 concurrent writers total throughput *drops* —
   queueing and lock contention, not XDP cost. The single enforcement
   consumer saturates near 1.7–2k ops/s.
2. **Read path ≈ 20× the write path** and scales with connections
   (18.8k/s pipelined reads; p50 halved when writes were off the path).
3. **Detection ingest headroom:** 60k eps with zero shed and zero channel
   depth — backpressure machinery never engaged at this scale.
4. **Detection works under fire:** 491 auto-blocks total across the run
   (240 at peak in A6, 251 in A9), subnet tier fired 4 times, CGNAT
   classify ticks 99 — all without touching the read or write paths'
   failure counts.
5. **Layering is load-bearing:** the auth gate rejects unsigned garbage
   before the parser (0 ms cost), signature verification canonicalises the
   non-auth JSON so byte-level malformation can never reach the parser,
   `max_line_length` (33 MB) and the per-connection byte cap (1 MB) are
   enforced by connection reset rather than buffering.
6. **No false positives** on distributed sub-threshold traffic (A10):
   200 IPs at ~8 eps each, four /24s — 382k events ingested, zero blocks.

## Known boundaries (from the run, not claims)

- 60k eps is the top of what this harness can push over one loopback IPC;
  ingest headroom above that is untested.
- The block path at 1.7k ops/s assumes XDP attached (kernel LPM update per
  block); scratch measurements without XDP show ~2.2k ops/s.
- XDP drop counters stayed 0 by construction (reserved-range sources never
  send real packets); the zero-drop attribution path was exercised in the
  suite's `xdp` layer separately.

Round 2 follows in the same harness (see
`references/attack-gauntlet.md` in the ramshield-development skill).
