//! High-Performance Shared Memory Rule Table with OS Fallback
//!
//! Provides a memory-mapped rule table for reverse-proxy lookups and
//! deterministic RFC 6598 shared-infrastructure protection.
//!
//! Designed for integration with the ramshield-analytics crate
//! (SubnetHll for IPv6 cardinality, host_bitmap for IPv4).

pub mod cgnat;
pub mod shm;

pub use cgnat::{
    CGNAT_TIER_ALLOW, CGNAT_TIER_BLOCK, CGNAT_TIER_CHALLENGE, CGNAT_TIER_XDP_DROP, CgnatGuard,
};
pub use shm::{FLAG_SHARED_INFRA, SHM_TABLE_CAPACITY, ShmRuleEntry, ShmTableManager};
