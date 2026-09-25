# Capacity envelope (measured)

**Not guessed.** Source: `docs/DDOS_BENCHMARK_REPORT.md` (2026-09-05, laptop-class, `master@61b9a78`, XDP skb on `lo`, HMAC IPC). Re-run on target hardware before quoting these as a sales SLO.

## Hardware class

| | |
|---|---|
| Host | Linux 6.8 laptop |
| Binary | `./target/release/ramshield` stripped ~7 MB |
| RSS idle | 44 MB |
| XDP | skb/generic on `lo` |
| IPC | HMAC-SHA256, 1 key |

## Measured points

| Workload | Result | Notes |
|---|---|---|
| Raw IPC throughput, 10s | **135,602 eps** | T11; 87 errors |
| Warm detect→mitigate | **108 ms** | T20 PASS |
| Cold detect→first block | **~8 s** | 1 full window; T20/T8 |
| Unblock RTT (10) | **52 ms** | T15 PASS |
| Unblock RTT (50) | **8.8 ms** / 50 | T10 |
| Benign FPR | **0 / 200 IPs** | 0.0000% in cited suite |
| Events in campaign | **21.3 M** | RSS stayed ~44 MB |
| RAM growth | **0.0004% / 1M events** | bounded store |
| Background EPS under attack | **−93.5%** | RFC 9411 §A — HEAVY IMPACT |

## What this is *not*

- Not a 2/4/8 vCPU matrix. That hardware was not run.
- Not a legitimate-traffic p99 under mixed attack. The −93.5% figure is the honest stand-in until a mixed-load bench exists.
- Do not encode these numbers as hard limits in source.

## Reproduce

```bash
cargo build --release --features full
# then the v2/v3 suites referenced in DDOS_BENCHMARK_REPORT.md
CFG=./config.prod.toml ./scripts/prod_smoke.sh
python3 scripts/enf_stress.py   # WAL kill/restart among other drills
```

## Operator takeaway

On this class of host: tens of MB RSS, ~135k IPC eps peak, warm mitigate ~100 ms, **legitimate throughput collapses under attack** until the serialization point is profiled. Treat −93.5% as the capacity warning, not 155k as the headline.
