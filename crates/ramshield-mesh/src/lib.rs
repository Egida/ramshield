//! Fleet federation: AWORSet CRDT + authenticated TCP transport.
pub mod aworset;
/// Condition-driven anti-entropy scheduling prototype; not wired into transport.
pub mod condition_sync;
pub mod hlc;
pub mod transport;
pub use transport::{MeshHandle, MeshMessage};
