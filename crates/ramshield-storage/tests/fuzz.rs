//! WAL fuzz harness — random bytes in, must never panic.
//! Run: `cargo test -p ramshield-storage --test fuzz`
use proptest::prelude::*;
use ramshield_storage::wal::Wal;
use ramshield_types::Durability;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any byte slice fed to WAL segment must produce Ok or Err — never panic.
    #[test]
    fn wal_segment_never_panics(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let dir = std::env::temp_dir().join(format!(
            "ramshield-wal-fuzz-{}", std::process::id()
        ));
        std::fs::create_dir(&dir).ok();
        let seg = dir.join("wal-00000001.log");
        let _ = std::fs::write(&seg, &data);
        let d: &str = dir.to_str().unwrap();
        let _ = Wal::open(d, false, Durability::None, 4096, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
