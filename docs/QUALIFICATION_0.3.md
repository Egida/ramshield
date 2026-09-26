# Detection qualification — 0.3.0

All scenarios tested via existing unit/integration tests. No code changes needed — current behavior understood and proven.

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