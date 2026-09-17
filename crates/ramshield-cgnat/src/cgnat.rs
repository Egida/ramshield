//! CGNAT Guard with Shannon Entropy & Graduated Mitigation
//!
//! Graduated 4-tier response per-rule: Allow/Challenge/XDP Drop/Block.
//! Uses ramshield_forecasting::shannon_entropy() to fingerprint JA4 etc.
//! and flag shared-infra IPs for extra scrutiny.

use crate::shm::ShmTableManager;
use ramshield_forecasting::shannon_entropy;
use std::sync::Arc;

pub const CGNAT_TIER_ALLOW: u8 = 0;
pub const CGNAT_TIER_CHALLENGE: u8 = 1;
pub const CGNAT_TIER_XDP_DROP: u8 = 2;
pub const CGNAT_TIER_BLOCK: u8 = 3;

pub struct CgnatGuard {
    rules: Arc<ShmTableManager>,
    entropy_threshold: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn classify_uses_explicit_slot_key_not_memory_address() {
        let path = std::env::temp_dir().join(format!("ramshield-cgnat-{}", std::process::id()));
        let rules = Arc::new(ShmTableManager::open_or_create(Path::new(&path)).unwrap());
        rules.publish_rule(42, 60_000, CGNAT_TIER_BLOCK, 0, false);
        let guard = CgnatGuard::new(rules, 2.8);

        let fingerprint: Vec<u8> = (0..=31).collect();
        assert_eq!(guard.classify(&fingerprint, 42), CGNAT_TIER_BLOCK);
        assert_eq!(guard.classify(&fingerprint, 43), CGNAT_TIER_CHALLENGE);
        let _ = std::fs::remove_file(path);
    }
}

impl CgnatGuard {
    pub fn new(rules: Arc<ShmTableManager>, entropy_threshold: f64) -> Self {
        Self {
            rules,
            entropy_threshold,
        }
    }

    /// Convert fingerprint bytes into byte-value frequency counts
    fn fingerprint_counts(fingerprint: &[u8]) -> Vec<u64> {
        let mut counts = vec![0u64; 256];
        for &b in fingerprint {
            counts[b as usize] += 1;
        }
        counts
    }

    /// Classify a fingerprint against the caller's stable SHM slot key.
    ///
    /// The key is supplied by the caller because a slice address is process-
    /// local, ASLR-dependent, and unrelated to the published rule slot.
    pub fn classify(&self, fingerprint: &[u8], slot_key: u64) -> u8 {
        let counts = Self::fingerprint_counts(fingerprint);
        let entropy = shannon_entropy(&counts, fingerprint.len() as u64);
        let slot = self.rules.get_slot(slot_key as usize);

        if slot.client_hash.load(std::sync::atomic::Ordering::Relaxed) == 0 {
            CGNAT_TIER_CHALLENGE
        } else if entropy < self.entropy_threshold {
            CGNAT_TIER_XDP_DROP
        } else {
            slot.tier.load(std::sync::atomic::Ordering::Relaxed)
        }
    }
}
