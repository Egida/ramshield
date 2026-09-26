


## Upgrade

```bash
# 1. Check current version
ramshield --version

# 2. Stop service
systemctl stop ramshield

# 3. Backup existing WAL
cp -a /var/lib/ramshield/wal /var/lib/ramshield/wal.backup

# 4. Install new version
dpkg -i ramshield_0.4.0_amd64.deb

# 5. Validate config
ramshield config validate --config /etc/ramshield/config.toml

# 6. Run doctor
ramshield --doctor --config /etc/ramshield/config.toml

# 7. Start service
systemctl start ramshield

# 8. Verify protection
ramshield status
curl http://127.0.0.1:9999/healthz
```

### Upgrade verification checklist

- [ ] `ramshield --version` reports new version
- [ ] `ramshield config validate` passes
- [ ] `ramshield --doctor` passes all checks
- [ ] `/healthz` returns 200
- [ ] Dashboard loads at `http://127.0.0.1:9999`
- [ ] `/metrics` returns Prometheus data
- [ ] Existing blocks are enforced
- [ ] New blocks can be created

### Major version upgrades

- Read release notes for breaking WAL format changes
- If WAL format changed: export block list before upgrade, truncate WAL after upgrade
- Roll out to staging first

### Back up block state

```bash
curl -s http://127.0.0.1:9999/api/blocks/active | jq -c '.[] | {ip, reason}'
```

## Rollback

### When to roll back

- New version fails to start or maintain protection
- Regression in detection or enforcement
- Incompatible config format
- Unacceptable performance degradation

### Rollback path

```bash
# 1. Install previous version
dpkg -i ramshield_0.2.0_amd64.deb

# 2. Restore config if format changed
cp /etc/ramshield/config.toml.backup /etc/ramshield/config.toml

# 3. Restart service
systemctl restart ramshield

# 4. Verify
ramshield --doctor --config /etc/ramshield/config.toml
ramshield status
curl http://127.0.0.1:9999/healthz
```

### Rollback verification

- [ ] Previous version starts cleanly
- [ ] Config loads and validates
- [ ] `/healthz` returns 200
- [ ] Dashboard accessible
- [ ] If WAL restored: existing blocks enforced
- [ ] New blocks can be created

### Staging rollback procedure

**Purpose:** Stop a RamShield pilot slice safely, preserve evidence, restore prior edge policy, verify protected service remains reachable.

**Preconditions:**
- Record commit/image digest, config hash, interface, traffic slice, active policy, WAL path, operator
- Confirm customer's existing edge policy and rollback owner
- Test this procedure in staging before production exposure
- Keep the prior edge configuration available and validated

**Host/systemd rollback:**
1. Stop new test traffic
2. Restore previously saved edge/proxy policy
3. `sudo systemctl stop ramshield`
4. Verify stopped: `systemctl is-active ramshield`
5. Verify interface and edge policy no longer use the pilot path
6. Check service logs and dashboard health
7. Preserve WAL and logs (do not delete before evidence capture)
8. Re-run health and reachability checks

**Evidence capture:**
Record UTC start/end, operator, reason, traffic slice, policy before/after, commands, service/pod status, health results, XDP status, WAL errors. Redact credentials, customer identifiers, IP traces, internal topology before sharing.