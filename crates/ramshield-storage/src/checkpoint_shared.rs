//! Shared checkpoint coordination between the enforcement actor and the
//! engine's periodic checkpoint loop.
//!
//! Owns two things:
//! * `barrier` — serializes enforcement durable mutations (WAL append +
//!   store mutation) against checkpoint boundary capture. Held briefly, never
//!   across an await, never around XDP/background work.
//! * `state` — a mirror of the enforcement expiration schedules (absolute
//!   Unix-ns deadlines), refreshed by the enforcement tick and consumed by
//!   `build_snapshot()` so checkpoints carry real TTLs instead of `None`.
//!
//! Lives in ramshield-storage so both the engine (which depends on storage)
//! and the enforcement actor (same) can share it without a cycle.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

use ramshield_types::IpNetwork;

/// CIDR block state carried in a checkpoint / shared mirror.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CidrSnapshot {
    pub network: IpNetwork,
    pub expires_at_ns: Option<u64>,
}

/// Explicit checkpoint state passed to `build_snapshot` — makes an
/// accidentally-empty checkpoint impossible (a defaulted struct is still
/// explicit at the call site, unlike a HashMap::new()).
#[derive(Debug, Clone, Default)]
pub struct CheckpointState {
    /// ip → absolute Unix-ns deadline (None semantic: absent = permanent).
    pub ip_expirations: std::collections::HashMap<IpAddr, u64>,
    /// Authoritative active CIDRs with their deadlines.
    pub cidrs: Vec<CidrSnapshot>,
}

/// Absolute deadline mirror + checkpoint barrier.
#[derive(Default)]
pub struct CheckpointShared {
    /// See module docs. Lock order: barrier → state (never the reverse).
    pub barrier: Mutex<()>,
    state: Mutex<SharedState>,
}

/// Snapshot of expiration deadlines, keyed by absolute Unix-ns.
/// `None`-expiry (permanent) entries are absent by construction: only
/// temporary blocks appear here.
#[derive(Debug, Clone, Default)]
pub struct SharedState {
    pub ip_expirations: HashMap<IpAddr, u64>,
    pub cidr_expirations: HashMap<IpNetwork, u64>,
}

impl SharedState {
    /// Move all entries from `other` into self (replace semantics).
    pub fn copy_from(&mut self, other: SharedState) {
        self.ip_expirations = other.ip_expirations;
        self.cidr_expirations = other.cidr_expirations;
    }
}

impl CheckpointShared {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the mirror wholesale (enforcement tick rebuilds it from its
    /// authoritative maps — bounded by pending expirations, small).
    pub fn publish(&self, state: SharedState) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .copy_from(state);
    }

    /// Point read for the checkpoint builder.
    pub fn state(&self) -> SharedState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn publish_then_state_returns_mirror() {
        let shared = CheckpointShared::new();
        let ip = IpAddr::V4(Ipv4Addr::new(10, 9, 8, 7));
        let net = IpNetwork::new(ip, 24).unwrap();
        shared.publish(SharedState {
            ip_expirations: [(ip, 123u64)].into(),
            cidr_expirations: [(net, 456u64)].into(),
        });
        let st = shared.state();
        assert_eq!(st.ip_expirations.get(&ip), Some(&123));
        assert_eq!(st.cidr_expirations.get(&net), Some(&456));
        // Empty publish clears.
        shared.publish(SharedState::default());
        assert!(shared.state().ip_expirations.is_empty());
    }

    #[test]
    fn barrier_is_exclusive() {
        let shared = CheckpointShared::new();
        let g = shared.barrier.lock().unwrap();
        assert!(shared.barrier.try_lock().is_err());
        drop(g);
        assert!(shared.barrier.try_lock().is_ok());
    }
}
