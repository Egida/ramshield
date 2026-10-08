use super::*;

impl BloomFilter {
    pub fn new(bits: usize) -> Self {
        Self {
            bits: (0..bits.div_ceil(64)).map(|_| AtomicU64::new(0)).collect(),
            size: bits,
        }
    }

    /// Wipe the filter. Required because the bloom is advisory and grows
    /// monotonically otherwise — every bit becomes set within hours of
    /// any real traffic, after which `contains_hashed` always returns
    /// true and the cold-skip short-circuit at line 365 stops skipping
    /// anything, ballooning the store to O(total_ips_ever_seen).
    pub fn clear(&mut self) {
        for w in &self.bits {
            w.store(0, Ordering::Relaxed);
        }
    }

    pub fn slots(ip: &IpAddr) -> (usize, usize) {
        // ahash, not DefaultHasher (SipHash): slots() runs once per event at
        // ingest rate. Collision-storm resistance is not a property the bloom
        // needs (false positives are its whole design); speed is.
        let mut h = ahash::AHasher::default();
        ip.hash(&mut h);
        let x = h.finish();
        let a = x as usize;
        let b = (x.rotate_left(17) as usize).wrapping_mul(2_654_435_761);
        (a, b)
    }

    pub fn contains_hashed(&self, a: usize, b: usize) -> bool {
        let a = a % self.size;
        let b = b % self.size;
        (self.bits[a / 64].load(Ordering::Relaxed) >> (a % 64)) & 1 == 1
            && (self.bits[b / 64].load(Ordering::Relaxed) >> (b % 64)) & 1 == 1
    }

    /// Shared insert through the ArcSwap-resident filter: fetch_or on the
    /// words in place. No clone, no store. Concurrency: the 8 s epoch clear
    /// on another thread may swap the Arc under us — a lost bit then only
    /// costs one re-promote (advisory revisit cache; FPs open, never reject).
    pub fn insert_shared(&self, ip: IpAddr) {
        let (a, b) = Self::slots(&ip);
        self.insert_hashed_in(a, b);
    }

    pub fn insert_hashed(&mut self, a: usize, b: usize) {
        self.insert_hashed_in(a, b);
    }

    pub(crate) fn insert_hashed_in(&self, a: usize, b: usize) {
        let a = a % self.size;
        let b = b % self.size;
        self.bits[a / 64].fetch_or(1u64 << (a % 64), Ordering::Relaxed);
        self.bits[b / 64].fetch_or(1u64 << (b % 64), Ordering::Relaxed);
    }

    pub fn contains(&self, ip: IpAddr) -> bool {
        let (a, b) = Self::slots(&ip);
        self.contains_hashed(a, b)
    }

    pub fn insert(&mut self, ip: IpAddr) {
        let (a, b) = Self::slots(&ip);
        self.insert_hashed(a, b);
    }
}
