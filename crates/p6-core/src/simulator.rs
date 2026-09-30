//! Deterministic simulated Prophet-6 implementing [`Transport`].
//!
//! It owns 1000 stored programs (500 user + 500 factory) and an independent edit
//! buffer, speaks through the real packing/framing code, records every send, and
//! can inject faults: dropped/late replies, fragmentation with real-time bytes,
//! store mismatches, disconnects and external drift. Synthetic payloads are test
//! data and must never be sent to a real synth.

use crate::device::{RecvEvent, Transport, TransportError, TransportKind};
use crate::protocol::framing::{FrameAssembler, FrameEvent, MIDI_MAX_FRAME};
use crate::protocol::messages::{edit_buffer_frame, parse_message, program_file_frame, P6Message};
use crate::protocol::payload::{synthetic_payload, Payload};
use crate::slot::{StoredAddress, UserSlot};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Default)]
pub struct SimControl {
    pub drop_replies: u32,
    /// Hold the next N replies until after the following request (late replies).
    pub delay_replies: u32,
    pub fragment_replies: bool,
    pub corrupt_store_slots: HashSet<u16>,
    pub ignore_store_slots: HashSet<u16>,
    /// Ignore the next N stored writes (write does not land), then behave normally.
    pub ignore_next_stores: u32,
    /// Disconnect after this many further stored writes are processed.
    pub disconnect_after_stores: Option<u32>,
    pub disconnect_now: bool,
    pub sent: Vec<Vec<u8>>,
    pub stores: Vec<u16>,
    pub edit_buffer_loads: u32,
}

pub struct SimState {
    pub programs: Vec<Payload>,
    pub edit_buffer: Payload,
}

pub struct SimulatedP6 {
    pub state: Arc<Mutex<SimState>>,
    pub control: Arc<Mutex<SimControl>>,
    inbox: VecDeque<RecvEvent>,
    held: VecDeque<Vec<u8>>,
    asm: FrameAssembler,
    persist: Option<PathBuf>,
    rng: u64,
}

pub fn factory_like_bank() -> Vec<Payload> {
    const NAMES: &[&str] = &[
        "Sim Bass",
        "Sim Lead",
        "Sim Pad",
        "Sim Keys",
        "Sim Pluck",
        "Sim Arp",
        "Sim Texture",
        "Sim Brass",
        "Sim Strings",
        "Sim Sub Bass",
    ];
    (0..1000u32)
        .map(|i| synthetic_payload(i + 1, &format!("{} {:03}", NAMES[(i % 10) as usize], i)))
        .collect()
}

impl SimulatedP6 {
    pub fn new() -> Self {
        let programs = factory_like_bank();
        let edit_buffer = programs[0].clone();
        Self::with_state(SimState {
            programs,
            edit_buffer,
        })
    }

    pub fn with_state(state: SimState) -> Self {
        Self {
            state: Arc::new(Mutex::new(state)),
            control: Arc::new(Mutex::new(SimControl::default())),
            inbox: VecDeque::new(),
            held: VecDeque::new(),
            asm: FrameAssembler::new(MIDI_MAX_FRAME),
            persist: None,
            rng: 0x9E3779B97F4A7C15,
        }
    }

    /// A second transport onto the same simulated synth (models a reconnect).
    pub fn reconnect(&self) -> Self {
        let mut s = Self::with_state(SimState {
            programs: vec![],
            edit_buffer: self.state.lock().unwrap().edit_buffer.clone(),
        });
        s.state = self.state.clone();
        s.control = self.control.clone();
        s.control.lock().unwrap().disconnect_now = false;
        s.persist = self.persist.clone();
        s
    }

    /// Load simulator memory from a separate path (never the real Vault database).
    pub fn persistent(path: PathBuf) -> Self {
        let mut sim = Self::new();
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(prev) = crate::library::import::preview_import("sim", &bytes) {
                let mut st = sim.state.lock().unwrap();
                for o in prev.occurrences {
                    if let Some(a) = o.address {
                        st.programs[a.absolute() as usize] = o.payload;
                    }
                }
            }
        }
        sim.persist = Some(path);
        sim
    }

    fn save(&self) {
        if let Some(p) = &self.persist {
            let st = self.state.lock().unwrap();
            let bank: Vec<Option<Payload>> = st.programs[..500].iter().cloned().map(Some).collect();
            if let Ok(bytes) = crate::library::export::bank_bytes(&bank) {
                let _ = crate::library::export::write_verified(
                    p,
                    &bytes,
                    &crate::library::export::Expected::Raw,
                );
            }
        }
    }

    /// Simulate the owner (or another librarian) storing a program on the synth.
    pub fn external_store(&self, slot: u16, p: Payload) {
        self.state.lock().unwrap().programs[slot as usize] = p;
    }

    pub fn user_bank(&self) -> Vec<Payload> {
        self.state.lock().unwrap().programs[..500].to_vec()
    }

    fn next_rand(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    fn reply(&mut self, frame: Vec<u8>) {
        let mut c = self.control.lock().unwrap();
        if c.drop_replies > 0 {
            c.drop_replies -= 1;
            return;
        }
        if c.delay_replies > 0 {
            c.delay_replies -= 1;
            self.held.push_back(frame);
            return;
        }
        let frag = c.fragment_replies;
        drop(c);
        self.deliver(frame, frag);
    }

    fn deliver(&mut self, frame: Vec<u8>, fragment: bool) {
        if !fragment {
            self.inbox.push_back(RecvEvent::Frame(frame));
            return;
        }
        // Push through a real assembler in random chunks with interleaved real-time bytes.
        let mut out = Vec::new();
        let mut i = 0;
        while i < frame.len() {
            let n = 1 + (self.next_rand() % 97) as usize;
            let end = (i + n).min(frame.len());
            self.asm.push(&frame[i..end], &mut out);
            self.asm.push(&[0xF8], &mut out);
            i = end;
        }
        for e in out {
            self.inbox.push_back(match e {
                FrameEvent::Frame { bytes, .. } => RecvEvent::Frame(bytes),
                FrameEvent::Malformed { reason, .. } => RecvEvent::Malformed(format!("{reason:?}")),
            });
        }
    }

    fn handle(&mut self, frame: &[u8]) {
        // Late replies from earlier requests arrive after this new request.
        while let Some(h) = self.held.pop_front() {
            self.inbox.push_back(RecvEvent::Frame(h));
        }
        if frame.len() == 6 && frame[1] == 0x7E && frame[3] == 0x06 && frame[4] == 0x01 {
            self.reply(vec![
                0xF0, 0x7E, 0x00, 0x06, 0x02, 0x01, 0x2D, 0x01, 0x00, 0x00, 0x01, 0x05, 0x00, 0xF7,
            ]);
            return;
        }
        match parse_message(frame) {
            Ok(P6Message::ProgramRequest { address }) => {
                let p = self.state.lock().unwrap().programs[address.absolute() as usize].clone();
                self.reply(program_file_frame(address, &p));
            }
            Ok(P6Message::EditBufferRequest) => {
                let p = self.state.lock().unwrap().edit_buffer.clone();
                self.reply(edit_buffer_frame(&p));
            }
            Ok(P6Message::EditBufferData { payload, .. }) => {
                self.state.lock().unwrap().edit_buffer = payload;
                self.control.lock().unwrap().edit_buffer_loads += 1;
            }
            Ok(P6Message::ProgramData {
                address, payload, ..
            }) => {
                let slot = address.absolute();
                let mut c = self.control.lock().unwrap();
                c.stores.push(slot);
                if let Some(n) = c.disconnect_after_stores {
                    if n == 0 {
                        c.disconnect_after_stores = None;
                        c.disconnect_now = true;
                        return;
                    }
                    c.disconnect_after_stores = Some(n - 1);
                }
                if c.ignore_next_stores > 0 {
                    c.ignore_next_stores -= 1;
                    return;
                }
                if c.ignore_store_slots.contains(&slot) || slot >= 500 {
                    return;
                }
                let stored = if c.corrupt_store_slots.contains(&slot) {
                    let mut b = *payload.bytes();
                    b[1000] ^= 0x55;
                    Payload::from_slice(&b).unwrap()
                } else {
                    payload
                };
                drop(c);
                self.state.lock().unwrap().programs[slot as usize] = stored;
                self.save();
            }
            _ => {}
        }
    }
}

impl Default for SimulatedP6 {
    fn default() -> Self {
        Self::new()
    }
}

impl Transport for SimulatedP6 {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        {
            let mut c = self.control.lock().unwrap();
            if c.disconnect_now {
                return Err(TransportError::Disconnected);
            }
            c.sent.push(bytes.to_vec());
        }
        if bytes.first() == Some(&0xF0) {
            self.handle(bytes);
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<RecvEvent>, TransportError> {
        if self.control.lock().unwrap().disconnect_now {
            return Err(TransportError::Disconnected);
        }
        if let Some(e) = self.inbox.pop_front() {
            return Ok(Some(e));
        }
        // Nothing will arrive spontaneously; emulate waiting without burning test time.
        std::thread::sleep(timeout.min(Duration::from_millis(2)));
        Ok(None)
    }

    fn kind(&self) -> TransportKind {
        TransportKind::Simulator
    }

    fn description(&self) -> String {
        "Simulated Prophet-6".into()
    }
}

/// Convenience: typed user slot helper for tests.
pub fn slot(n: u16) -> UserSlot {
    UserSlot::new(n).unwrap()
}

pub fn addr(n: u16) -> StoredAddress {
    StoredAddress::from_absolute(n).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::{Device, DeviceError, TransportProfile};

    fn dev(sim: SimulatedP6) -> Device {
        Device::new(Box::new(sim), TransportProfile::simulator(), 1)
    }

    #[test]
    fn probe_and_read() {
        let sim = SimulatedP6::new();
        let bank = sim.user_bank();
        let mut d = dev(sim);
        let p = d.probe().unwrap();
        assert_eq!(p.identity_family.unwrap()[0], 0x2D);
        assert_eq!(d.read_program(addr(123), None).unwrap(), bank[123]);
    }

    #[test]
    fn fragmented_replies_parse() {
        let sim = SimulatedP6::new();
        sim.control.lock().unwrap().fragment_replies = true;
        let bank = sim.user_bank();
        let mut d = dev(sim);
        for i in [0u16, 1, 250, 499] {
            assert_eq!(d.read_program(addr(i), None).unwrap(), bank[i as usize]);
        }
    }

    #[test]
    fn dropped_reply_retries_and_late_reply_ignored() {
        let sim = SimulatedP6::new();
        let bank = sim.user_bank();
        sim.control.lock().unwrap().drop_replies = 1;
        let mut d = dev(sim);
        assert_eq!(d.read_program(addr(5), None).unwrap(), bank[5]);
    }

    #[test]
    fn late_reply_for_other_address_not_accepted() {
        let sim = SimulatedP6::new();
        let bank = sim.user_bank();
        let ctl = sim.control.clone();
        let mut d = dev(sim);
        ctl.lock().unwrap().delay_replies = 1;
        // Request 10 -> reply held. Retry request 10 -> held reply (10) arrives, then fresh reply.
        assert_eq!(d.read_program(addr(10), None).unwrap(), bank[10]);
        // Next request for 11 must not accept a stale 10.
        assert_eq!(d.read_program(addr(11), None).unwrap(), bank[11]);
    }

    #[test]
    fn all_drop_is_unresponsive() {
        let sim = SimulatedP6::new();
        sim.control.lock().unwrap().drop_replies = 10;
        let mut d = dev(sim);
        assert_eq!(
            d.read_program(addr(1), None),
            Err(DeviceError::DeviceUnresponsive { attempts: 3 })
        );
    }

    #[test]
    fn audition_uses_command_03_only() {
        let sim = SimulatedP6::new();
        let ctl = sim.control.clone();
        let st = sim.state.clone();
        let mut d = dev(sim);
        let p = synthetic_payload(999, "Audition");
        d.load_edit_buffer(&p).unwrap();
        assert_eq!(st.lock().unwrap().edit_buffer, p);
        let c = ctl.lock().unwrap();
        assert!(c.stores.is_empty());
        assert!(c
            .sent
            .iter()
            .all(|f| !crate::protocol::messages::is_stored_write(f)));
    }

    #[test]
    fn disconnect_reports() {
        let sim = SimulatedP6::new();
        sim.control.lock().unwrap().disconnect_now = true;
        let mut d = dev(sim);
        assert_eq!(
            d.read_program(addr(1), None),
            Err(DeviceError::Disconnected)
        );
    }
}
