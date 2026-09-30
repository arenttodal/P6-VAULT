//! .syx import preview. Pure: parses bytes, sends no MIDI, writes nothing.

use crate::protocol::framing::{split_all, FrameEvent, MalformedReason, FILE_MAX_FRAME};
use crate::protocol::messages::{parse_message, MessageError, P6Message};
use crate::protocol::payload::Payload;
use crate::slot::StoredAddress;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub const MAX_IMPORT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum OccurrenceKind {
    Program,
    EditBuffer,
}

#[derive(Debug, Clone)]
pub struct ParsedOccurrence {
    pub message_index: usize,
    pub byte_offset: u64,
    pub kind: OccurrenceKind,
    pub address: Option<StoredAddress>,
    pub payload: Payload,
    pub frame: Vec<u8>,
    /// Frame was packed noncanonically; original frame retained, quarantined from sending.
    pub noncanonical: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Excluded {
    pub message_index: usize,
    pub byte_offset: u64,
    pub reason: String,
    pub kind: ExcludedKind,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ExcludedKind {
    Unrelated,
    Unsupported,
    Malformed,
    Truncated,
}

#[derive(Debug, Clone)]
pub struct ImportPreview {
    pub file_name: String,
    pub file_hash: String,
    pub file_len: usize,
    pub occurrences: Vec<ParsedOccurrence>,
    pub excluded: Vec<Excluded>,
    pub repeated_in_file: usize,
    pub unique_payloads: usize,
    pub repeated_addresses: Vec<StoredAddress>,
}

#[derive(Debug, Clone, thiserror::Error, Serialize)]
pub enum ImportError {
    #[error("file is {0} bytes; the limit is 32 MiB")]
    TooLarge(usize),
    #[error("no Prophet-6 program or edit-buffer data found")]
    NoPrograms,
}

pub fn file_hash(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn preview_import(file_name: &str, bytes: &[u8]) -> Result<ImportPreview, ImportError> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(ImportError::TooLarge(bytes.len()));
    }
    let mut occurrences = Vec::new();
    let mut excluded = Vec::new();
    for (idx, ev) in split_all(bytes, FILE_MAX_FRAME).into_iter().enumerate() {
        match ev {
            FrameEvent::Frame {
                bytes: frame,
                start_offset,
            } => match parse_message(&frame) {
                Ok(P6Message::ProgramData {
                    address,
                    payload,
                    canonical,
                }) => occurrences.push(ParsedOccurrence {
                    message_index: idx,
                    byte_offset: start_offset,
                    kind: OccurrenceKind::Program,
                    address: Some(address),
                    payload,
                    frame,
                    noncanonical: !canonical,
                }),
                Ok(P6Message::EditBufferData { payload, canonical }) => {
                    occurrences.push(ParsedOccurrence {
                        message_index: idx,
                        byte_offset: start_offset,
                        kind: OccurrenceKind::EditBuffer,
                        address: None,
                        payload,
                        frame,
                        noncanonical: !canonical,
                    })
                }
                Ok(other) => excluded.push(Excluded {
                    message_index: idx,
                    byte_offset: start_offset,
                    reason: format!("Prophet-6 message not imported ({})", describe(&other)),
                    kind: ExcludedKind::Unsupported,
                }),
                Err(MessageError::Unrelated) => excluded.push(Excluded {
                    message_index: idx,
                    byte_offset: start_offset,
                    reason: "not a Prophet-6 message".into(),
                    kind: ExcludedKind::Unrelated,
                }),
                Err(e) => excluded.push(Excluded {
                    message_index: idx,
                    byte_offset: start_offset,
                    reason: e.to_string(),
                    kind: ExcludedKind::Malformed,
                }),
            },
            FrameEvent::Malformed {
                reason,
                start_offset,
                ..
            } => excluded.push(Excluded {
                message_index: idx,
                byte_offset: start_offset,
                reason: format!("{reason:?}"),
                kind: if reason == MalformedReason::Truncated {
                    ExcludedKind::Truncated
                } else {
                    ExcludedKind::Malformed
                },
            }),
        }
    }
    if occurrences.is_empty() {
        return Err(ImportError::NoPrograms);
    }
    let mut seen: HashMap<String, usize> = HashMap::new();
    for o in &occurrences {
        *seen.entry(o.payload.exact_hash()).or_default() += 1;
    }
    let unique_payloads = seen.len();
    let repeated_in_file = occurrences.len() - unique_payloads;
    let mut addr_count: HashMap<StoredAddress, usize> = HashMap::new();
    for a in occurrences.iter().filter_map(|o| o.address) {
        *addr_count.entry(a).or_default() += 1;
    }
    let mut repeated_addresses: Vec<_> = addr_count
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(a, _)| a)
        .collect();
    repeated_addresses.sort();
    Ok(ImportPreview {
        file_name: file_name.to_string(),
        file_hash: file_hash(bytes),
        file_len: bytes.len(),
        occurrences,
        excluded,
        repeated_in_file,
        unique_payloads,
        repeated_addresses,
    })
}

fn describe(m: &P6Message) -> String {
    match m {
        P6Message::ProgramRequest { .. } => "program request".into(),
        P6Message::EditBufferRequest => "edit-buffer request".into(),
        P6Message::IdentityReply { .. } => "identity reply".into(),
        P6Message::OtherP6 { command } => format!("command {command:#04x}"),
        _ => "program data".into(),
    }
}

impl ImportPreview {
    /// Is this an unambiguous, complete 000-499 user bank?
    pub fn complete_user_bank(&self) -> Option<Vec<Payload>> {
        if !self.repeated_addresses.is_empty() {
            return None;
        }
        let mut bank: Vec<Option<Payload>> = vec![None; 500];
        for o in &self.occurrences {
            if let Some(s) = o.address.and_then(|a| a.user_slot()) {
                bank[s.index()] = Some(o.payload.clone());
            }
        }
        bank.into_iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::messages::{edit_buffer_frame, program_file_frame};
    use crate::protocol::payload::synthetic_payload;

    #[test]
    fn mixed_file() {
        let mut data = Vec::new();
        let a = synthetic_payload(1, "One");
        data.extend(program_file_frame(StoredAddress::new(0, 1).unwrap(), &a));
        data.extend([0xF0, 0x42, 0x00, 0xF7]); // unrelated
        data.extend(program_file_frame(StoredAddress::new(0, 1).unwrap(), &a)); // repeated address + payload
        data.extend(edit_buffer_frame(&synthetic_payload(2, "Buf")));
        data.extend([0xF0, 0x01, 0x2D, 0x02, 0x00]); // truncated
        let p = preview_import("x.syx", &data).unwrap();
        assert_eq!(p.occurrences.len(), 3);
        assert_eq!(p.unique_payloads, 2);
        assert_eq!(p.repeated_in_file, 1);
        assert_eq!(p.repeated_addresses.len(), 1);
        assert_eq!(p.occurrences[2].address, None);
        assert_eq!(p.occurrences[2].kind, OccurrenceKind::EditBuffer);
        assert_eq!(p.excluded.len(), 2);
        assert!(p.complete_user_bank().is_none());
    }

    #[test]
    fn empty_file() {
        assert!(matches!(
            preview_import("x", &[1, 2, 3]),
            Err(ImportError::NoPrograms)
        ));
    }
}
