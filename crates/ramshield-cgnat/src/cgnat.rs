//! Deterministic CGNAT guard for subnet-level mitigation.

use std::net::IpAddr;

pub const CGNAT_TIER_ALLOW: u8 = 0;
pub const CGNAT_TIER_CHALLENGE: u8 = 1;
pub const CGNAT_TIER_XDP_DROP: u8 = 2;
pub const CGNAT_TIER_BLOCK: u8 = 3;

pub struct CgnatGuard;

impl CgnatGuard {
    pub fn new() -> Self {
        Self
    }

    /// RFC 6598 shared address space must stay out of the L3 drop path.
    pub fn is_shared_infrastructure(ip: IpAddr) -> bool {
        matches!(ip, IpAddr::V4(v4) if (u32::from(v4) & 0xffc0_0000) == 0x6440_0000)
    }

    /// Classify the subnet itself. Text entropy and heap addresses are not
    /// network signals and cannot determine an enforcement tier.
    pub fn classify_subnet(&self, subnet: IpAddr) -> u8 {
        if Self::is_shared_infrastructure(subnet) {
            CGNAT_TIER_CHALLENGE
        } else {
            CGNAT_TIER_BLOCK
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    #[test]
    fn cgnat_prefix_is_challenge_and_public_prefix_is_block() {
        let guard = CgnatGuard::new();
        assert_eq!(
            guard.classify_subnet(IpAddr::V4(Ipv4Addr::new(100, 64, 1, 0))),
            CGNAT_TIER_CHALLENGE
        );
        assert_eq!(
            guard.classify_subnet(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 0))),
            CGNAT_TIER_BLOCK
        );
    }
}
