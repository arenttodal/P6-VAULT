//! CoreMIDI transport via midir. The input callback only copies bytes into a bounded
//! queue; framing/parsing happens on the MIDI actor thread.

use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use p6_core::device::{RecvEvent, Transport, TransportError, TransportKind};
use p6_core::protocol::framing::{FrameAssembler, FrameEvent, MIDI_MAX_FRAME};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

const QUEUE_CAPACITY: usize = 8192;

#[derive(Debug, Clone, Serialize)]
pub struct PortList {
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub suggested_input: Option<String>,
    pub suggested_output: Option<String>,
}

fn looks_like_p6(name: &str) -> bool {
    let n = name.to_lowercase().replace([' ', '-', '_'], "");
    n.contains("prophet6")
}

pub fn list_ports() -> Result<PortList, String> {
    let mi = MidiInput::new("P6 Vault probe").map_err(|e| e.to_string())?;
    let mo = MidiOutput::new("P6 Vault probe").map_err(|e| e.to_string())?;
    let inputs: Vec<String> = mi.ports().iter().filter_map(|p| mi.port_name(p).ok()).collect();
    let outputs: Vec<String> = mo.ports().iter().filter_map(|p| mo.port_name(p).ok()).collect();
    Ok(PortList {
        suggested_input: inputs.iter().find(|n| looks_like_p6(n)).cloned(),
        suggested_output: outputs.iter().find(|n| looks_like_p6(n)).cloned(),
        inputs,
        outputs,
    })
}

pub struct MidirTransport {
    out: MidiOutputConnection,
    _input: MidiInputConnection<()>,
    rx: Receiver<Vec<u8>>,
    overflow: Arc<AtomicBool>,
    asm: FrameAssembler,
    ready: VecDeque<RecvEvent>,
    kind: TransportKind,
    desc: String,
}

impl MidirTransport {
    pub fn open(input_name: &str, output_name: &str, din: bool) -> Result<Self, String> {
        let mut mi = MidiInput::new("P6 Vault").map_err(|e| e.to_string())?;
        // Explicitly receive SysEx; ignore only active-sensing/timing noise.
        mi.ignore(Ignore::TimeAndActiveSense);
        let in_port = mi
            .ports()
            .into_iter()
            .find(|p| mi.port_name(p).ok().as_deref() == Some(input_name))
            .ok_or_else(|| format!("input port '{input_name}' not found"))?;
        let mo = MidiOutput::new("P6 Vault").map_err(|e| e.to_string())?;
        let out_port = mo
            .ports()
            .into_iter()
            .find(|p| mo.port_name(p).ok().as_deref() == Some(output_name))
            .ok_or_else(|| format!("output port '{output_name}' not found"))?;
        let (tx, rx) = sync_channel::<Vec<u8>>(QUEUE_CAPACITY);
        let overflow = Arc::new(AtomicBool::new(false));
        let ov = overflow.clone();
        let input = mi
            .connect(
                &in_port,
                "p6-vault-in",
                move |_ts, bytes, _| {
                    if let Err(TrySendError::Full(_)) = tx.try_send(bytes.to_vec()) {
                        ov.store(true, Ordering::SeqCst);
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
        let out = mo.connect(&out_port, "p6-vault-out").map_err(|e| e.to_string())?;
        Ok(Self {
            out,
            _input: input,
            rx,
            overflow,
            asm: FrameAssembler::new(MIDI_MAX_FRAME),
            ready: VecDeque::new(),
            kind: if din { TransportKind::Din } else { TransportKind::Usb },
            desc: format!("{input_name} / {output_name}"),
        })
    }

    fn absorb(&mut self, bytes: &[u8]) {
        let mut evs = Vec::new();
        self.asm.push(bytes, &mut evs);
        for e in evs {
            self.ready.push_back(match e {
                FrameEvent::Frame { bytes, .. } => RecvEvent::Frame(bytes),
                FrameEvent::Malformed { reason, .. } => RecvEvent::Malformed(format!("{reason:?}")),
            });
        }
    }
}

impl Transport for MidirTransport {
    fn send(&mut self, bytes: &[u8]) -> Result<(), TransportError> {
        self.out.send(bytes).map_err(|e| TransportError::Io(e.to_string()))
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<RecvEvent>, TransportError> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.overflow.swap(false, Ordering::SeqCst) {
                self.asm.reset();
                self.ready.clear();
                return Err(TransportError::QueueOverflow);
            }
            if let Some(e) = self.ready.pop_front() {
                return Ok(Some(e));
            }
            let now = Instant::now();
            if now >= deadline && !timeout.is_zero() {
                return Ok(None);
            }
            let left = deadline.saturating_duration_since(now);
            match self.rx.recv_timeout(left) {
                Ok(b) => self.absorb(&b),
                Err(RecvTimeoutError::Timeout) => return Ok(None),
                Err(RecvTimeoutError::Disconnected) => return Err(TransportError::Disconnected),
            }
        }
    }

    fn kind(&self) -> TransportKind {
        self.kind
    }

    fn description(&self) -> String {
        self.desc.clone()
    }
}

/// Is the named endpoint still present? (CoreMIDI removes ports on unplug.)
pub fn ports_present(input: &str, output: &str) -> bool {
    list_ports().map(|p| p.inputs.iter().any(|n| n == input) && p.outputs.iter().any(|n| n == output)).unwrap_or(false)
}
