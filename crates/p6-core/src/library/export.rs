//! .syx export. Pure byte construction + atomic file writing with re-parse verification.
//! Exports never transmit MIDI.

use crate::protocol::framing::{split_all, FrameEvent, FILE_MAX_FRAME};
use crate::protocol::messages::{edit_buffer_frame, parse_message, program_file_frame, P6Message};
use crate::protocol::payload::Payload;
use crate::slot::{StoredAddress, UserSlot};
use std::io::Write;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    #[error("the bank has {0} empty slot(s); a complete bank export needs all 500")]
    IncompleteBank(usize),
    #[error("destination {0} is used more than once")]
    DuplicateDestination(UserSlot),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("export verification failed: {0}")]
    Verification(String),
}

/// 500 addressed stored-program messages in ascending slot order: 589000 bytes.
pub fn bank_bytes(bank: &[Option<Payload>]) -> Result<Vec<u8>, ExportError> {
    let empty = bank.iter().filter(|p| p.is_none()).count() + 500usize.saturating_sub(bank.len());
    if empty > 0 || bank.len() != 500 {
        return Err(ExportError::IncompleteBank(empty));
    }
    let mut out = Vec::with_capacity(500 * 1178);
    for (i, p) in bank.iter().enumerate() {
        out.extend(program_file_frame(
            UserSlot::new(i as u16).unwrap().address(),
            p.as_ref().unwrap(),
        ));
    }
    Ok(out)
}

pub fn selection_bytes(items: &[(UserSlot, Payload)]) -> Result<Vec<u8>, ExportError> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (s, p) in items {
        if !seen.insert(*s) {
            return Err(ExportError::DuplicateDestination(*s));
        }
        out.extend(program_file_frame(s.address(), p));
    }
    Ok(out)
}

pub fn edit_buffer_bytes(p: &Payload) -> Vec<u8> {
    edit_buffer_frame(p)
}

/// Expected content, used to verify a file after writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expected {
    Programs(Vec<(StoredAddress, Payload)>),
    EditBuffer(Payload),
    Raw,
}

pub fn verify_bytes(bytes: &[u8], expected: &Expected) -> Result<(), ExportError> {
    let parsed: Vec<P6Message> = split_all(bytes, FILE_MAX_FRAME)
        .into_iter()
        .map(|e| match e {
            FrameEvent::Frame { bytes, .. } => {
                parse_message(&bytes).map_err(|e| ExportError::Verification(e.to_string()))
            }
            FrameEvent::Malformed { reason, .. } => {
                Err(ExportError::Verification(format!("{reason:?}")))
            }
        })
        .collect::<Result<_, _>>()?;
    match expected {
        Expected::Raw => Ok(()),
        Expected::EditBuffer(p) => match parsed.as_slice() {
            [P6Message::EditBufferData { payload, .. }] if payload == p => Ok(()),
            _ => Err(ExportError::Verification(
                "edit buffer did not re-parse identically".into(),
            )),
        },
        Expected::Programs(list) => {
            if parsed.len() != list.len() {
                return Err(ExportError::Verification(format!(
                    "{} messages, expected {}",
                    parsed.len(),
                    list.len()
                )));
            }
            for (m, (a, p)) in parsed.iter().zip(list) {
                match m {
                    P6Message::ProgramData {
                        address, payload, ..
                    } if address == a && payload == p => {}
                    _ => {
                        return Err(ExportError::Verification(format!(
                            "slot {a} did not re-parse identically"
                        )))
                    }
                }
            }
            Ok(())
        }
    }
}

/// Write to a temp file, fsync, atomically rename, then re-read and verify.
pub fn write_verified(path: &Path, bytes: &[u8], expected: &Expected) -> Result<(), ExportError> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let tmp = dir.join(format!(
        ".{}.p6vault-tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("export")
    ));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
    let back = std::fs::read(path)?;
    if back != bytes {
        return Err(ExportError::Verification(
            "file contents differ after write".into(),
        ));
    }
    verify_bytes(&back, expected)
}

pub fn bank_expected(bank: &[Option<Payload>]) -> Expected {
    Expected::Programs(
        bank.iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    UserSlot::new(i as u16).unwrap().address(),
                    p.clone().unwrap(),
                )
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::payload::synthetic_payload;

    #[test]
    fn full_bank_589000() {
        let bank: Vec<Option<Payload>> = (0..500)
            .map(|i| Some(synthetic_payload(i, &format!("P{i}"))))
            .collect();
        let bytes = bank_bytes(&bank).unwrap();
        assert_eq!(bytes.len(), 589000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bank.syx");
        write_verified(&path, &bytes, &bank_expected(&bank)).unwrap();
        let prev =
            crate::library::import::preview_import("bank.syx", &std::fs::read(&path).unwrap())
                .unwrap();
        assert_eq!(
            prev.complete_user_bank().unwrap(),
            bank.into_iter().map(Option::unwrap).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_empty_slots() {
        let mut bank: Vec<Option<Payload>> =
            (0..500).map(|i| Some(synthetic_payload(i, "x"))).collect();
        bank[7] = None;
        assert!(matches!(
            bank_bytes(&bank),
            Err(ExportError::IncompleteBank(1))
        ));
    }
}
