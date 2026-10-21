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

## The shared-memory rule table (C ABI)

`include/ramshield_shm.h` publishes a memory-mapped rule table that a native
reverse proxy reads directly. The Rust side of this crate **writes** the
table (`ShmTableManager::publish_rule`); it does not read it. The C header
is therefore the only rule-lookup path, and its consumer is out of repo.

### Reading a rule

```c
RamshieldShmRuleSnapshot snap;
RamshieldShmLookup r = ramshield_shm_read_ex(table, hash, now_ms, &snap);
```

| result | meaning |
|---|---|
| `RAMSHIELD_SHM_FOUND` | a coherent, live rule was copied into `snap` |
| `RAMSHIELD_SHM_MISS` | the lookup completed coherently and no live rule matched |
| `RAMSHIELD_SHM_CONTENDED` | the lookup could not obtain a coherent snapshot within the bounded retry budget |
| `RAMSHIELD_SHM_INVALID` | the input was rejected before any slot was read |

### Release note

The bool wrapper's signature and its "true only on a match" behaviour are
unchanged. Two error cases differ from the previous release, both in the
old code's favour:

- an invalid prefix (0, or above 32) is no longer looked up as the
  reserved key 0, so it can no longer find a rule stored under that key;
  and
- a lookup for the reserved key 0 itself no longer finds a rule. The old
  reader matched `client_hash == hash` with `hash == 0`, so it could report
  a rule stored under the reserved key.

The whole probe window is scanned, exactly as before. The old comment said
"empty terminates the probe", which did not describe the code: its `break`
ended the inner retry loop, and the probe continued to the next slot.

### The consumer contract (integration requirement)

The proxy that reads this table is **outside this repository**, so the Rust
and C tests cannot prove how it reacts. The contract it must implement:

| result | required interpretation |
|---|---|
| `FOUND` | a coherent matching entry was read |
| `MISS` | a valid lookup completed and no matching entry was found |
| `CONTENDED` | no reliable conclusion about presence was reached within the retry budget |
| `INVALID` | the request cannot be processed as a valid lookup |

For `CONTENDED` the consumer must not take a security-sensitive action that
assumes the entry is absent. It should use an explicitly documented retry,
defer, or conservative policy appropriate to its role. Treating it as a
miss is the one response that is always wrong.

The legacy `ramshield_shm_read()` collapses `CONTENDED` and `INVALID` into
`false`, so the ambiguity remains for a caller that uses it. That is
accepted for compatibility, and it is why the explicit-result form exists.
New security-sensitive callers should use `ramshield_shm_read_ex()`.

**`CONTENDED` is not a miss.** It means the slot could not be read
coherently — a writer held it, or its sequence counter changed on every
attempt — so the rule may well be there. A proxy that treats `CONTENDED` as
"no rule" fails **open**: it stops challenging or blocking a client the
table does hold a rule for. Handle it explicitly.

`ramshield_shm_read()` is the older boolean form and is kept for
compatibility. It returns `true` only on a match, so it cannot distinguish
a miss from contention — the behaviour it has always had. New code should
use the `_ex` form.

### Prefix lengths

`ramshield_shm_read_ipv4_ex()` rejects two classes of prefix **before** it
reads any slot:

- `prefix_len > 32` — not an IPv4 prefix.
- `prefix_len == 0` — a /0 prefix masks to network 0, which hashes to key
  0, and key 0 is the reserved empty-slot sentinel. A rule for it was
  published into a slot every other publisher treats as free, so the next
  colliding publish overwrote it; and the lookup said nothing was wrong
  with the input. The lookup now reports it as `INVALID` instead of
  returning a silent miss, and the publisher refuses the same key.

  The reserved key is not only the /0 case: the slot-key mixer is
  0-preserving, so a key is 0 exactly when the masked network address
  equals its prefix length — every /0, and `0.0.0.32/32` among others.

The representable range is 1..=32.

### Slot keys

`ramshield_subnet_key()` (C) and `subnet_key()` (Rust) must produce the
same key for the same masked network, or a rule published by the Rust
writer lands in a slot the C reader never looks in. Both sides pin the same
vectors: `c_and_rust_slot_key_hash_agree` in `src/shm.rs` and
`test_key_parity_with_rust` in `scripts/shm_lookup_contract.c`. A change to
either implementation breaks one of the two.

### The cost of a lookup

The table is the reverse proxy's lookup path, so the cost of a colliding
key is a property of the hot path.
`scripts/shm_lookup_bench.c` measures it — the cost of a lookup by the
probe distance of its rule, and the cost of a mixed hit/miss workload —
and checks every answer while it does so.

Measured on the development host (i7-3687U @ 2.10 GHz, gcc -O2, best of 5
repetitions of 200 000 lookups):

| rule's probe distance | slots scanned | ns/lookup |
|---|---|---|
| 0 | 1 | 25.7 |
| 1 | 2 | 29.1 |
| 2 | 3 | 32.4 |
| 3 | 4 | 36.3 |

Mixed workload (4096 rules, hits and misses over a 4096-key working set):
**54.3 ns/lookup**. Each extra slot in the probe window costs about
3.5 ns, so the full-window scan — which the reader has always done — costs
about 10 ns more than a single-slot lookup.

These figures are for that host. A cost figure may only be published with
the workload and the machine recorded.

### Testing

`scripts/verify_shm_header.sh` (the `shm-abi` gate) compiles and runs:

- `scripts/shm_abi_stress.c` — the C ABI layout asserts plus a concurrent
  reader/writer stress that proves a reader never observes a torn rule.
- `scripts/shm_lookup_contract.c` — the deterministic contract tests: a hit,
  a coherent miss, an expired rule, a permanently odd sequence, a changing
  sequence, the prefix boundaries, and the key parity.

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