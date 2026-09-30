//! Transport abstraction shared by CoreMIDI (midir) and the simulator.

use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
pub enum TransportError {
    #[error("device disconnected")]
    Disconnected,
    #[error("MIDI input queue overflowed; data was lost")]
    QueueOverflow,
    #[error("MIDI i/o error: {0}")]
    Io(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportKind {
    Usb,
    Din,
    Simulator,
}

/// A bidirectional SysEx transport. `recv` returns complete frames (F0..F7) or
/// malformed-frame notifications as `Err`-free `RecvEvent`s.
pub trait Transport: Send {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError>;
    fn recv(&mut self, timeout: Duration) -> Result<Option<RecvEvent>, TransportError>;
    fn kind(&self) -> TransportKind;
    fn description(&self) -> String;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecvEvent {
    Frame(Vec<u8>),
    Malformed(String),
}

/// Timing profile. Engineering defaults, not manufacturer guarantees; tune on hardware.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportProfile {
    pub read_timeout_ms: u64,
    pub read_retries: u32,
    pub inter_request_ms: u64,
    pub post_write_settle_ms: u64,
    /// Serial bits per second if the link is rate-limited (DIN = 31250). None for USB.
    pub serial_bps: Option<u32>,
    pub max_write_attempts: u32,
}

impl TransportProfile {
    pub fn usb() -> Self {
        Self { read_timeout_ms: 2500, read_retries: 2, inter_request_ms: 30, post_write_settle_ms: 120, serial_bps: None, max_write_attempts: 3 }
    }
    pub fn din() -> Self {
        Self { read_timeout_ms: 4000, read_retries: 2, inter_request_ms: 50, post_write_settle_ms: 120, serial_bps: Some(31250), max_write_attempts: 3 }
    }
    pub fn simulator() -> Self {
        Self { read_timeout_ms: 500, read_retries: 2, inter_request_ms: 0, post_write_settle_ms: 0, serial_bps: None, max_write_attempts: 3 }
    }
    pub fn for_kind(k: TransportKind) -> Self {
        match k {
            TransportKind::Usb => Self::usb(),
            TransportKind::Din => Self::din(),
            TransportKind::Simulator => Self::simulator(),
        }
    }
    /// Time a frame occupies on the wire (10 serial bits per byte).
    pub fn wire_time(&self, bytes: usize) -> Duration {
        match self.serial_bps {
            Some(bps) => Duration::from_micros((bytes as u64 * 10 * 1_000_000) / bps as u64),
            None => Duration::ZERO,
        }
    }
    /// Rough estimate for reading `n` programs.
    pub fn estimate_read(&self, n: usize) -> Duration {
        let per = self.wire_time(7) + self.wire_time(1178) + Duration::from_millis(self.inter_request_ms + if self.serial_bps.is_none() { 25 } else { 0 });
        per * n as u32
    }
    pub fn estimate_write(&self, n: usize) -> Duration {
        let per = self.estimate_read(2) + self.wire_time(1178) + Duration::from_millis(self.post_write_settle_ms);
        per * n as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn din_timing() {
        let p = TransportProfile::din();
        assert_eq!(p.wire_time(1178).as_millis(), 376);
        assert!(p.estimate_read(500).as_secs() >= 188);
    }
}
