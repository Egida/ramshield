# RamShield Development Process

Status: working agreement
Scope: code, metrics, dashboard, trace logs, Docker deployments

## Story: why the wiring grew

RamShield did not become difficult because one algorithm was unusually large. It became difficult because every meaningful operation crossed several contracts:

```
packet / IPC frame
  -> parser
  -> metric owner
  -> atomic counter
  -> snapshot
  -> SSE / Prometheus
  -> dashboard renderer
  -> test fixture
  -> runtime log
  -> deployment capability/config
```

A local fix could therefore create a distant defect:

- IPC parse failures were visible in TRACE but absent from dashboard totals.
- Adding them to `events_rejected` broke pipeline arithmetic because that counter has event semantics while parse failures have frame semantics.
- XDP appeared broken after builds because file capabilities belong to the binary inode and disappeared on every rebuild.
- Dashboard pipeline stages mixed events, packets, IPs, batches, decisions, and applied blocks.
- A green load generator did not prove ingestion; malformed IP fixtures silently rejected whole frames.
- Embedded HTML required a release rebuild before the served dashboard changed.

The lesson: a feature is not wired when its source compiles. It is wired only when ownership, persistence, transport, presentation, tests, logs, and deployment agree.

## Non-negotiable rules

1. Verify before editing.
   - Read the owning constructor, writer, readers, route, config loader, deployment manifest, and test fixture.
   - Query codebase-memory-MCP for the relation path.
   - Query `docs/metrics/metric-keystore.json` for owner, unit, invariant, source refs, and readers.

2. One finding, one change.
   - Do not bundle unrelated dashboard, logging, metrics, and deployment edits.
   - State the finding, evidence, smallest fix, and expected post-fix observation before patching.

3. One metric, one owner, one unit.
   - Counters increment only at the operation site.
   - Gauges sample state only at the sampling site.
   - Never sum or compare values with different units.
   - Every metric needs a keystore entry before publication.

4. Tests must fail before the fix when practical.
   - Add the narrow regression assertion first.
   - Confirm RED.
   - Implement the smallest fix.
   - Confirm GREEN, then run the owning crate suite.

5. Logs are evidence, not decoration.
   - TRACE: per-event branch and rejection evidence.
   - DEBUG: bounded batch summaries and periodic audits.
   - INFO: lifecycle, configuration, bind, mode.
   - WARN: actionable, rate-limited anomalies.
   - Structured fields only; no interpolated hot-path messages.

6. Deployment state is part of correctness.
   - Config flags, environment overrides, Docker image digest, capabilities, ports, schema version, and migration state must be recorded together.
   - Never call a build deployed until the running process reports the expected commit and image/config identity.

7. No blind automation.
   - Automated fixes use a whitelist and dry-run default.
   - Deployments stop on failed contract, schema, health, metric, or rollback checks.

## Verification process before any code change

### Step 0 — freeze the baseline

Record:

```text
git status --short
git log --oneline -5
running PID and listening ports
config path and relevant environment names
image digest, if containerized
active runtime log path
```

Capture:

```bash
curl -fsS http://127.0.0.1:9999/healthz
curl -fsS http://127.0.0.1:9999/api/snapshot
curl -fsS http://127.0.0.1:9999/api/status/modules
```

Stop here if another daemon owns the ports or the baseline identity is unknown.

### Step 1 — map the relation

Use codebase-memory-MCP:

```bash
codebase-memory-mcp cli index_repository --repo-path "$PWD" --mode full
codebase-memory-mcp cli get_architecture --project <project> --aspects all
codebase-memory-mcp cli get_graph_schema --project <project>
```

Trace:

```text
writer -> metric atomic -> snapshot -> route/SSE/Prometheus -> dashboard/test
```

Check co-changing files and deployment consumers before touching the writer.

### Step 2 — reconcile the keystore

For the finding, confirm:

```text
metric ID
owner module
exact writer
unit
invariant
zero semantics
log events
source references
readers
executable test reference
```

Run:

```bash
python3 scripts/validate_metric_keystore.py docs/metrics/metric-keystore.json
python3 scripts/export_metric_keystore.py
```

A missing relation is a finding. Do not patch code to hide it.

### Step 3 — define the contract

Write one short change record before editing:

```text
Finding:
Evidence:
Owner:
Unit:
Expected invariant:
Smallest change:
Files allowed:
Regression check:
Runtime observation:
Rollback:
```

Keep the record beside the change or in the issue/commit body. No unbounded scope.

### Step 4 — prove the test boundary

Choose the narrowest check:

- parser: malformed and valid frame test
- metric: writer/read-path assertion
- dashboard: live SSE key and DOM consumer check
- log: trace stimulus plus audit rule
- deployment: container health, config identity, capability check

For a new behavior, make the check fail first. If RED is impractical, record why.

### Step 5 — edit minimally

Patch only allowed files. Re-run the codebase graph query if the edit changes a public field, route, metric, trait, config key, or deployment value.

For embedded dashboard HTML:

```text
source edit -> release rebuild -> capability restore -> daemon restart -> curl served HTML
```

Source inspection alone is not verification.

## Verification after each change

Run in this order:

```bash
cargo fmt --all -- --check
cargo check --workspace --locked --all-targets --features full
cargo clippy --workspace --all-targets --features full -- -D warnings
cargo test --workspace --locked --features full
python3 scripts/validate_metric_keystore.py docs/metrics/metric-keystore.json
python3 -m py_compile scripts/*.py
```

Then verify runtime:

```bash
sudo setcap 'cap_net_admin,cap_bpf,cap_perfmon+eip' target/release/ramshield
getcap target/release/ramshield
scripts/run_trace_logged.sh "<finding>"
python3 scripts/trace_coverage_probe.py
python3 scripts/metrics_smoke.py
python3 scripts/log_audit.py --log <trace-log>
```

Require:

```text
one daemon
healthz OK
expected commit identity
expected xdp_active mode
metric changed under intended stimulus
metric unchanged under unrelated stimulus
SSE field present
Prometheus field present
dashboard field consumes the same key
trace branch present
no unexpected ERROR/panic/clobber/unknown reason
```

## Docker image and metric database process

Treat the Docker image plus the metric keystore as one deployable contract.

### Build artifact

Record in the image metadata:

```text
source commit
image digest
Rust/toolchain version
config profile
metric-keystore version/hash
NoSQL schema version/hash
build timestamp
```

Do not use a mutable tag as deployment identity. Resolve the digest before rollout.

### Metrics database

Use the keystore as the registry, not as free-form documentation. Store/import:

```text
metric ID
owner and writer
unit and type
labels
invariants
zero semantics
source refs
reader refs
schema version
first/last compatible image digest
```

Schema changes require one of:

```text
backward-compatible additive field
explicit migration
versioned replacement
```

Never silently rename a metric. Keep an alias during migration or fail validation.

### Automated deployment gate

A deployment proceeds only when all pass:

1. image digest is immutable and recorded;
2. keystore and NoSQL schema validate;
3. source commit matches image metadata;
4. config parses and required overrides are recognized;
5. container health succeeds;
6. exactly one expected listener exists;
7. `/api/snapshot`, `/api/stream`, `/metrics`, and `/healthz` respond;
8. smoke traffic changes intended counters;
9. trace audit has no high-severity finding;
10. rollback image is known and runnable.

Failed gate means no rollout. Preserve logs and the failed contract ID.

## Edit tracking

Every change leaves these artifacts:

```text
commit: one atomic behavioral change
keystore diff: metric contract change, if applicable
change record: finding/evidence/verification/rollback
runtime log: exact trace or debug path
verification report: commands and exit results
image metadata: digest/config/schema/commit, if deployed
```

Commit format:

```text
<area>: <single behavioral change>

Finding: <short evidence-backed statement>
Contract: <metric/unit/path affected>
Verified: <focused test>; <runtime check>; <log audit>
Rollback: <commit or image digest>
```

Do not amend old commits. Do not reset hard. If automation creates an unrelated commit, stop it before continuing.

## Change ledger template

Keep one ledger entry per finding:

```yaml
id: OBS-YYYYMMDD-NNN
finding: ""
evidence:
  graph_path: ""
  keystore_metric: ""
  runtime_log: ""
  live_payload: ""
owner: ""
unit: ""
files_allowed: []
red_check: ""
fix_commit: ""
verification:
  cargo: ""
  keystore: ""
  runtime: ""
  dashboard: ""
  log_audit: ""
image_digest: ""
rollback: ""
status: open
```

This process prevents circular debugging: first map the relation, then define the contract, then make the smallest edit, then prove source, runtime, dashboard, logs, and deployment agree.
