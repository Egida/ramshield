# RamShield metric keystore

Canonical metric graph. One document per metric, with ownership, write paths, readers, invariants, zero semantics, log evidence, and tests.

## Files

- `metric-keystore.json`: source registry.
- `nosql-schema.json`: document validation contract.
- `metric-keystore.jsonl`: generated import form; one module/metric/relation/evidence document per line.
- `../audits/`: historical narrative audits; not canonical ownership data.

## Query chain

`metric ID → owner module → writer path → log event → SSE/Prometheus reader → dashboard field → test/evidence`

Examples:

```bash
python3 scripts/validate_metric_keystore.py
python3 scripts/export_metric_keystore.py
python3 -m json.tool docs/metrics/metric-keystore.json >/dev/null
jq 'select(.collection == "metrics" and .owner_module == "mesh")' docs/metrics/metric-keystore.jsonl
jq 'select(.collection == "relations" and .from == "xdp")' docs/metrics/metric-keystore.jsonl
```

## Document model

`modules` describe runtime owners. `metrics` are canonical logical signals. `relations` form the graph. `evidence` names commands or runtime artifacts that prove invariants.

Metric kinds:

- `counter`: monotonic event total; only the event owner increments it.
- `gauge`: current state; periodic sampling may overwrite it.
- `rate`: derived from counter deltas; never written as a source counter.
- `snapshot`: point-in-time object.
- `histogram`: latency/distribution data.
- `derived`: arithmetic combination; source metrics must be relations.

Security/cardinality rules:

- No IP, token, payload, or user ID as a metric label.
- Raw IP may appear only in sampled operational events where existing debugging requires it; never in Prometheus labels.
- Logs prove transitions. Counters prove volume. Do not reconstruct counters from log line counts.

## Current known gaps

The registry deliberately records gaps rather than inventing writers:

- CGNAT and analytics crates have zero direct tracing sites; their aggregate metrics are written by detection.
- `cms_decay_ticks`, `shm_lookup_count`, and `shm_cache_hits` have metric fields but dead writer paths; they are not fabricated into this initial ownership graph.
- `blocks_detection` and `blocks_subnet` mean commands sent to enforcement; `enforcement.blocks_total` means applied. They are separate metrics.
- `xdp.*` counters describe kernel wire traffic. IPC traffic cannot increment them.
- Mesh HLC/purge zeros are valid when mesh/coordinator wiring is disabled.

## Update rule

Every new metric requires: one owner, one writer path, at least one reader, one invariant, zero semantics, source references, and executable evidence. Run the validator and exporter before commit.
