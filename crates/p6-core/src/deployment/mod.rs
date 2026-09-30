//! Hardware reads, pre-write backups, frozen plans, the guarded WriteEngine and recovery.

pub mod backup;
pub mod permit;
pub mod plan;
pub mod recovery;
pub mod sync;
pub mod writer;

use crate::device::DeviceError;
use crate::storage::VaultError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Progress {
    pub phase: String,
    pub done: usize,
    pub total: usize,
    pub slot: Option<u16>,
    pub message: Option<String>,
}

impl Progress {
    pub fn new(phase: &str, done: usize, total: usize, slot: Option<u16>) -> Self {
        Self { phase: phase.into(), done, total, slot, message: None }
    }
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(tag = "code", content = "message")]
pub enum DeployError {
    #[error("{0}")]
    Device(#[from] DeviceError),
    #[error("{0}")]
    Vault(#[from] VaultError),
    #[error("the bank read is incomplete: {missing} slot(s) missing")]
    IncompleteBank { missing: usize },
    #[error("the synth changed since Current was captured ({slots} slot(s)); reconcile before writing")]
    HardwareDrift { slots: usize },
    #[error("backup failed: {0}")]
    BackupFailed(String),
    #[error("journal failed: {0}")]
    JournalFailed(String),
    #[error("verification mismatch at slot {slot:03} ({differing} bytes differ)")]
    VerificationMismatch { slot: u16, differing: usize },
    #[error("write permit is not valid: {0}")]
    InvalidPermit(String),
    #[error("{0}")]
    Invalid(String),
    #[error("cancelled")]
    Cancelled,
    #[error(
        "hardware validation is not complete ({done}/{required} single-slot tests). Until then, real writes are limited to exactly one changed slot (New has {changed}). See docs/HARDWARE-TESTS.md."
    )]
    HardwareGate { done: usize, required: usize, changed: usize },
}

pub type DResult<T> = Result<T, DeployError>;
