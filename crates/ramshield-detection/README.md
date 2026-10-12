# ramshield-detection

Consumes bounded connection telemetry, aggregates events, evaluates configured
detection signals, and emits enforcement decisions. It does not own the final
block state or the XDP dataplane; those belong to `ramshield-enforcement`.

## Processing model

The engine pre-aggregates events into bounded per-IP and subnet state, then
flushes batches on configured time and size limits. The configuration schema is
`DetectionConfig` in `ramshield-config`; defaults and validation live there.
Do not copy threshold values into deployment configuration without checking
that schema and the deployed version.

The detector includes these signal families:

- **Rate / EWMA:** compare observed per-IP rates with a smoothed baseline and
  configured absolute threshold.
- **CUSUM:** accumulate sustained deviation over samples; the warm-up and
  allowance are part of the detector logic.
- **Pulse correlation:** count distinct over-threshold observations in a
  sliding window. Both the window and sample threshold are configurable.
- **Subnet batch:** require both a unique-IP threshold and an event-volume
  threshold for a subnet batch decision; a single high-volume address is not
  equivalent to a distributed swarm.
- **Relative baseline:** opt-in only, applies to promoted IP records, and has
  sample, factor, floor, and consecutive-breach controls. It must be qualified
  against representative traffic before use.
- **Application telemetry:** optional bounded L7 and HTTP/2 metadata signals
  are evaluated only when enabled and when producers provide the fields.

The emergency per-IP burst path may emit an in-flight decision before a normal
batch flush. Batch size, pre-aggregation limits, and flush intervals bound
memory and latency. They are operational controls, not promises of a particular
throughput on every host.

## Operating safely

- Treat detector output as a policy input. Choose thresholds and TTLs for the
  protected service and its shared-egress / CGNAT characteristics.
- Keep relative detection disabled until its baseline behavior is qualified.
- Start with representative replay or staging traffic; monitor false positives,
  batch pressure, queue drops, and enforcement projection health.
- Do not infer kernel packet drops from a detector decision. Check the
  enforcement result and XDP health separately.
- The detector works on events supplied by its configured telemetry path. It
  is not a general-purpose HTTP parser or a substitute for an upstream filter.

See `docs/OPERATIONS.md` for runtime checks and `docs/DEFENSE_MODEL.md` for the
supported telemetry and protocol boundaries.
