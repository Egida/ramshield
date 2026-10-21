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

pub const SHM_TABLE_CAPACITY: usize = 262_144; // 256K rule slots (~16 MiB at 64 B/slot)
pub const SHM_PROBE_LIMIT: usize = 4;

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
    /// The 16-byte challenge seed is split into two naturally aligned
    /// atomic 64-bit words so a reader never observes a torn mix of two
    /// writer generations under seqlock.
    pub challenge_seed_lo: AtomicU64, // offset 24
    pub challenge_seed_hi: AtomicU64, // offset 32
    pub max_rps: AtomicU16,       // 0 = Block, >0 = Rate Limit
    pub tier: AtomicU8,           // 0: Allow, 1: Challenge (429+JS), 2: XDP Drop, 3: Block
    pub flags: AtomicU8,          // Bit 0: Shared Infrastructure / CGNAT
    pub _padding: [u8; 20],       // Exact 64-byte slot alignment
}

// Compile-time ABI assertions also run when this crate is cross-compiled. This
// catches architecture-dependent atomic alignment/offset changes before a
// target binary can be produced.
const _: () = {
    assert!(std::mem::size_of::<ShmRuleEntry>() == 64);
    assert!(std::mem::align_of::<ShmRuleEntry>() == 64);
    assert!(std::mem::offset_of!(ShmRuleEntry, seq) == 0);
    assert!(std::mem::offset_of!(ShmRuleEntry, client_hash) == 8);
    assert!(std::mem::offset_of!(ShmRuleEntry, expires_at_ms) == 16);
    assert!(std::mem::offset_of!(ShmRuleEntry, challenge_seed_lo) == 24);
    assert!(std::mem::offset_of!(ShmRuleEntry, challenge_seed_hi) == 32);
    assert!(std::mem::offset_of!(ShmRuleEntry, max_rps) == 40);
    assert!(std::mem::offset_of!(ShmRuleEntry, tier) == 42);
    assert!(std::mem::offset_of!(ShmRuleEntry, flags) == 43);
    assert!(std::mem::offset_of!(ShmRuleEntry, _padding) == 44);
};

#[cfg(test)]
mod abi_asserts {
    use super::*;

    #[test]
    fn challenge_seed_lo_is_naturally_aligned() {
        use std::mem::{align_of, size_of};
        assert_eq!(align_of::<ShmRuleEntry>(), 64);
        assert_eq!(size_of::<ShmRuleEntry>(), 64);
        let entry = ShmRuleEntry {
            seq: AtomicU32::new(0),
            client_hash: AtomicU64::new(0),
            expires_at_ms: AtomicU64::new(0),
            challenge_seed_lo: AtomicU64::new(0),
            challenge_seed_hi: AtomicU64::new(0),
            max_rps: AtomicU16::new(0),
            tier: AtomicU8::new(0),
            flags: AtomicU8::new(0),
            _padding: [0; 20],
        };
        // ptr::addr_of! on a field of a let-bound value works because the value
        // lives for the duration of the statement.
        let offset = std::ptr::addr_of!(entry.challenge_seed_lo) as usize;
        assert_eq!(offset % 8, 0, "challenge_seed_lo must be 8-byte aligned");
    }

    #[test]
    fn challenge_seed_hi_is_naturally_aligned() {
        let entry = ShmRuleEntry {
            seq: AtomicU32::new(0),
            client_hash: AtomicU64::new(0),
            expires_at_ms: AtomicU64::new(0),
            challenge_seed_lo: AtomicU64::new(0),
            challenge_seed_hi: AtomicU64::new(0),
            max_rps: AtomicU16::new(0),
            tier: AtomicU8::new(0),
            flags: AtomicU8::new(0),
            _padding: [0; 20],
        };
        // ptr::addr_of! on a field of a let-bound value works because the value
        // lives for the duration of the statement.
        let offset = std::ptr::addr_of!(entry.challenge_seed_hi) as usize;
        assert_eq!(offset % 8, 0, "challenge_seed_hi must be 8-byte aligned");
    }
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
        let mut opts = OpenOptions::new();
        opts.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
            // Refuse to follow a symlink planted at the path. `/dev/shm` is
            // world-writable and sticky, so any local user can pre-plant a
            // symlink named `ramshield_rules`. Without O_NOFOLLOW the daemon
            // would open the symlink target, chmod it to 0600 and resize it to
            // the table size, destroying an arbitrary file.
            opts.custom_flags(libc::O_NOFOLLOW);
        }
        let file = opts.open(path)?;

        // The file we just opened must be a regular file that belongs to us.
        // This closes the remaining hole: another local user can still
        // pre-plant a regular file (or a hard link) at the path, and a
        // privileged daemon would otherwise chmod and resize it. Failing closed
        // here is strictly earlier than the pre-existing `set_permissions`
        // call, which already failed with EPERM for a non-root daemon on a
        // file it did not own.
        //
        // The uid compared against is the EFFECTIVE uid. The daemon performs
        // no privilege drop anywhere in the tree (there is no `setuid`,
        // `setgid` or `seteuid` call site), so the euid is stable for the
        // process lifetime and a file the daemon itself created always
        // matches.
        //
        // What this does NOT close: a hard link to a file owned by the
        // daemon's own user still passes the ownership test, because it *is*
        // owned by that user. Such a file cannot have been planted by a
        // different local user, so it is not the attack this check targets.
        {
            let meta = file.metadata()?;
            if !meta.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is not a regular file", path.display()),
                ));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                let euid = unsafe { libc::geteuid() };
                if meta.uid() != euid {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::PermissionDenied,
                        format!(
                            "{} is owned by uid {}, not by this process (uid {})",
                            path.display(),
                            meta.uid(),
                            euid
                        ),
                    ));
                }
            }
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }

        // Grow-only initialization. Never shrink or truncate an active mapping:
        // readers that still hold the old mapping cannot receive SIGBUS from a
        // daemon reopen or log rotation.
        // ponytail: a leftover file from the pre-P0-B 128-byte ABI is read as
        // all-zero-ish garbage here (hash mismatch → empty slots); WAL replay
        // repopulates at boot. If a file must be reset explicitly, add an ABI
        // magic word at offset 0 and invalidate on mismatch.
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

    /// Publish a rule into the shared-memory table.
    ///
    /// Returns `false` when the 8-slot probe window is saturated, a concurrent
    /// writer holds the seqlock, or the selected slot became a live foreign
    /// rule after we observed it. Callers must treat `false` as “projection
    /// failed; userspace enforcement remains authoritative”.
    ///
    /// Writer protocol (must be mirrored by every other writer path):
    /// 1. Select a candidate slot (prefer existing hash, else free/expired).
    /// 2. Acquire exclusive writer ownership by CAS of `seq` even→odd.
    /// 3. Re-validate claim eligibility under the lock.
    /// 4. Write the full payload while `seq` is odd.
    /// 5. Release-fence, then publish stable even `seq`.
    ///
    /// Critical ordering: `client_hash` and every other payload field are
    /// written ONLY while `seq` is odd. Readers that observe an even `seq`
    /// therefore never see a mixed-generation snapshot.
    pub fn publish_rule(
        &self,
        client_hash: u64,
        ttl_ms: u64,
        tier: u8,
        max_rps: u16,
        is_shared: bool,
    ) -> bool {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Key 0 is the reserved empty-slot sentinel: a reader treats a slot
        // with client_hash == 0 as empty, so a rule published under it would
        // be invisible to every reader. Refuse it rather than publish a rule
        // that can never be enforced.
        if client_hash == 0 {
            return false;
        }

        let primary = client_hash as usize & (SHM_TABLE_CAPACITY - 1);
        let mut selected = None;

        // Phase 1: prefer an existing rule for this hash (canonical probe order).
        for probe in 0..SHM_PROBE_LIMIT {
            let slot = self.get_slot(primary.wrapping_add(probe));
            if slot.client_hash.load(Ordering::Acquire) == client_hash {
                selected = Some(slot);
                break;
            }
        }
        // Phase 2: first free (0) or expired slot.
        if selected.is_none() {
            for probe in 0..SHM_PROBE_LIMIT {
                let slot = self.get_slot(primary.wrapping_add(probe));
                let existing = slot.client_hash.load(Ordering::Acquire);
                let expires = slot.expires_at_ms.load(Ordering::Acquire);
                if existing == 0 || expires <= now_ms {
                    selected = Some(slot);
                    break;
                }
            }
        }
        let Some(slot) = selected else {
            return false;
        };

        // Phase 3: acquire exclusive writer ownership via seq even→odd CAS.
        // Spin a bounded number of times if another writer holds the lock.
        let mut acquired = false;
        for _ in 0..64 {
            let s = slot.seq.load(Ordering::Acquire);
            if s & 1 != 0 {
                // Another writer is active; brief pause then retry.
                std::hint::spin_loop();
                continue;
            }
            // CAS even → odd claims the writer lock.
            if slot
                .seq
                .compare_exchange(s, s.wrapping_add(1), Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                acquired = true;
                break;
            }
            // Lost the CAS race; retry.
            std::hint::spin_loop();
        }
        if !acquired {
            return false;
        }

        // Phase 4: re-validate under the lock. A concurrent writer may have
        // refreshed this slot between our probe and the CAS.
        let existing = slot.client_hash.load(Ordering::Acquire);
        let expires = slot.expires_at_ms.load(Ordering::Acquire);
        let claimable = existing == 0 || existing == client_hash || expires <= now_ms;
        if !claimable {
            // Release writer lock without mutating payload (seq odd → even).
            slot.seq.fetch_add(1, Ordering::Release);
            return false;
        }

        // Phase 5: write full payload while seq is odd.
        // Readers observing even seq will never see a partial update.
        let flags = if is_shared { FLAG_SHARED_INFRA } else { 0 };
        // Challenge seed: zeros until a real generator is wired. Written under
        // the odd-seq window so readers never observe a torn seed.
        slot.challenge_seed_lo.store(0, Ordering::Relaxed);
        slot.challenge_seed_hi.store(0, Ordering::Relaxed);
        slot.client_hash.store(client_hash, Ordering::Relaxed);
        slot.tier.store(tier, Ordering::Relaxed);
        slot.max_rps.store(max_rps, Ordering::Relaxed);
        slot.flags.store(flags, Ordering::Relaxed);
        slot.expires_at_ms
            .store(now_ms.saturating_add(ttl_ms), Ordering::Relaxed);

        // Phase 6: publish stable snapshot. Release fence ensures all payload
        // stores are globally visible before the even seq is observed.
        std::sync::atomic::fence(Ordering::Release);
        slot.seq.fetch_add(1, Ordering::Release); // odd → even
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_is_fixed_and_cache_aligned() {
        assert_eq!(std::mem::size_of::<ShmRuleEntry>(), 64);
        assert_eq!(std::mem::align_of::<ShmRuleEntry>(), 64);
        // Field offsets must match the C header exactly (P0-B).
        use std::mem::offset_of;
        assert_eq!(offset_of!(ShmRuleEntry, seq), 0);
        assert_eq!(offset_of!(ShmRuleEntry, client_hash), 8);
        assert_eq!(offset_of!(ShmRuleEntry, expires_at_ms), 16);
        assert_eq!(offset_of!(ShmRuleEntry, challenge_seed_lo), 24);
        assert_eq!(offset_of!(ShmRuleEntry, challenge_seed_hi), 32);
        assert_eq!(offset_of!(ShmRuleEntry, max_rps), 40);
        assert_eq!(offset_of!(ShmRuleEntry, tier), 42);
        assert_eq!(offset_of!(ShmRuleEntry, flags), 43);
    }

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
            assert!(manager.publish_rule(7, 60_000, 3, 0, false));
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

    #[test]
    fn bounded_probe_preserves_colliding_rules() {
        let path = std::env::temp_dir().join(format!(
            "ramshield-shm-collision-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ShmTableManager::open_or_create(&path).unwrap();
        let a = 7u64;
        let b = a + SHM_TABLE_CAPACITY as u64;
        assert!(manager.publish_rule(a, 60_000, 2, 0, false));
        assert!(manager.publish_rule(b, 60_000, 3, 0, false));
        assert_eq!(
            manager
                .get_slot(a as usize)
                .client_hash
                .load(Ordering::Acquire),
            a
        );
        assert_eq!(
            manager
                .get_slot(a as usize + 1)
                .client_hash
                .load(Ordering::Acquire),
            b
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn ttl_saturates_instead_of_wrapping() {
        let path = std::env::temp_dir().join(format!(
            "ramshield-shm-ttl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = ShmTableManager::open_or_create(&path).unwrap();
        assert!(manager.publish_rule(17, u64::MAX, 3, 0, false));
        assert_eq!(
            manager.get_slot(17).expires_at_ms.load(Ordering::Acquire),
            u64::MAX
        );
        let _ = std::fs::remove_file(path);
    }

    #[cfg(unix)]
    #[test]
    fn shm_file_is_owner_only_on_creation() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "ramshield-shm-mode-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _manager = ShmTableManager::open_or_create(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn concurrent_writers_preserve_even_seq_and_no_torn_hash() {
        use std::sync::Arc;
        use std::thread;

        let path = std::env::temp_dir().join(format!(
            "ramshield-shm-stress-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Arc::new(ShmTableManager::open_or_create(&path).unwrap());
        let mut handles = Vec::new();
        for t_id in 0..8u64 {
            let mgr = Arc::clone(&manager);
            handles.push(thread::spawn(move || {
                for i in 0..200u64 {
                    // Distinct hashes that still collide into nearby probe windows.
                    let hash = (t_id << 16) ^ i.wrapping_mul(0x9E37_79B9);
                    let _ = mgr.publish_rule(hash, 60_000, 3, 0, false);
                }
            }));
        }
        for h in handles {
            h.join().expect("writer thread");
        }

        // Every occupied slot must end on an even seq; client_hash must be
        // stable under a double-read (seqlock reader pattern).
        for idx in 0..SHM_TABLE_CAPACITY {
            let slot = manager.get_slot(idx);
            let s1 = slot.seq.load(Ordering::Acquire);
            if s1 & 1 != 0 {
                // Transient writer should not remain after join.
                panic!("slot {idx} left with odd seq after writers finished");
            }
            let h1 = slot.client_hash.load(Ordering::Acquire);
            let s2 = slot.seq.load(Ordering::Acquire);
            assert_eq!(s1, s2, "seq changed mid-read on slot {idx}");
            if h1 != 0 {
                let h2 = slot.client_hash.load(Ordering::Acquire);
                assert_eq!(h1, h2, "torn client_hash on slot {idx}");
            }
        }
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn concurrent_same_hash_writers_leave_consistent_rule() {
        use std::sync::Arc;
        use std::thread;

        let path = std::env::temp_dir().join(format!(
            "ramshield-shm-samehash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let manager = Arc::new(ShmTableManager::open_or_create(&path).unwrap());
        let hash = 42u64;
        let mut handles = Vec::new();
        for tier in 0..4u8 {
            let mgr = Arc::clone(&manager);
            handles.push(thread::spawn(move || {
                for _ in 0..100 {
                    let _ = mgr.publish_rule(hash, 30_000, tier, 10, false);
                }
            }));
        }
        for h in handles {
            h.join().expect("writer thread");
        }
        let slot = manager.get_slot(hash as usize);
        let seq = slot.seq.load(Ordering::Acquire);
        assert_eq!(seq % 2, 0, "must finish even");
        assert_eq!(
            slot.client_hash.load(Ordering::Acquire),
            hash,
            "same-hash writers must leave the canonical hash"
        );
        let tier = slot.tier.load(Ordering::Acquire);
        assert!(tier <= 3, "tier out of range: {tier}");
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod path_safety {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Unique scratch directory under the system temp dir.
    ///
    /// `ramshield-cgnat` has no `tempfile` dev-dependency, so the test builds
    /// and removes its own directory instead of adding a dependency.
    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ramshield-cgnat-{}-{}-{}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// P1: a symlink planted at the shared-memory path must not be followed.
    ///
    /// `/dev/shm` is world-writable and sticky, so any local user can
    /// pre-plant a symlink named `ramshield_rules` pointing at a file the
    /// daemon can reach. The daemon then opens the target, chmods it to 0600
    /// and resizes it to the rules-table size — destroying an arbitrary file.
    #[test]
    fn open_or_create_refuses_a_symlink_at_the_path() {
        let dir = scratch("symlink");
        let target = dir.join("victim");
        let link = dir.join("ramshield_rules");
        std::fs::write(&target, b"do not touch").expect("write victim");
        std::os::unix::fs::symlink(&target, &link).expect("symlink");

        let before = std::fs::metadata(&target).expect("stat victim");

        let result = ShmTableManager::open_or_create(&link);
        assert!(
            result.is_err(),
            "open_or_create must refuse a symlink at the shared-memory path"
        );

        let after = std::fs::metadata(&target).expect("stat victim");
        assert_eq!(
            before.len(),
            after.len(),
            "the symlink target must not be resized"
        );
        assert_eq!(
            std::fs::read(&target).expect("read victim"),
            b"do not touch",
            "the symlink target content must be intact"
        );
        assert_eq!(
            before.permissions().mode() & 0o777,
            after.permissions().mode() & 0o777,
            "the symlink target mode must not be changed"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// P1: a FIFO (or any non-regular file) at the path must not be used as
    /// the rules table. Opening a FIFO for read+write would block forever.
    #[test]
    fn open_or_create_refuses_a_non_regular_file() {
        let dir = scratch("fifo");
        let path = dir.join("ramshield_rules");
        // A plain file that is not the table is fine, but a directory is not.
        std::fs::create_dir(&path).expect("mkdir");

        let result = ShmTableManager::open_or_create(&path);
        assert!(
            result.is_err(),
            "open_or_create must refuse a non-regular file at the path"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The C slot-key hash and the Rust one must agree.
    ///
    /// A rule published by the Rust writer is looked up by the C reader. If
    /// the two hashes diverge, the reader looks in a slot the writer never
    /// touched, and every rule is silently unenforced. The same vectors are
    /// pinned by `scripts/shm_lookup_contract.c`, so a change to either
    /// implementation breaks one of the two tests.
    #[test]
    fn c_and_rust_slot_key_hash_agree() {
        assert_eq!(subnet_key(0xC000_0200, 24), 0xff30_6298);
        assert_eq!(subnet_key(0xC000_0200, 0), 0xab61_6229);
        // The C ipv4 variant masks the client IP to the prefix first, so
        // these are the masked-network vectors.
        assert_eq!(subnet_key(0xC633_6400, 24), 0x7601_e635);
        assert_eq!(subnet_key(0xC633_6401, 32), 0x4c38_342d);
    }

    /// A rule published under the reserved key 0 would be invisible to every
    /// reader, because a slot with client_hash == 0 is the empty-slot
    /// sentinel. The publisher must refuse it.
    #[test]
    fn publish_refuses_the_reserved_zero_key() {
        // `scratch` returns a directory; the table is a file inside it.
        let path = scratch("zero-key").join("rules.shm");
        let mgr = ShmTableManager::open_or_create(&path).expect("open");
        assert!(
            !mgr.publish_rule(0, 60_000, 2, 0, false),
            "a rule under the reserved key 0 must be refused, not published"
        );
    }
}
