# RamShield Troubleshooting Guide

## XDP Unhealthy

**Symptom:** `/healthz` returns `503` with `reason: xdp_unhealthy` or `status: degraded`.
**Likely causes:**
- Kernel <5.15 (missing BPF features)
- BPF disabled in kernel config
- Interface does not exist or is down
- Missing capabilities (CAP_BPF, CAP_NET_ADMIN)
- XDP program unloadable (incompatible BPF bytecode)

**Diagnostic:**
```bash
ramshield --doctor
uname -r
getcap $(which ramshield)
ip link show
bpftool net
```

**Remediation:**
1. Verify kernel ≥5.15: `uname -r`
2. Set capabilities: `setcap 'cap_net_admin,cap_perfmon,cap_bpf+eip' $(which ramshield)`
3. Verify interface: `ip link show eth0`
4. Check kernel config: `cat /proc/sys/kernel/kernel.unprivileged_bpf_disabled` (should be 1)

**Escalation:** If XDP cannot be enabled, RamShield runs in degraded mode (in-band enforcement only — blocks are tracked but packets reach the proxy). This is better than no protection but should be temporary.

---

## WAL Unhealthy

**Symptom:** `/healthz` returns `503` with `reason: wal_unhealthy`. Dashboard shows `wal_lsn: 0`.
**Likely causes:**
- WAL directory does not exist or is not writable
- Disk full
- Filesystem permissions incorrect
- WAL segment corruption

**Diagnostic:**
```bash
ramshield --doctor
ls -la /var/lib/ramshield/wal/
df -h /var/lib/ramshield/wal/
```

**Remediation:**
1. Create directory: `mkdir -p /var/lib/ramshield/wal && chown ramshield:ramshield /var/lib/ramshield/wal`
2. Free disk space or increase `wal.retention_max_bytes`
3. If corrupted, truncate the WAL directory (blocks lost — start fresh)

**Escalation:** WAL is non-critical for runtime (blocks are in-memory + XDP). Only durability of blocks across restart is lost. All other protection remains active.

---

## Detector Unavailable

**Symptom:** No new blocks being created despite visible attack traffic. `ramshield_batches_total` counter stops incrementing.
**Likely causes:**
- Detection channel full (64k capacity)
- Detection thread panicked
- IPC events not reaching the engine

**Diagnostic:**
```bash
curl http://127.0.0.1:9999/healthz
curl http://127.0.0.1:9999/metrics | grep ramshield_events
journalctl -u ramshield -n 100 | grep -i detect
```

**Remediation:**
1. Check `ramshield_ingest_channel_depth` gauge — if near 64000, the IPC ingest is saturated.
2. Restart the service: `systemctl restart ramshield`
3. Check proxy is sending telemetry to the correct IPC address.

**Escalation:** Process restart. If detection remains unavailable, the proxy is still serving traffic (just with no abuse filtering).

---

## High Memory

**Symptom:** `ramshield_store_ram_pct` > 90. Dashboard shows high RSS.
**Likely causes:**
- Too many tracked IPs under active attack
- `engine.ram_limit_mb` too low for current traffic
- Memory leak (report as bug)

**Diagnostic:**
```bash
curl http://127.0.0.1:9999/metrics | grep ramshield_ram
curl http://127.0.0.1:9999/api/snapshot | jq '.ram_pct, .memory_usage_mb, .ips_tracked'
```

**Remediation:**
1. Increase `engine.ram_limit_mb` (requires restart)
2. Under severe attack, tracked IPs = attacker IPs — this is expected. The store evicts oldest tracked IPs when RAM limit is reached. Blocked IPs are never evicted.

**Escalation:** If `ram_limit_mb` is exhausted and IPs are being evicted, the missed attackers may not be detected. Increase limit or add more RAM to the host.

---

## High CPU

**Symptom:** `ramshield` process uses >100% CPU on multi-core host.
**Likely causes:**
- Very high event ingestion rate (>100k events/s)
- Detection engine saturated
- Dashboard polling too aggressive

**Diagnostic:**
```bash
top -b -n 1 | grep ramshield
curl http://127.0.0.1:9999/metrics | grep ramshield_events_ingested
```

**Remediation:**
1. Increase `detection.batch_window_ms` (reduces detection frequency)
2. Increase `detection.promote_min_events` (reduces tracked IPs)
3. Reduce dashboard polling frequency

---

## Blocks Not Appearing

**Symptom:** Attack traffic visible but no blocks created.
**Likely causes:**
- Detection thresholds too high
- Detection channel dropping events (check `ramshield_enforcement_dropped_total`)
- Config `detection.batch_block_enabled = false`

**Diagnostic:**
```bash
ramshield-cli stats
curl http://127.0.0.1:9999/metrics | grep -E 'ramshield_(blocks|enforcement_dropped)'
```

**Remediation:**
1. Lower `detection.rps_threshold`
2. Verify `detection.batch_block_enabled = true`
3. Check IPC is receiving events: `ramshield-cli stats`

---

## Blocks Not Expiring

**Symptom:** IPs remain blocked past their TTL.
**Likely causes:**
- Enforcement actor tick not running
- Clock skew (RAMShield uses system monotonic clock)
- TTL scheduling ring stalled

**Diagnostic:**
```bash
curl http://127.0.0.1:9999/healthz
```

**Remediation:** Restart the service.

---

## Dashboard Unavailable

**Symptom:** `curl http://127.0.0.1:9999/healthz` fails.
**Likely causes:**
- Dashboard disabled in config (`dashboard.enabled = false`)
- Dashboard thread panicked
- Port conflict

**Diagnostic:**
```bash
ss -tlnp | grep 9999
journalctl -u ramshield -n 50 | grep dashboard
```

**Remediation:**
1. Enable dashboard: `dashboard.enabled = true`
2. Check port: `ss -tlnp | grep 9999`
3. Restart service

---

## Restart Recovery Failed

**Symptom:** After restart, blocks are not restored. `/healthz` shows degraded state.
**Likely causes:**
- WAL replay failed
- WAL directory permissions changed
- WAL segment corruption

**Diagnostic:**
```bash
journalctl -u ramshield -n 100 | grep -i wal
journalctl -u ramshield -n 100 | grep -i replay
```

**Remediation:**
1. Run `ramshield --doctor`
2. Run `ramshield recovery test`
3. Check WAL directory permissions
4. Restart service