//! Serialized device transactions over a [`Transport`].

pub mod actor;
pub mod transport;

use crate::protocol::messages::{self, is_stored_write, parse_message, P6Message};
use crate::protocol::payload::Payload;
use crate::slot::{StoredAddress, UserSlot};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
pub use transport::{RecvEvent, Transport, TransportError, TransportKind, TransportProfile};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code", content = "detail")]
pub enum DeviceError {
    #[error("the Prophet-6 did not answer (timed out after {attempts} attempt(s))")]
    DeviceUnresponsive { attempts: u32 },
    #[error("device disconnected")]
    Disconnected,
    #[error("MIDI input overflowed; the transaction is not trustworthy")]
    QueueOverflow,
    #[error("cancelled")]
    Cancelled,
    #[error("MIDI i/o error: {0}")]
    Io(String),
    #[error("refused: {0}")]
    Refused(String),
    #[error("no valid Prophet-6 reply: {0}")]
    NotAProphet6(String),
}

impl From<TransportError> for DeviceError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Disconnected => DeviceError::Disconnected,
            TransportError::QueueOverflow => DeviceError::QueueOverflow,
            TransportError::Io(s) => DeviceError::Io(s),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagEntry {
    pub at_ms: i64,
    pub direction: &'static str,
    pub summary: String,
    pub bytes: usize,
    pub attempt: u32,
    pub elapsed_ms: u64,
    pub result: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProbeResult {
    pub identity_family: Option<Vec<u8>>,
    pub identity_version: Option<Vec<u8>>,
    pub program_read_ok: bool,
    pub description: String,
}

/// Owns a transport. All MIDI goes through here; stored writes are only reachable
/// through the crate-private [`Device::transmit_stored_program`].
pub struct Device {
    transport: Box<dyn Transport>,
    pub profile: TransportProfile,
    epoch: u64,
    diag: VecDeque<DiagEntry>,
    pub quarantined: u64,
    held_notes: Vec<(u8, u8)>,
}

const DIAG_MAX: usize = 2000;

impl Device {
    pub fn new(transport: Box<dyn Transport>, profile: TransportProfile, epoch: u64) -> Self {
        Self {
            transport,
            profile,
            epoch,
            diag: VecDeque::new(),
            quarantined: 0,
            held_notes: Vec::new(),
        }
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
    pub fn kind(&self) -> TransportKind {
        self.transport.kind()
    }
    pub fn description(&self) -> String {
        self.transport.description()
    }
    pub fn diagnostics(&self) -> Vec<DiagEntry> {
        self.diag.iter().cloned().collect()
    }

    fn log(
        &mut self,
        direction: &'static str,
        summary: String,
        bytes: usize,
        attempt: u32,
        started: Instant,
        result: &str,
    ) {
        if self.diag.len() >= DIAG_MAX {
            self.diag.pop_front();
        }
        self.diag.push_back(DiagEntry {
            at_ms: crate::util::now_ms(),
            direction,
            summary,
            bytes,
            attempt,
            elapsed_ms: started.elapsed().as_millis() as u64,
            result: result.to_string(),
        });
    }

    /// The single choke point for outgoing bytes. Refuses command 02 unless `store_ok`.
    fn send_frame(
        &mut self,
        frame: &[u8],
        store_ok: bool,
        summary: String,
    ) -> Result<(), DeviceError> {
        if is_stored_write(frame) && !store_ok {
            return Err(DeviceError::Refused(
                "stored-program writes are only allowed through the write engine".into(),
            ));
        }
        let t = Instant::now();
        let r = self.transport.send(frame);
        self.log(
            "out",
            summary,
            frame.len(),
            1,
            t,
            if r.is_ok() { "sent" } else { "error" },
        );
        r?;
        std::thread::sleep(self.profile.wire_time(frame.len()));
        Ok(())
    }

    /// Discard any queued input (stale/late replies), counting it as quarantined.
    pub fn drain(&mut self, settle: Duration) -> Result<usize, DeviceError> {
        let mut n = 0;
        let end = Instant::now() + settle;
        while self
            .transport
            .recv(end.saturating_duration_since(Instant::now()))?
            .is_some()
        {
            n += 1;
        }
        if n > 0 {
            self.quarantined += n as u64;
            self.log(
                "in",
                format!("quarantined {n} stale frame(s)"),
                0,
                0,
                Instant::now(),
                "drained",
            );
        }
        Ok(n)
    }

    fn wait_for<T>(
        &mut self,
        deadline: Instant,
        mut want: impl FnMut(&P6Message) -> Option<T>,
    ) -> Result<Option<T>, DeviceError> {
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            let slice = (deadline - now).min(Duration::from_millis(100));
            match self.transport.recv(slice)? {
                Some(RecvEvent::Frame(f)) => match parse_message(&f) {
                    Ok(m) => {
                        if let Some(v) = want(&m) {
                            return Ok(Some(v));
                        }
                        self.quarantined += 1;
                    }
                    Err(_) => self.quarantined += 1,
                },
                Some(RecvEvent::Malformed(_)) => self.quarantined += 1,
                None => {}
            }
        }
    }

    fn inter_request(&self) {
        std::thread::sleep(Duration::from_millis(self.profile.inter_request_ms));
    }

    pub fn probe(&mut self) -> Result<ProbeResult, DeviceError> {
        self.drain(Duration::from_millis(20))?;
        let t = Instant::now();
        self.send_frame(
            &messages::identity_inquiry(),
            false,
            "identity inquiry".into(),
        )?;
        let deadline =
            Instant::now() + Duration::from_millis(self.profile.read_timeout_ms.min(1500));
        let ident = self.wait_for(deadline, |m| match m {
            P6Message::IdentityReply {
                family, version, ..
            } => Some((family.clone(), version.clone())),
            _ => None,
        })?;
        self.log(
            "in",
            "identity reply".into(),
            0,
            1,
            t,
            if ident.is_some() { "ok" } else { "no reply" },
        );
        self.inter_request();
        let prog_ok = self
            .read_program(StoredAddress::new(0, 0).unwrap(), None)
            .is_ok();
        if ident.is_none() && !prog_ok {
            return Err(DeviceError::NotAProphet6(
                "no identity reply and no valid program dump".into(),
            ));
        }
        Ok(ProbeResult {
            identity_family: ident.as_ref().map(|i| i.0.clone()),
            identity_version: ident.map(|i| i.1),
            program_read_ok: prog_ok,
            description: self.transport.description(),
        })
    }

    /// Read one stored program with bounded retries. Only a ProgramData reply for
    /// exactly this address is accepted. Cancellation never interrupts an in-flight
    /// request; it only prevents further retries.
    pub fn read_program(
        &mut self,
        addr: StoredAddress,
        cancel: Option<&AtomicBool>,
    ) -> Result<Payload, DeviceError> {
        let attempts = 1 + self.profile.read_retries;
        for attempt in 1..=attempts {
            if attempt > 1 {
                if cancel.is_some_and(|c| c.load(Ordering::Relaxed)) {
                    return Err(DeviceError::Cancelled);
                }
                // Drain late replies to the timed-out request before retrying.
                self.drain(Duration::from_millis(
                    self.profile.inter_request_ms.max(50) * 2,
                ))?;
            }
            let t = Instant::now();
            self.send_frame(
                &messages::request_program(addr),
                false,
                format!("request program {addr}"),
            )?;
            let deadline = Instant::now() + Duration::from_millis(self.profile.read_timeout_ms);
            let got = self.wait_for(deadline, |m| match m {
                P6Message::ProgramData {
                    address, payload, ..
                } if *address == addr => Some(payload.clone()),
                _ => None,
            })?;
            self.log(
                "in",
                format!("program {addr}"),
                1178,
                attempt,
                t,
                if got.is_some() { "ok" } else { "timeout" },
            );
            if let Some(p) = got {
                return Ok(p);
            }
        }
        Err(DeviceError::DeviceUnresponsive { attempts })
    }

    pub fn read_edit_buffer(&mut self) -> Result<Payload, DeviceError> {
        let attempts = 1 + self.profile.read_retries;
        for attempt in 1..=attempts {
            if attempt > 1 {
                self.drain(Duration::from_millis(100))?;
            }
            let t = Instant::now();
            self.send_frame(
                &messages::request_edit_buffer(),
                false,
                "request edit buffer".into(),
            )?;
            let deadline = Instant::now() + Duration::from_millis(self.profile.read_timeout_ms);
            let got = self.wait_for(deadline, |m| match m {
                P6Message::EditBufferData { payload, .. } => Some(payload.clone()),
                _ => None,
            })?;
            self.log(
                "in",
                "edit buffer".into(),
                1176,
                attempt,
                t,
                if got.is_some() { "ok" } else { "timeout" },
            );
            if let Some(p) = got {
                return Ok(p);
            }
        }
        Err(DeviceError::DeviceUnresponsive { attempts })
    }

    /// Audition: load a payload into the edit buffer (command 03). Never stores.
    pub fn load_edit_buffer(&mut self, p: &Payload) -> Result<(), DeviceError> {
        self.release_notes()?;
        self.send_frame(
            &messages::edit_buffer_frame(p),
            false,
            format!(
                "edit buffer load '{}'",
                p.display_name().unwrap_or_default()
            ),
        )?;
        std::thread::sleep(Duration::from_millis(self.profile.inter_request_ms));
        Ok(())
    }

    /// Crate-private stored write. Only `deployment::writer` holds the permit type needed to call it.
    pub(crate) fn transmit_stored_program(
        &mut self,
        _permit: &crate::deployment::permit::ConfirmedWritePermit,
        slot: UserSlot,
        p: &Payload,
    ) -> Result<(), DeviceError> {
        self.release_notes()?;
        let frame = messages::stored_write_frame(slot, p);
        self.send_frame(&frame, true, format!("STORE program {slot}"))?;
        std::thread::sleep(Duration::from_millis(self.profile.post_write_settle_ms));
        Ok(())
    }

    pub fn note_on(&mut self, channel: u8, note: u8, velocity: u8) -> Result<(), DeviceError> {
        let ch = channel.clamp(1, 16) - 1;
        self.transport
            .send(&[0x90 | ch, note & 0x7F, velocity & 0x7F])?;
        self.held_notes.push((ch, note));
        Ok(())
    }
    pub fn release_notes(&mut self) -> Result<(), DeviceError> {
        for (ch, n) in std::mem::take(&mut self.held_notes) {
            self.transport.send(&[0x80 | ch, n, 0])?;
        }
        Ok(())
    }
    /// All Notes Off + explicit Note Off for app-owned notes on the configured channel only.
    pub fn panic(&mut self, channel: u8) -> Result<(), DeviceError> {
        self.release_notes()?;
        let ch = channel.clamp(1, 16) - 1;
        self.transport.send(&[0xB0 | ch, 123, 0])?;
        self.transport.send(&[0xB0 | ch, 64, 0])?;
        Ok(())
    }
}
