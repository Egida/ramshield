//! High-Performance Shared Memory Rule Table with OS Fallback
//!
//! Provides a memory-mapped rule table for sub-20ns reverse-proxy
//! lookups with deterministic subnet keys for shared-infrastructure handling.
//!
//! Designed for integration with the ramshield-analytics crate
//! (SubnetHll for IPv6 cardinality, host_bitmap for IPv4).

use memmap2::{MmapMut, MmapOptions};
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, AtomicU16, AtomicU32, AtomicU64, Ordering};

pub const SHM_TABLE_CAPACITY: usize = 65_536; // 64K rule slots

/// Stable Rust/C slot key for an IPv4 network prefix.
pub fn subnet_key(network: u32, prefix_len: u8) -> u64 {
    let mut h = network ^ u32::from(prefix_len);
    h ^= h >> 16;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    h = h.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 16;
    u64::from(h)
}

pub const FLAG_SHARED_INFRA: u8 = 0x01;

#[repr(C, align(64))]
pub struct ShmRuleEntry {
    /// Even values are stable; odd values mean a writer is publishing.
    pub seq: AtomicU32,
    pub client_hash: AtomicU64,   // 0 = Empty
    pub expires_at_ms: AtomicU64, // Absolute Unix epoch (ms)
    pub max_rps: AtomicU16,       // 0 = Block, >0 = Rate Limit
    pub tier: AtomicU8,           // 0: Allow, 1: 429, 2: Challenge, 3: XDP Drop
    pub flags: AtomicU8,          // Bit 0: Shared Infrastructure / CGNAT
    pub challenge_seed: [u8; 16],
    pub _padding: [u8; 26], // Exact 128-byte slot alignment
}

pub struct ShmTableManager {
    _file: std::fs::File,
    mmap: MmapMut,
    pub path: PathBuf,
}

impl ShmTableManager {
    /// Selects /dev/shm if available (Linux), falling back to temp_dir (macOS/Windows/CI)
    pub fn default_path() -> PathBuf {
        let dev_shm = Path::new("/dev/shm");
        if dev_shm.exists() && dev_shm.is_dir() {
            dev_shm.join("ramshield_rules")
        } else {
            std::env::temp_dir().join("ramshield_rules")
        }
    }

    pub fn open_or_create(path: &Path) -> std::io::Result<Self> {
        let total_size = SHM_TABLE_CAPACITY * std::mem::size_of::<ShmRuleEntry>();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;

        // Grow-only initialization. Never shrink or truncate an active mapping:
        // readers that still hold the old mapping cannot receive SIGBUS from a
        // daemon reopen or log rotation.
        if file.metadata()?.len() < total_size as u64 {
            file.set_len(total_size as u64)?;
        }

        // SAFETY: File is at least the mapped size. The mapping is shared with read-only proxies.
        let mmap = unsafe { MmapOptions::new().map_mut(&file)? };

        Ok(Self {
            _file: file,
            mmap,
            path: path.to_path_buf(),
        })
    }

    #[inline(always)]
    pub fn get_slot(&self, index: usize) -> &ShmRuleEntry {
        let offset = (index & (SHM_TABLE_CAPACITY - 1)) * std::mem::size_of::<ShmRuleEntry>();
        // SAFETY: Offset is masked by (SHM_TABLE_CAPACITY - 1), guaranteeing bounds within mmap.
        // Alignment is guaranteed by repr(C, align(64)).
        unsafe { &*(self.mmap.as_ptr().add(offset) as *const ShmRuleEntry) }
    }

    pub fn publish_rule(
        &self,
        client_hash: u64,
        ttl_ms: u64,
        tier: u8,
        max_rps: u16,
        is_shared: bool,
    ) {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let slot = self.get_slot(client_hash as usize);
        let flags = if is_shared { FLAG_SHARED_INFRA } else { 0 };

        // Seqlock publication: odd means a reader must retry; the final even
        // Release publishes the complete rule as one coherent snapshot.
        slot.seq.fetch_add(1, Ordering::Relaxed);
        slot.tier.store(tier, Ordering::Relaxed);
        slot.max_rps.store(max_rps, Ordering::Relaxed);
        slot.flags.store(flags, Ordering::Relaxed);
        slot.expires_at_ms.store(now_ms + ttl_ms, Ordering::Relaxed);
        slot.client_hash.store(client_hash, Ordering::Relaxed);
        slot.seq.fetch_add(1, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publish_ends_with_even_seqlock_and_open_never_shrinks_file() {
        let path = std::env::temp_dir().join(format!(
            "ramshield-shm-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let total = SHM_TABLE_CAPACITY * std::mem::size_of::<ShmRuleEntry>();
        {
            let manager = ShmTableManager::open_or_create(&path).unwrap();
            manager.publish_rule(7, 60_000, 3, 0, false);
            assert_eq!(manager.get_slot(7).seq.load(Ordering::Acquire) % 2, 0);
        }
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len((total + 4096) as u64)
            .unwrap();
        let _manager = ShmTableManager::open_or_create(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().len(),
            (total + 4096) as u64
        );
        let _ = std::fs::remove_file(path);
    }
}
