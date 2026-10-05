# Threat classes (claims vs coverage)

This matrix is the product contract for detection. Anything not listed as
**Supported** must not be marketed as blocked.

| Class | Status | Mechanism | Test / artifact |
| --- | --- | --- | --- |
| Loud absolute RPS | Supported | EWMA ≥ `rps_threshold` (debounced hot) | unit + integration |
| Sustained sub-threshold drift (loud baseline) | Supported | CUSUM vs absolute allowance | `rate_tracker` / CUSUM tests |
| Pulse wave (short bursts) | Supported | pulse window samples | pulse tracker tests |
| Subnet /24 swarm | Supported | subnet batch gates | Subnet Stress Pin CI |
| Cardinality flood | Mitigated | cold-skip + bloom + RAM budget | capacity / bloom tests |
| Low-and-slow relative (N× baseline, absolute ≪ threshold) | **Opt-in** | `relative_enabled` gate on promoted records | `relative_enabled_catches_low_and_slow` |
| Multi-node coordinated | Experimental | mesh crate | not production-primary |

## Relative gate (0.3.4+)

Default **off** so existing deployments keep absolute-only behavior.

When `relative_enabled = true`:

```text
need = max(relative_floor_rps, relative_factor * baseline_rps)
fire if sample_count >= relative_min_samples AND inst_rps >= need
```

Constraints:

- Runs only on **promoted** IPs inside `merge_record` (after cold-skip).
- No per-IP tracker DashMap on the flush path.
- Soak against CGNAT/CDN sawtooth before enabling in production.

See also `docs/SMALL_SCALE_DETECTION.md` for the verified absolute-only gap.
