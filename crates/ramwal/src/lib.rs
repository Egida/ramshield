//! RAMWAL — a recovery-first append-only log.
//!
//! A WAL is easy to write. Recovery is the product.
//!
//! The two rules everything derives from:
//!
//! ```text
//! INCOMPLETE (only at EOF, final segment)  →  safe to repair
//! INVALID                                  →  stop, report corruption
//! ```
//!
//! Recovery runs inside [`Wal::open`] and owns the append boundary. The
//! writer never independently discovers where to write.
//!
//! ```
//! use ramwal::{Config, Durability, Wal};
//!
//! let mut cfg = Config::new("./example-wal");
//! cfg.durability = Durability::Explicit;
//! let wal = Wal::open(cfg)?;
//!
//! let lsn = wal.append(b"increment")?;
//! wal.sync()?;                  // lsn is now durable
//! wal.checkpoint(lsn)?;         // app has persisted a snapshot through lsn
//! wal.truncate_before(lsn)?;    // may now drop older segments
//! # Ok::<(), ramwal::Error>(())
//! ```

mod record;
mod segment;

#[doc(hidden)]
pub mod testkit;

pub mod config;
pub mod error;
pub mod lsn;
pub mod recovery;
pub mod wal;

pub use config::{Compression, Config, Durability};
pub use error::{Corruption, Error, TailState};
pub use lsn::Lsn;
pub use recovery::{RecoveredRecord, RecoveryReport};
pub use wal::Wal;
