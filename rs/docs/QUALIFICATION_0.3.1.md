# Qualification — 0.3.1

Production hardening release. All 0.3.0 detection tests carry forward unchanged. New sections cover enforcement, persistence, failure semantics, and release artifacts.

---

## Section A — Detection (0.3.0 carry-forward)

| Scenario | Pass | Key test(s) |
|---|---|---|
| normal traffic | ✅ | benign_cold_start_ramp_never_accumulates, cusum_quiet_stays_zero |
| single-IP burst | ✅ | single_ip_burst_does_not_block_subnet, single_spike_cannot_arm_tripwire |
| sustained abuse | ✅ | cusum_sustained_drift_fires, emergency_burst_fires_once_before_flush |
| distributed abuse | ✅ | distributed_swarm_satisfies_dual_gate |
| subnet abuse | ✅ | subnet_rate_accumulates (v4), v6_subnet_swarm_blocks_via_index_cardinality |
| cold start | ✅ | benign_cold_start_ramp (unit), subnet_stress.py (e2e) |
| shared/CGNAT | ✅ | cgnat_integration::test_end_to_end_cgnat_shielding |
| legit + abusive | ✅ | pulsed_swarm_meets_store_dual_gate, raw_volume_alone_insufficient |

---

## Section B — Enforcement

| Test | Trigger | Expected state | Health result | Operator signal |
|---|---|---|---|---|
| IPv4 block | ramshield-cli block 203.0.113.7 | Block present in store + XDP map | 200 | /api/blocks/active |
| IPv6 block | ramshield-cli block 2001:db8::1 | Block present in store + XDP map | 200 | /api/blocks/active |
| IPv4 CIDR | ramshield-cli block 198.51.100.0/24 | CIDR present in store + XDP map | 200 | /api/blocks/active |
| IPv6 CIDR | ramshield-cli block 2001:db8::/48 | CIDR present in store + XDP map | 200 | /api/blocks/active |
| TTL expiry | block --ttl 10; wait 12s | Block absent from store + XDP map | 200 | metric ramshield_blocks_expired_total |
| manual unblock | ramshield-cli unblock <ip> | Block absent immediately | 200 | /api/blocks/active |
| reconciliation | evict from map; verify userspace re-adds | Rule reappears in map | 200 | ramshield_xdp_reconcile_successes_total |
| XDP attach | boot with xdp.enabled=true | xdp_active=true, protection_state=Protected | 200 | /healthz protection_state |
| XDP detach | set xdp.enabled=false; restart | xdp_active=false, protection_state=Degraded | 200 | /healthz protection_state | config change requires restart |
| XDP reattach | set xdp.enabled=true; restart | xdp_active=true, protection_state=Protected | 200 | /healthz protection_state | config change requires restart |

---

## Section C — Persistence

| Test | Trigger | Expected state | Health result | Operator signal |
|---|---|---|---|---|
| SIGKILL recovery | kill -9; restart | All non-expired blocks restored | 200 | /healthz + ramshield_cli check |
| WAL rotation recovery | fill WAL > seg_max_bytes; rotate; restart | Blocks restored from new segment | 200 | /healthz + ramshield_cli check |
| WAL retention recovery | retain > retention_max_bytes; restart | Active blocks survive retention | 200 | ramshield_wal_segments_pruned_total |
| CIDR recovery | block CIDR; kill -9; restart | CIDR block restored | 200 | ramshield_cli check <cidr> |
| TTL recovery | block with TTL; kill -9 before expiry; restart | Block restored with remaining TTL | 200 | ramshield_cli info <ip> |
| WAL open failure | chmod 000 WAL dir; restart | protection_state=Failed, HTTP 503 | 503 | /healthz reason="wal open failed" |
| WAL replay failure | corrupt WAL; restart | protection_state=Failed, HTTP 503 | 503 | /healthz reason="wal replay failed" |

---

## Section D — Failure semantics

| Scenario | Trigger | Expected state | Health | Signal | Recovery |
|---|---|---|---|---|---|
| XDP failure | bad interface/capabilities | protection_state=Failed/Degraded | 503/200 | /healthz xdp_active=false | fix interface/caps; restart |
| WAL open failure | permission denied | protection_state=Failed | 503 | reason="wal open failed" | fix perms; restart |
| WAL replay failure | corrupted segment | protection_state=Failed | 503 | reason="wal replay failed" | remove corrupt; restart |
| WAL disk full | fill disk | protection_state=Degraded/Failed | 503 | wal_write_errors_total | free space; restart |
| WAL permission denied | chmod WAL dir | protection_state=Failed | 503 | /healthz reason="wal open failed" | fix perms; restart |
| XDP map full | > max_entries | reconciliation continues; eviction logged | 200 | ramshield_xdp_evictions_total | wait for TTL expiry |
| enforcement queue full | burst > queue cap | backpressure; drops logged | 200 | enforcement_queue_full_total | throttle proxy |
| IPC overload | > max_connections | new connections rejected | 200 | ipc_connections_rejected_total | backoff clients |
| dashboard overload | > max_login_attempts | lockout enforced | 200 | auth_lockout_total | wait/cooldown |
| Argon2 overload | > argon2_parallelism | queued; no thread starvation | 200 | auth_verification_wait_ms | scale down auth traffic |

---

## Section E — Deployment / Release artifacts

| Artifact | Check | Pass criteria |
|---|---|---|
| clean checkout | git clone + cargo build --release --locked --features full | builds without local state |
| release_candidate.sh | ./scripts/release_candidate.sh | produces sha256 of binary + Cargo.lock |
| no local symlinks | find . -type l | zero results |
| no generated artifacts | find . -name '*.pyc' -o -name '__pycache__' | zero results |
| version consistency | Cargo.toml version == git tag | exact match |
| CHANGELOG | contains 0.3.1 section | Fixed/Security/Qualification entries |
| K8s manifests | kubectl apply --dry-run=client -f deploy/k8s/ | no errors |
| NetworkPolicy | selectors non-empty | specific pod/namespace selectors |
| Secret required | argon2-hash optional=false | dashboard binds public → required |

---

## Pass gate

All sections must pass for 0.3.1 tag. Each failure scenario must have:
- deterministic trigger
- observable state transition
- documented operator remediation