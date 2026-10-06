# Threat classes (claims vs coverage)

This matrix is the product contract for detection. Anything not listed as
**Supported** must not be marketed as blocked.

| Class | Status | Mechanism | Evidence |
| --- | --- | --- | --- |
| Loud absolute RPS | Supported | EWMA threshold + debounce | detection tests |
| Sustained sub-threshold drift | Supported | CUSUM | rate-tracker tests |
| Pulse wave | Supported | pulse correlation | pulse tests |
| Subnet /24 swarm | Supported | subnet batch gates | subnet tests |
| Cardinality flood | Mitigated | cold-skip + bloom + RAM budget | capacity tests |
| Low-and-slow relative | **Experimental / opt-in** | prior slow baseline + maturity + breach streak | relative detector tests; soak required |
| Multi-node coordinated | Experimental | mesh | not production-primary |

## Relative detector contract

Relative detection is deliberately **off by default**. When enabled, a promoted
record is eligible only after `relative_min_samples` observations. The current
sample is compared against the **prior** slow baseline, then the baseline is
updated. A breach must persist for `relative_min_breaches` consecutive samples.

```text
prior_baseline
      ↓
max(relative_floor_rps, relative_factor × prior_baseline)
      ↓
current sample
      ↓
consecutive breach streak
      ↓
block decision
      ↓
slow baseline update
```

`relative_factor` and `relative_floor_rps` must be finite. `relative_min_samples`
is limited to the representable `IpRecord::sample_count` range (1..=255).

This detector remains experimental until false-positive soak testing covers the
intended deployment population (including CGNAT/CDN sawtooth workloads).
