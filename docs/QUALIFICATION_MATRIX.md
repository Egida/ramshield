# Qualification Matrix — RamShield P1 #33

Audit §33: each dimension in the production contract, which test or
artifact proves it, and its current status on this host.

| # | Dimension                | Required qualification                  | Proof artifact                                  | Status |
|---|--------------------------|-----------------------------------------|------------------------------------------------|--------|
| 1 | Linux kernel             | Documented supported range              | `uname -r`                                     | ✅ 6.8.0-142-generic (kernel range not yet documented) |
| 2 | XDP mode                 | SKB / driver / native                   | `bpf-linker --version`; BPF ELF at             | ⚠️ linker 0.11.1 present; BPF crate exists, ELF build gated by target |
| 3 | IPv4                     | yes                                     | `tests/recovery_restart.rs` `crates/ramshield-detection/` v4 integration | ✅ covered by multiple integration paths |
| 4 | IPv6                     | yes                                     | `v6_events_aggregate_and_promote`, `v6_subnet_swarm_blocks`, `subnet_key_v6_roundtrip` | ✅ 4+ dedicated tests |
| 5 | CIDR                     | yes                                     | `recovery_cidr_block`, `checkpoint_cidr_only`, `checkpoint_equivalent_overlapping_cidrs` | ✅ 5+ tests covering CIDR persist + checkpoint |
| 6 | WAL                      | yes                                     | `crates/ramwal/` full crate; `cargo test -p ramwal` | ✅ 51/51 pass |
| 7 | SIGKILL recovery         | yes                                     | `tests/recovery_restart.rs` (kill/sleep/restart under real Store+WAL) | ✅ 5+ recovery-fn tests |
| 8 | Corrupted tail           | yes                                     | `partial_header_at_eof`, `golden_truncate_mid_payload`, `truncate_entire_segment_quarantines` | ✅ dedicated tests |
| 9 | Historical corruption    | yes                                     | `crc_mismatch_is_corruption`, `corrupt_magic`, `golden_corrupt_crc_first` | ✅ dedicated tests |
| 10 | Disk full                 | yes                                     | `ramwal/src/error.rs::Error::Io` checked; no e2e ENOSPC test | ❌ no disk-full test |
| 11 | Checkpoint                | yes                                     | `checkpoint_*` tests in `tests/recovery_restart.rs` + `crates/ramshield-storage/` | ✅ 10+ checkpoint lifecycle tests |
| 12 | Retention                 | yes                                     | `retention_without_checkpoint_keeps_everything`, `retention_with_checkpoint_deletes_older_segments`, `retention_never_deletes_active_segment`, `checkpoint_and_retention` | ✅ 4 dedicated tests |
| 13 | IPC authentication       | yes                                     | `auth_verify_never_panics`, `auth_rejects_wrong_key_signature` + protocol crate | ✅ coverage present |
| 14 | Replay protection        | yes                                     | `replay_outside_window_rejected`, `replay_store_distinguishes_nonces`, `replay_store_ttl_eviction` | ✅ 6+ dedicated tests |
| 15 | Public dashboard         | TLS proxy                              | `config.rs` — `behind_tls_proxy` / `public_bind_covers_v6_forms` | ✅ config validator enforces proxy requirement |
| 16 | systemd                  | yes                                     | `deploy/ramshield.service` unit; `ramshield-operator.service` declared | ⚠️ unit defined, not currently active on this host |
| 17 | Kubernetes               | Documented supported setup             | `deploy/k8s/` manifests (NetworkPolicy, RBAC, seccomp, probes) | ⚠️ manifests present; no k8s runtime on this host |

**Key gaps:**
- Disk-full (ENOSPC) has no e2e test — the main untested failure mode
- Kernel support range undocumented
- XDP ELF build not continuously verified
- systemd unit inactive on this host (daemon runs as foreground process)
- K8s manifests unevaluated outside of manifest review