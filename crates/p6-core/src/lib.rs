//! P6 Vault core: everything that decides bytes, bank contents and write permission.

pub mod classification;
pub mod deployment;
pub mod device;
pub mod library;
pub mod protocol;
pub mod simulator;
pub mod slot;
pub mod storage;
pub mod util;
pub mod workspace;

pub use protocol::payload::Payload;
pub use slot::{StoredAddress, UserSlot};
