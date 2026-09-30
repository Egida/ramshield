#[derive(Clone, Copy, Debug)]
pub enum Durability {
    /// Bytes remain buffered. No durability barrier is issued.
    Buffered,

    /// Append writes to the userspace buffer. The caller controls
    /// durability explicitly with `sync()`.
    Explicit,

    /// Every successful append crosses a durability barrier before
    /// returning.
    SyncEach,

    /// Appends opportunistically issue a shared durability barrier when
    /// the group window is due. Call `sync()` when explicit completion
    /// is required.
    GroupCommit,
}

#[derive(Clone, Copy, Debug)]
pub enum Compression {
    None,
    Lz4,
}

/// User configuration.
///
/// The on-disk record limit is part of the format (`record::MAX_RECORD_SIZE`)
/// and is deliberately not configurable here: there is one authority.
#[derive(Clone, Debug)]
pub struct Config {
    pub dir: String,
    pub durability: Durability,
    pub compression: Compression,

    /// Rotation threshold, not a hard file-size limit. A record is never
    /// split across segments, so a segment may exceed this by one record.
    pub seg_max_bytes: u64,

    /// Size target for retention. See `docs/CHECKPOINTING.md`: bytes never
    /// authorise deleting history that no checkpoint covers.
    pub retention_max_bytes: u64,

    pub buffer_capacity: usize,
}

impl Config {
    pub fn new(dir: &str) -> Self {
        Self {
            dir: dir.to_string(),
            durability: Durability::Explicit,
            compression: Compression::None,
            seg_max_bytes: 64 * 1024 * 1024,
            retention_max_bytes: 512 * 1024 * 1024,
            buffer_capacity: 64 * 1024,
        }
    }
}
