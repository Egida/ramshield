use super::enforce::PendingMeshMutation;
use crate::*;
use ramshield_mesh::transport::{MeshMessage, MeshMutationKind, ReplicatedMeshMutation};

/// Local processing status for a batch drained from the mesh transport.
/// This is deliberately separate from the sender's socket-write report.
struct WalPeerInFlight {
    peer: std::net::SocketAddr,
    peers: std::sync::Arc<std::sync::Mutex<ahash::AHashSet<std::net::SocketAddr>>>,
}

impl Drop for WalPeerInFlight {
    fn drop(&mut self) {
        if let Ok(mut peers) = self.peers.lock() {
            peers.remove(&self.peer);
        }
    }
}

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
    /// Apply a tombstone that does not release the effective IP because
    /// another origin dot is still live. WAL append, fsync and CRDT merge are
    /// kept inside the checkpoint barrier; the aggregate Store projection stays
    /// blocked, so a concurrent checkpoint remains consistent.
    fn persist_and_merge_unblock_only(
        &mut self,
        delta: &ramshield_mesh::aworset::ClusterUnblockDelta,
    ) -> Result<(), EnforcementError> {
        let checkpoint_arc = self.checkpoint_shared.clone();
        let _checkpoint_guard = checkpoint_arc
            .as_ref()
            .map(|shared| shared.barrier.lock().unwrap_or_else(|e| e.into_inner()));
        if let Some(wal) = self.wal.as_ref() {
            let ts_ns = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0);
            let mutation = PendingMeshMutation::Unblock {
                ip: delta.ip,
                node_id: delta.dot.node_id,
                incarnation: delta.dot.incarnation,
                counter: delta.dot.counter,
            };
            let lsn = wal
                .append(&mutation.to_wal_entry(ts_ns))
                .map_err(|error| EnforcementError::Wal(error.to_string()))?;
            wal.sync()
                .map_err(|error| EnforcementError::Wal(error.to_string()))?;
            self.last_wal_lsn = Some(lsn);
        }
        if let Some(mesh) = &self.mesh_blocklist {
            mesh.merge_unblock_delta(delta);
        }
        Ok(())
    }

    /// Schedule bounded pages of this node's durable WAL to each peer.
    /// Network waits and WAL directory scans run off the enforcement actor's
    /// 250ms tick; each peer has at most one in-flight page.
    pub(crate) async fn replicate_mesh_wal(&mut self) {
        let (Some(wal), Some(handle)) = (self.wal.clone(), self.mesh_handle.clone()) else {
            return;
        };
        let cursors = self.mesh_wal_cursors.clone();
        let inflight = self.mesh_wal_inflight.clone();

        for peer in handle.peers() {
            let inserted = inflight
                .lock()
                .map(|mut peers| peers.insert(peer))
                .unwrap_or(false);
            if !inserted {
                continue;
            }
            let cursor = cursors
                .lock()
                .ok()
                .and_then(|values| values.get(&peer).copied())
                .unwrap_or(0);
            let peer_wal = wal.clone();
            let peer_handle = handle.clone();
            let peer_cursors = cursors.clone();
            let peer_inflight = inflight.clone();
            tokio::spawn(async move {
                let _in_flight = WalPeerInFlight {
                    peer,
                    peers: peer_inflight,
                };
                let next_cursor = async {
                    let latest_lsn = peer_wal.durable_lsn();
                    if cursor >= latest_lsn {
                        return None;
                    }

                    let oldest_lsn = peer_wal.oldest_lsn();
                    let history_gap = match oldest_lsn {
                        Some(oldest) => cursor.saturating_add(1) < oldest,
                        None => cursor < latest_lsn,
                    };
                    if history_gap {
                        peer_handle.request_full_sync(peer).await;
                        warn!(
                            peer = %peer,
                            cursor_lsn = cursor,
                            oldest_lsn = ?oldest_lsn,
                            boundary_lsn = latest_lsn,
                            "mesh WAL cursor fell behind retention; requested full CRDT anti-entropy and reset tail boundary"
                        );
                        return Some(latest_lsn);
                    }

                    let read_wal = peer_wal.clone();
                    let page = tokio::task::spawn_blocking(move || {
                        read_wal.replay_with_lsn_page(cursor, 128)
                    })
                    .await;
                    let (records, _has_more) = match page {
                        Ok(Ok(page)) => page,
                        Ok(Err(error)) => {
                            warn!(peer = %peer, error = %error, "mesh WAL page read failed");
                            return None;
                        }
                        Err(error) => {
                            warn!(peer = %peer, error = %error, "mesh WAL page worker failed");
                            return None;
                        }
                    };
                    if records.is_empty() {
                        return None;
                    }
                    let start_lsn = records[0].0;
                    let end_lsn = records.last().map(|record| record.0).unwrap_or(cursor);
                    let mut mutations = Vec::new();
                    for (source_lsn, entry) in records {
                        if let WalEntry::MeshMutation {
                            schema_version,
                            origin_node_id,
                            origin_incarnation,
                            origin_counter,
                            ip,
                            action,
                            created_at_ms,
                            expires_at_ms,
                            tier,
                            ..
                        } = entry
                        {
                            if schema_version != 1
                                || origin_node_id == 0
                                || origin_counter == 0
                                || uuid::Uuid::parse_str(&origin_incarnation).is_err()
                            {
                                error!(
                                    peer = %peer,
                                    source_lsn,
                                    "invalid mesh WAL event blocks replication; peer cursor retained"
                                );
                                return None;
                            }
                            let Ok(ip) = ip.parse() else {
                                error!(
                                    peer = %peer,
                                    source_lsn,
                                    "invalid IP in mesh WAL event blocks replication; peer cursor retained"
                                );
                                return None;
                            };
                            mutations.push(ReplicatedMeshMutation {
                                source_lsn,
                                origin_node_id,
                                origin_incarnation,
                                origin_counter,
                                ip,
                                action: match action {
                                    ramshield_storage::wal::MeshMutationAction::Block => {
                                        MeshMutationKind::Block
                                    }
                                    ramshield_storage::wal::MeshMutationAction::Unblock => {
                                        MeshMutationKind::Unblock
                                    }
                                },
                                created_at_ms,
                                expires_at_ms,
                                tier,
                            });
                        }
                    }

                    let message = MeshMessage::WalBatch {
                        source_node_id: peer_handle.node_id(),
                        start_lsn,
                        end_lsn,
                        mutations,
                    };
                    match peer_handle.send_wal_batch(peer, message).await {
                        Ok(Some(ack_lsn)) if ack_lsn == end_lsn => Some(end_lsn),
                        Ok(ack_lsn) => {
                            warn!(
                                peer = %peer,
                                expected_lsn = end_lsn,
                                ack_lsn = ?ack_lsn,
                                "mesh WAL acknowledgement did not match page boundary; cursor retained"
                            );
                            None
                        }
                        Err(error) => {
                            warn!(
                                peer = %peer,
                                end_lsn,
                                error = %error,
                                "mesh WAL page was not acknowledged; cursor retained for retry"
                            );
                            None
                        }
                    }
                }
                .await;

                if let Some(next_lsn) = next_cursor {
                    if let Ok(mut values) = peer_cursors.lock() {
                        values.insert(peer, next_lsn);
                    }
                }
            });
        }
    }

    pub(crate) async fn apply_mesh_messages(&mut self) -> MeshApplyReport {
        let mut report = MeshApplyReport::default();
        let Some(handle) = self.mesh_handle.clone() else {
            return report;
        };
        let messages = handle.drain_with_ack().await;
        if messages.is_empty() {
            return report;
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        for received in messages {
            let mut ack_accepted = true;
            let mut ack_lsn = None;
            match received.message.clone() {
                MeshMessage::Block(delta) => {
                    let peer_node_id = delta.dot.node_id;
                    let ip = delta.ip;
                    let (state_changed, result) = self.apply_mesh_block(delta, now_ms).await;
                    ack_accepted &= mesh_ackable(&result, self.wal.is_some());
                    report.record(result, state_changed, peer_node_id, ip, "block");
                }
                MeshMessage::Unblock(delta) => {
                    let peer_node_id = delta.dot.node_id;
                    let ip = delta.ip;
                    let (state_changed, result) = self.apply_mesh_unblock(delta, now_ms).await;
                    ack_accepted &= mesh_ackable(&result, self.wal.is_some());
                    report.record(result, state_changed, peer_node_id, ip, "unblock");
                }
                MeshMessage::Sync { blocks, unblocks } => {
                    for delta in blocks {
                        let peer_node_id = delta.dot.node_id;
                        let ip = delta.ip;
                        let (state_changed, result) = self.apply_mesh_block(delta, now_ms).await;
                        ack_accepted &= mesh_ackable(&result, self.wal.is_some());
                        report.record(result, state_changed, peer_node_id, ip, "sync_block");
                    }
                    for delta in unblocks {
                        let peer_node_id = delta.dot.node_id;
                        let ip = delta.ip;
                        let (state_changed, result) = self.apply_mesh_unblock(delta, now_ms).await;
                        ack_accepted &= mesh_ackable(&result, self.wal.is_some());
                        report.record(result, state_changed, peer_node_id, ip, "sync_unblock");
                    }
                }
                MeshMessage::WalBatch {
                    start_lsn: _,
                    end_lsn,
                    mutations,
                    ..
                } => {
                    for mutation in mutations {
                        let incarnation = match Uuid::parse_str(&mutation.origin_incarnation) {
                            Ok(incarnation) if !incarnation.is_nil() => incarnation,
                            _ => {
                                ack_accepted = false;
                                continue;
                            }
                        };
                        match mutation.action {
                            MeshMutationKind::Block => {
                                let delta = ramshield_mesh::aworset::ClusterBlockDelta {
                                    ip: mutation.ip,
                                    dot: ramshield_mesh::aworset::ClusterDot {
                                        node_id: mutation.origin_node_id,
                                        incarnation,
                                        counter: mutation.origin_counter,
                                    },
                                    created_at_ms: mutation.created_at_ms,
                                    expires_at_ms: mutation.expires_at_ms.unwrap_or(u64::MAX),
                                    tier: mutation.tier,
                                };
                                let peer_node_id = delta.dot.node_id;
                                let ip = delta.ip;
                                let (state_changed, result) =
                                    self.apply_mesh_block(delta, now_ms).await;
                                ack_accepted &= mesh_ackable(&result, self.wal.is_some());
                                report.record(result, state_changed, peer_node_id, ip, "wal_block");
                            }
                            MeshMutationKind::Unblock => {
                                let delta = ramshield_mesh::aworset::ClusterUnblockDelta {
                                    ip: mutation.ip,
                                    dot: ramshield_mesh::aworset::ClusterDot {
                                        node_id: mutation.origin_node_id,
                                        incarnation,
                                        counter: mutation.origin_counter,
                                    },
                                };
                                let peer_node_id = delta.dot.node_id;
                                let ip = delta.ip;
                                let (state_changed, result) =
                                    self.apply_mesh_unblock(delta, now_ms).await;
                                ack_accepted &= mesh_ackable(&result, self.wal.is_some());
                                report.record(result, state_changed, peer_node_id, ip, "wal_unblock");
                            }
                        }
                    }
                    if ack_accepted {
                        ack_lsn = Some(end_lsn);
                    }
                }
            }
            received.acknowledge_with_lsn(ack_accepted, ack_lsn);
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
        // Do not mutate the CRDT yet. For a new dot, enforce_inner persists
        // and merges it while holding the checkpoint barrier, together with the
        // Store mutation. A retry of an already durable dot only repairs projection.
        let changed = mesh.needs_merge_delta(&delta);
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
        let enforce_result = if changed {
            self.enforce_mesh_mutation(
                cmd,
                PendingMeshMutation::Block {
                    ip: delta.ip,
                    node_id: delta.dot.node_id,
                    incarnation: delta.dot.incarnation,
                    counter: delta.dot.counter,
                    created_at_ms: delta.created_at_ms,
                    expires_at_ms: (delta.expires_at_ms != u64::MAX).then_some(delta.expires_at_ms),
                    tier: delta.tier,
                },
            )
            .await
        } else {
            self.enforce(cmd).await
        }
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
        // Predict the effective state without mutating it. The durable event and
        // tombstone merge must be atomic with the checkpoint barrier.
        let changed = mesh.needs_merge_unblock(&delta);
        let still_blocked = mesh.would_be_blocked_after_unblock(&delta, now_ms);
        let locally_applied = self.mesh_applied_ips.contains(&delta.ip);
        if changed && (still_blocked || !locally_applied) {
            return match self.persist_and_merge_unblock_only(&delta) {
                Ok(()) => (true, Ok(MeshApplyOutcome::Ignored)),
                Err(error) => (false, Err(error)),
            };
        }
        if !changed && should_skip_mesh_unblock(still_blocked, locally_applied) {
            return (false, Ok(MeshApplyOutcome::Ignored));
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
        let enforce_result = if changed {
            self.enforce_mesh_mutation(
                cmd,
                PendingMeshMutation::Unblock {
                    ip: delta.ip,
                    node_id: delta.dot.node_id,
                    incarnation: delta.dot.incarnation,
                    counter: delta.dot.counter,
                },
            )
            .await
        } else {
            self.enforce(cmd).await
        }
        .and_then(|result| require_xdp_projection(result, "unblock"));
        if enforce_result.is_ok() {
            self.mesh_applied_ips.remove(&delta.ip);
        }
        (changed, enforce_result.map(|_| MeshApplyOutcome::Applied))
    }
}

fn mesh_ackable(result: &Result<MeshApplyOutcome, EnforcementError>, has_wal: bool) -> bool {
    match result {
        Ok(MeshApplyOutcome::Ignored) => true,
        Ok(MeshApplyOutcome::Applied) => has_wal,
        Err(EnforcementError::Wal(_) | EnforcementError::InvalidCommand(_)) => false,
        // Storage/XDP failures occur after the mesh event is durable and merged;
        // the local reconciler retries the projection independently.
        Err(_) => has_wal,
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
