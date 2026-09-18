use std::sync::atomic::{AtomicU16, AtomicU32, AtomicU64, AtomicU8};

pub const FLAG_SHARED_INFRA: u8 = 0x01;

/* ── Cache-line aligned slot: exactly 64 bytes ── */
#[repr(C, align(64))]
pub struct ShmRuleEntry {
    /// Seqlock sequence counter: Odd = Writer active, Even = Consistent snapshot.
    pub sequence: AtomicU32,
    /// Mitigation tier: 0 = Allow, 1 = Soft Throttle / 429, 2 = PoW Challenge, 3 = Kernel Drop.
    pub tier: AtomicU8,
    /// Bitmask flags (0x01 = Shared Infrastructure / CGNAT).
    pub flags: AtomicU8,
    /// Rate limit quota: max requests per second; 0 = hard block.
    pub max_rps: AtomicU16,
    /// 64-bit key identifier (e.g., SipHash-2-4 of IP or Session).
    pub client_hash: AtomicU64,
    /// Absolute expiration timestamp (Unix epoch milliseconds).
    pub expires_at_ms: AtomicU64,
    /// Cryptographic salt or auxiliary challenge payload.
    pub challenge_seed: [u8; 16],
    /// Padding to round struct to exactly 64 bytes.
    pub _padding: [u8; 26],
}

impl ShmRuleEntry {
    pub const fn empty() -> Self {
        Self {
            sequence: AtomicU32::new(0),
            tier: AtomicU8::new(0),
            flags: AtomicU8::new(0),
            max_rps: AtomicU16::new(0),
            client_hash: AtomicU64::new(0),
            expires_at_ms: AtomicU64::new(0),
            challenge_seed: [0; 16],
            _padding: [0; 26],
        }
    }
}

/* ── Atomic, non-torn snapshot extracted by a reader ── */
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuleSnapshot {
    pub tier: u8,
    pub flags: u8,
    pub max_rps: u16,
    pub expires_at_ms: u64,
    pub challenge_seed: [u8; 16],
}