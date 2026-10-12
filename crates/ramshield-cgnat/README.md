# ramshield-cgnat

## Why it exists
Dedicated CGNAT (Carrier-Grade NAT) classification for shared-infrastructure protection using RFC 6598 address space (100.64.0.0/10). The guard enables detection/classification of shared-subnet traffic for throttling, challenge, or blocking.

## How it works

- `CGNAT_TIER_ALLOW` (0) — no special handling
- `CGNAT_TIER_CHALLENGE` (1) — rate above threshold suggests shared-subnet with probe traffic
- `CGNAT_TIER_BLOCK` (3) — high rate or host density indicates heavy shared-infrastructure use

The key entry point is `CgnatGuard::classify_subnet(subnet, host_count, rate)`:

| condition | tier |
|---|---|
| not shared infra + host_count > 64 + rate > 50_000 | `Block` |
| shared infra + rate > 100_000 | `Challenge` |
| rate > 5_000 (any) | `Challenge` |
| otherwise | `Allow` |

## Uniqueness
RFC 6598 detection is combined with rate + host-density checks — not just address-range membership. The guard intentionally limits scope to subnet-level classification; full CGNAT mapping and lease tracking live in higher layers.

## Dependencies
`ramshield-types` (IpAddr, Ipv4Addr); no `ipnet` dependency. Uses `std::net` types only.

## Benchmarks
```bash
cargo bench --bench hot_paths --features full -- cgnat_classify
```

## Testing
All CGNAT classifier logic is unit-tested in `crates/ramshield-cgnat/src/cgnat.rs` — classifier test exercises shared-infra density and rate thresholds.