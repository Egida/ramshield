use crate::*;
use ramshield_mesh::transport::MeshMessage;

/// Local processing status for a batch drained from the mesh transport.
/// This is deliberately separate from the sender's socket-write report.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MeshApplyReport {
    pub received: usize,
    pub applied: usize,
    pub ignored: usize,
    pub failed: usize,
    pub state_changed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshApplyOutcome {
    Applied,
    Ignored,
}

impl MeshApplyReport {
    fn record(
        &mut self,
        result: Result<MeshApplyOutcome, EnforcementError>,
        state_changed: bool,
        peer_node_id: u32,
        ip: std::net::IpAddr,
        action: &'static str,
    ) {
        self.received += 1;
        self.state_changed |= state_changed;
        match result {
            Ok(MeshApplyOutcome::Applied) => self.applied += 1,
            Ok(MeshApplyOutcome::Ignored) => self.ignored += 1,
            Err(error) => {
                self.failed += 1;
                tracing::warn!(
                    peer_node_id,
                    ip = %ip,
                    action,
                    error = %error,
                    "mesh message was received but local enforcement application failed; anti-entropy must retry"
                );
            }
        }
    }
}

impl EnforcementService {
    /// Persist the original mesh operation identity before allowing it into the
    /// local CRDT. A local WAL LSN is not an origin counter and is never used
    /// to rewrite the remote dot.
    fn persist_mesh_mutation(
        &mut self,
        ip: std::net::IpAddr,
        dot: ramshield_mesh::aworset::ClusterDot,
        action: ramshield_storage::wal::MeshMutationAction,
        created_at_ms: u64,
        expires_at_ms: Option<u64>,
        tier: u8,
    ) -> Result<(), EnforcementError> {
        let Some(wal) = self.wal.as_ref() else {
            return Ok(());
        };
        let ts_ns = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        let entry = ramshield_storage::wal::WalEntry::MeshMutation {
            schema_version: 1,
            origin_node_id: dot.node_id,
            origin_incarnation: dot.incarnation.to_string(),
            origin_counter: dot.counter,
            ip: ip.to_string(),
            action,
            created_at_ms,
            expires_at_ms,
            tier,
            signature: None,
            ts_ns,
        };
        let lsn = wal
            .append(&entry)
            .map_err(|error| EnforcementError::Wal(error.to_string()))?;
        wal.sync()
            .map_err(|error| EnforcementError::Wal(error.to_string()))?;
        self.last_wal_lsn = Some(lsn);
        Ok(())
    }

    pub(crate) async fn apply_mesh_messages(&mut self) -> MeshApplyReport {
        let mut report = MeshApplyReport::default();
        let Some(handle) = self.mesh_handle.clone() else {
            return report;
        };
        let messages = handle.drain().await;
        if messages.is_empty() {
            return report;
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        for message in messages {
            match message {
                MeshMessage::Block(delta) => {
                    let peer_node_id = delta.dot.node_id;
                    let ip = delta.ip;
                    let (state_changed, result) = self.apply_mesh_block(delta, now_ms).await;
                    report.record(result, state_changed, peer_node_id, ip, "block");
                }
                MeshMessage::Unblock(delta) => {
                    let peer_node_id = delta.dot.node_id;
                    let ip = delta.ip;
                    let (state_changed, result) = self.apply_mesh_unblock(delta, now_ms).await;
                    report.record(result, state_changed, peer_node_id, ip, "unblock");
                }
                MeshMessage::Sync { blocks, unblocks } => {
                    for delta in blocks {
                        let peer_node_id = delta.dot.node_id;
                        let ip = delta.ip;
                        let (state_changed, result) = self.apply_mesh_block(delta, now_ms).await;
                        report.record(result, state_changed, peer_node_id, ip, "sync_block");
                    }
                    for delta in unblocks {
                        let peer_node_id = delta.dot.node_id;
                        let ip = delta.ip;
                        let (state_changed, result) = self.apply_mesh_unblock(delta, now_ms).await;
                        report.record(result, state_changed, peer_node_id, ip, "sync_unblock");
                    }
                }
            }
        }
        // Notify only after processing a batch that changed the CRDT. A merge
        // still needs propagation if its later enforcement/XDP projection fails.
        if report.state_changed {
            handle.notify_remote_state_applied().await;
        }
        if report.failed > 0 {
            tracing::warn!(
                received = report.received,
                applied = report.applied,
                ignored = report.ignored,
                failed = report.failed,
                "mesh batch completed with enforcement failures"
            );
        }
        report
    }

    async fn apply_mesh_block(
        &mut self,
        delta: ramshield_mesh::aworset::ClusterBlockDelta,
        now_ms: u64,
    ) -> (bool, Result<MeshApplyOutcome, EnforcementError>) {
        if delta.expires_at_ms <= now_ms {
            return (false, Ok(MeshApplyOutcome::Ignored));
        }
        let Some(mesh) = self.mesh_blocklist.clone() else {
            return (false, Ok(MeshApplyOutcome::Ignored));
        };
        if self.mesh_operator_suppressions.contains(&delta.ip) {
            return (false, Ok(MeshApplyOutcome::Ignored));
        }
        // Persist the original origin dot before merging. Repeated deliveries
        // whose dot is already retained need no additional WAL record.
        if mesh.needs_merge_delta(&delta) {
            if let Err(error) = self.persist_mesh_mutation(
                delta.ip,
                delta.dot,
                ramshield_storage::wal::MeshMutationAction::Block,
                delta.created_at_ms,
                (delta.expires_at_ms != u64::MAX).then_some(delta.expires_at_ms),
                delta.tier,
            ) {
                return (false, Err(error));
            }
        }
        // CRDT state can be merged before enforcement fails (e.g. storage or
        // XDP projection error); retries repair the local projection.
        let changed = mesh.merge_delta(&delta);
        if should_skip_mesh_block(changed, self.mesh_applied_ips.contains(&delta.ip)) {
            return (changed, Ok(MeshApplyOutcome::Ignored));
        }
        let remaining_ms = delta.expires_at_ms.saturating_sub(now_ms);
        let ttl = if delta.expires_at_ms == u64::MAX {
            0
        } else {
            remaining_ms.saturating_add(999) / 1000
        };
        if delta.expires_at_ms != u64::MAX && ttl == 0 {
            return (changed, Ok(MeshApplyOutcome::Ignored));
        }
        let cmd = EnforceCommand {
            decision_id: Uuid::new_v4(),
            policy_version: 1,
            source: "mesh".into(),
            actor: format!("mesh:{}", delta.dot.node_id),
            timestamp_utc: (now_ms / 1000) as i64,
            ttl_seconds: ttl,
            reason: "mesh_final".into(),
            ip: delta.ip,
            cidr: None,
            action: EnforceAction::Block,
            evidence_source: ramshield_types::EvidenceSource::FleetSignals,
        };
        let enforce_result = self
            .enforce(cmd)
            .await
            .and_then(|result| require_xdp_projection(result, "block"));
        if enforce_result.is_ok() {
            self.mesh_applied_ips.insert(delta.ip);
        }
        (changed, enforce_result.map(|_| MeshApplyOutcome::Applied))
    }

    async fn apply_mesh_unblock(
        &mut self,
        delta: ramshield_mesh::aworset::ClusterUnblockDelta,
        now_ms: u64,
    ) -> (bool, Result<MeshApplyOutcome, EnforcementError>) {
        let Some(mesh) = self.mesh_blocklist.clone() else {
            return (false, Ok(MeshApplyOutcome::Ignored));
        };
        // The tombstone must be durable before it can suppress a live dot.
        if mesh.needs_merge_unblock(&delta) {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if let Err(error) = self.persist_mesh_mutation(
                delta.ip,
                delta.dot,
                ramshield_storage::wal::MeshMutationAction::Unblock,
                now_ms,
                None,
                0,
            ) {
                return (false, Err(error));
            }
        }
        // As with blocks, retain retryability if the local enforcement unblock fails.
        let changed = mesh.merge_unblock_delta(&delta);
        let still_blocked = mesh.is_blocked(&delta.ip, now_ms);
        let locally_applied = self.mesh_applied_ips.contains(&delta.ip);
        if should_skip_mesh_unblock(still_blocked, locally_applied) {
            return (changed, Ok(MeshApplyOutcome::Ignored));
        }
        let cmd = EnforceCommand {
            decision_id: Uuid::new_v4(),
            policy_version: 1,
            source: "mesh".into(),
            actor: format!("mesh:{}", delta.dot.node_id),
            timestamp_utc: (now_ms / 1000) as i64,
            ttl_seconds: 0,
            reason: "mesh_purge".into(),
            ip: delta.ip,
            cidr: None,
            action: EnforceAction::Unblock,
            evidence_source: ramshield_types::EvidenceSource::FleetSignals,
        };
        let enforce_result = self
            .enforce(cmd)
            .await
            .and_then(|result| require_xdp_projection(result, "unblock"));
        if enforce_result.is_ok() {
            self.mesh_applied_ips.remove(&delta.ip);
        }
        (changed, enforce_result.map(|_| MeshApplyOutcome::Applied))
    }
}

fn require_xdp_projection(
    result: EnforceResult,
    action: &'static str,
) -> Result<EnforceResult, EnforcementError> {
    if result.xdp_applied {
        Ok(result)
    } else {
        Err(EnforcementError::Xdp(result.error.unwrap_or_else(|| {
            format!("mesh {action} committed but XDP projection was not applied")
        })))
    }
}

fn should_skip_mesh_block(crdt_changed: bool, locally_applied: bool) -> bool {
    !crdt_changed && locally_applied
}

fn should_skip_mesh_unblock(still_blocked: bool, locally_applied: bool) -> bool {
    still_blocked || !locally_applied
}

#[cfg(test)]
mod retry_tests {
    use super::{
        MeshApplyOutcome, MeshApplyReport, require_xdp_projection, should_skip_mesh_block,
        should_skip_mesh_unblock,
    };
    use crate::{EnforceResult, EnforcementError};
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn failed_xdp_projection_is_not_reported_as_applied() {
        let result = EnforceResult {
            decision_id: uuid::Uuid::nil(),
            committed: true,
            applied: true,
            wal_lsn: None,
            xdp_applied: false,
            error: None,
        };

        assert!(matches!(
            require_xdp_projection(result, "block"),
            Err(EnforcementError::Xdp(_))
        ));
    }

    #[test]
    fn application_report_distinguishes_applied_ignored_and_failed() {
        let mut report = MeshApplyReport::default();
        let ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

        report.record(Ok(MeshApplyOutcome::Applied), false, 7, ip, "block");
        report.record(Ok(MeshApplyOutcome::Ignored), false, 7, ip, "duplicate");
        report.record(
            Err(EnforcementError::Xdp("test projection failure".into())),
            true,
            7,
            ip,
            "block",
        );

        assert_eq!(report.received, 3);
        assert_eq!(report.applied, 1);
        assert_eq!(report.ignored, 1);
        assert_eq!(report.failed, 1);
        assert!(
            report.state_changed,
            "a CRDT merge must trigger repair even if projection fails"
        );
    }

    #[test]
    fn unchanged_block_is_retried_until_local_projection_succeeds() {
        assert!(
            !should_skip_mesh_block(false, false),
            "failed prior apply must retry"
        );
        assert!(
            should_skip_mesh_block(false, true),
            "successful duplicate can be skipped"
        );
        assert!(
            !should_skip_mesh_block(true, true),
            "new CRDT state must be applied"
        );
    }

    #[test]
    fn merged_unblock_is_retried_until_local_projection_succeeds() {
        assert!(
            !should_skip_mesh_unblock(false, true),
            "failed prior unblock must retry"
        );
        assert!(
            should_skip_mesh_unblock(true, true),
            "another live ban still blocks the IP"
        );
        assert!(
            should_skip_mesh_unblock(false, false),
            "already-unapplied IP needs no duplicate unblock"
        );
    }
}
