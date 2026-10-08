//! Non-API scaffolding for building hostile WAL fixtures in tests.
//!
//! This module exists so the crash laboratory and golden-fixture oracles can
//! construct exact on-disk bytes without reimplementing the format — which
//! would make a second, divergent parser. It is `#[doc(hidden)]` because it
//! is not consumer API: applications append through [`crate::Wal`].
//!
//! The format itself is frozen and public in `docs/FORMAT.md`; the
//! constructors are not.

use std::io::{self, Read, Seek, SeekFrom};

pub use crate::lsn::Lsn;
pub use crate::record::{
    FLAG_COMPRESSED, FORMAT_VERSION, HEADER_SIZE, MAX_RECORD_SIZE, RECORD_MAGIC, RecordHeader,
    encode_payload,
};
pub use crate::segment::{MANIFEST_NAME, QUARANTINE_DIR, SEG_EXT, parse_seg_idx, seg_path};

/// A reader that returns real bytes for the first `ok_bytes`, then fails with
/// a non-EOF error.
///
/// This is how the "an I/O failure is not a torn tail" rule is tested: the
/// scanner must see the same `Err` a failing disk would produce, not a
/// simulated result. EOF is deliberately *not* what it returns.
pub struct FailingReader<R> {
    inner: R,
    ok_bytes: u64,
    kind: io::ErrorKind,
}

impl<R: Read + Seek> FailingReader<R> {
    pub fn new(inner: R, ok_bytes: u64, kind: io::ErrorKind) -> Self {
        Self {
            inner,
            ok_bytes,
            kind,
        }
    }
}

impl<R: Read + Seek> Read for FailingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.inner.stream_position()? >= self.ok_bytes {
            return Err(io::Error::new(self.kind, "injected read failure"));
        }
        self.inner.read(buf)
    }
}

impl<R: Seek> Seek for FailingReader<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.inner.seek(pos)
    }
}
