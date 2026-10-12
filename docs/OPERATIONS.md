# Operations

Daily operator commands and the small set of checks that matter when RamShield is running.

## Check the process and protection state

Health endpoint:

```bash
curl http://127.0.0.1:9999/healthz
```

CLI status:

```bash
ramshield-cli status
```

JSON status:

```bash
ramshield-cli status --json
```

The important distinction is:

```text
process running != kernel enforcement active
```

When XDP is enabled, check the reported XDP state before treating the host as kernel-enforced.

## Inspect traffic and blocks

```bash
ramshield-cli stats
ramshield-cli check <ip>
ramshield-cli info <ip>
```

List active blocks through the dashboard API:

```bash
curl http://127.0.0.1:9999/api/blocks/active
```

Block and unblock manually:

```bash
ramshield-cli block <ip> --reason manual --ttl 300
ramshield-cli unblock <ip>
ramshield-cli unblock-cidr <cidr>
```

Use manual blocks deliberately. They enter the same enforcement path as other block commands.

## Inspect the dashboard API

The dashboard currently exposes:

```text
/healthz
/metrics
/api/snapshot
/api/stream
/api/history/batches
/api/history/blocks
/api/blocks/active
/api/traffic/subnets
/api/status/modules
/api/config
```

Authenticated dashboard routes should be treated as administrative interfaces.

## Logs

Run the daemon with a normal info level:

```bash
RUST_LOG=ramshield=info ./target/release/ramshield --config config.toml
```

For debugging:

```bash
RUST_LOG=ramshield=debug ./target/release/ramshield --config config.toml
```

Look for:

- XDP load/attach failures;
- WAL open/replay failures;
- enforcement queue pressure;
- detection errors;
- shutdown messages;
- authentication or authorization failures.

## WAL

Check the configured WAL directory:

```bash
ls -la /var/lib/ramshield/wal
df -h /var/lib/ramshield/wal
```

When WAL is enabled, block state is replayed on startup. A running process does not by itself prove WAL durability.

## Restart

After a restart, verify:

```bash
curl http://127.0.0.1:9999/healthz
ramshield-cli status
```

If persistent blocks matter, also verify the expected block with:

```bash
ramshield-cli check <ip>
```

and inspect the WAL/logs if recovery did not behave as expected.

## Upgrade

For a release upgrade:

1. Read the release entry in `CHANGELOG.md`.
2. Keep the existing config and WAL.
3. Stop the current service/process.
4. Install or build the new version.
5. Validate startup using the normal config.
6. Check health and status.
7. Verify an expected existing block if persistence is required.
8. Only then reopen normal traffic.

Do not assume WAL/config compatibility across releases unless the release notes say so.

## Rollback

Rollback to the previously verified version/config pair.

Keep the previous WAL and logs until the new version is verified or the rollback decision is complete.

After rollback, check health, status, authentication, and representative block/unblock behavior.

## Important operator rule

Do not report "protected" from process liveness alone.

At minimum, verify:

```text
daemon is healthy
XDP is active when kernel enforcement is required
persistence is healthy when restart durability is required
```

## Deployment assumptions

- **Telemetry:** detection only sees events delivered by the configured ingest
  path. Verify the proxy/application producer and its bounded event fields;
  source support alone does not mean telemetry is connected.
- **Durability:** enable and qualify WAL if blocks must survive restart. Without
  WAL, do not claim WAL-backed recovery. Keep WAL storage writable, sized, and
  monitored; verify replay after upgrades in staging.
- **Kernel enforcement:** XDP requires a supported Linux kernel, interface,
  driver mode, permissions, and available map capacity. If kernel packet drops
  are required, configure startup to fail when XDP cannot attach and verify the
  reported protection state after every restart.
- **Management network:** HMAC authenticates IPC but does not encrypt it. Keep
  management listeners on trusted networks or terminate TLS/mTLS at a controlled
  proxy. Dashboard cookie security flags do not encrypt HTTP traffic.
- **Capacity and policy:** detection thresholds, memory limits, telemetry rate,
  and shared-egress behavior vary by deployment. Qualify false positives,
  queue pressure, and XDP/SHM projection capacity under representative load.
- **Mesh:** mesh functionality is experimental and has separate acceptance
  requirements. Do not interpret enabled mesh code or green core workflows as
  production multi-node qualification.
