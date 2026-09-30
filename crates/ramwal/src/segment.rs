/// Segment management — naming, discovery, I/O.
use std::fs::{File, OpenOptions};
use std::path::PathBuf;

pub const SEG_EXT: &str = "rwl";
pub const MANIFEST_NAME: &str = "MANIFEST";
pub const QUARANTINE_DIR: &str = "quarantine";

/// Build segment path from directory and 0‑padded index.
pub fn seg_path(dir: &str, idx: u64) -> PathBuf {
    PathBuf::from(dir).join(format!("segment-{:020}.{}", idx, SEG_EXT))
}

/// Parse segment index from "segment-00000000000000000001.rwl".
pub fn parse_seg_idx(name: &str) -> Option<u64> {
    let s = name.strip_prefix("segment-")?;
    let s = s.strip_suffix(".rwl")?;
    s.parse::<u64>().ok()
}

pub fn list_segs(dir: &str) -> std::io::Result<Vec<PathBuf>> {
    let mut segs = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;

        if let Some(idx) = parse_seg_idx(&entry.file_name().to_string_lossy()) {
            segs.push((idx, entry.path()));
        }
    }

    segs.sort_by_key(|&(idx, _)| idx);

    Ok(segs.into_iter().map(|(_, path)| path).collect())
}

pub fn list_segs_with_size(dir: &str) -> std::io::Result<Vec<(u64, PathBuf, u64)>> {
    let mut segs = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;

        if let Some(idx) = parse_seg_idx(&entry.file_name().to_string_lossy()) {
            let path = entry.path();
            let size = entry.metadata()?.len();
            segs.push((idx, path, size));
        }
    }

    segs.sort_by_key(|&(idx, _, _)| idx);

    Ok(segs)
}

pub fn open_segment(path: &PathBuf) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

pub fn open_segment_read(path: &PathBuf) -> std::io::Result<File> {
    File::open(path)
}

pub fn open_segment_write(path: &PathBuf) -> std::io::Result<File> {
    OpenOptions::new().write(true).open(path)
}

/// Create (or truncate) a segment file with owner-only permissions
/// where the platform supports them.
///
/// `ponytail:` one `#[cfg]` instead of the runbook's `storage/{unix,linux,
/// macos,windows}.rs` trait tree — the only platform difference here is
/// whether the mode bits exist. Split into a `DurabilityBackend` trait when
/// a second real difference appears (e.g. macOS `F_FULLFSYNC`).
pub fn create_segment_truncate(path: &PathBuf) -> std::io::Result<File> {
    let mut opts = OpenOptions::new();
    opts.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)
}

/// Flush and fsync a directory so a newly created or renamed entry is durable.
#[cfg(unix)]
pub fn fsync_dir(dir: &str) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

/// Windows cannot open a directory as a file; the platform has no
/// equivalent of `fsync(dir)`. Entry durability is the filesystem's
/// responsibility there.
#[cfg(not(unix))]
pub fn fsync_dir(_dir: &str) -> std::io::Result<()> {
    Ok(())
}
