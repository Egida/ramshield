//! Hybrid Logical Clock (HLC) with burst-safe 32-bit logical counter
//!
//! 64-bit wall-clock milliseconds + 32-bit logical counter.
//! 32-bit counter avoids the 3.6-year wrap that a 16-bit counter would hit
//! under sustained burst traffic.

use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct Hlc {
    state: Mutex<(u64, u32)>,
    tick_count: AtomicU64,
}

impl Hlc {
    pub fn new() -> Self {
        Self {
            state: Mutex::new((0, 0)),
            tick_count: AtomicU64::new(0),
        }
    }

    /// Merge-and-tick: advance past local wall clock and received remote
    /// timestamp, bump the logical counter when physical time is tied.
    /// The mutex makes the physical/logical pair one atomic state transition
    /// under concurrent callers.
    pub fn tick(&self, remote_ms: u64, remote_seq: u32) -> (u64, u32) {
        let wall_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let (cur_phys, cur_seq) = *state;
        let next_phys = wall_ms.max(remote_ms).max(cur_phys);
        let next_seq = if next_phys == cur_phys && next_phys == remote_ms {
            cur_seq.max(remote_seq).saturating_add(1)
        } else if next_phys == cur_phys {
            cur_seq.saturating_add(1)
        } else {
            0
        };
        *state = (next_phys, next_seq);
        self.tick_count.fetch_add(1, Ordering::Relaxed);
        (next_phys, next_seq)
    }

    pub fn tick_count(&self) -> u64 {
        self.tick_count.load(Ordering::Relaxed)
    }

    /// Local-only tick: advance past the wall clock, zero the counter when
    /// physical time moves, else bump it.
    pub fn now(&self) -> u128 {
        let (phys, seq) = self.tick(0, 0);
        ((phys as u128) << 32) | (seq as u128)
    }
}

impl Default for Hlc {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monotonicity() {
        let hlc = Hlc::new();
        let t1 = hlc.now();
        let t2 = hlc.now();
        assert!(t2 >= t1);
        assert_eq!(hlc.tick_count(), 2);
    }

    #[test]
    fn remote_timestamp_wins() {
        let hlc = Hlc::new();
        let (phys, seq) = hlc.tick(u64::MAX, 7);
        assert!(phys >= u64::MAX - 1, "must advance past remote: {phys}");
        assert!(seq == 0, "remote ms ahead → seq reset, got {seq}");
        let (_, seq2) = hlc.tick(0, 0);
        assert!(seq2 >= 1, "tie on physical → seq bumps, got {seq2}");
    }
}
