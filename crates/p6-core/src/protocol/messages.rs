//! Exact frame builders and validated message parsing.

use super::packing::{pack, unpack};
use super::payload::Payload;
use super::*;
use crate::slot::{StoredAddress, UserSlot};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum P6Message {
    ProgramData {
        address: StoredAddress,
        payload: Payload,
        canonical: bool,
    },
    EditBufferData {
        payload: Payload,
        canonical: bool,
    },
    ProgramRequest {
        address: StoredAddress,
    },
    EditBufferRequest,
    IdentityReply {
        manufacturer: u8,
        family: Vec<u8>,
        version: Vec<u8>,
    },
    /// A frame addressed to the P6 with a command we do not handle (global, firmware, ...).
    OtherP6 {
        command: u8,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
pub enum MessageError {
    #[error("frame is not a SysEx frame")]
    NotSysEx,
    #[error("not a Prophet-6 message")]
    Unrelated,
    #[error("wrong length for command {command:#04x}: {len} bytes (expected {expected})")]
    BadLength {
        command: u8,
        len: usize,
        expected: usize,
    },
    #[error("invalid program address bank {bank} program {program}")]
    BadAddress { bank: u8, program: u8 },
    #[error("packed payload is corrupt: {0}")]
    BadPacking(String),
}

pub fn request_program(addr: StoredAddress) -> Vec<u8> {
    vec![
        0xF0,
        SEQUENTIAL_ID,
        P6_MODEL_ID,
        CMD_REQUEST_PROGRAM,
        addr.bank,
        addr.program,
        0xF7,
    ]
}

pub fn request_edit_buffer() -> Vec<u8> {
    vec![
        0xF0,
        SEQUENTIAL_ID,
        P6_MODEL_ID,
        CMD_REQUEST_EDIT_BUFFER,
        0xF7,
    ]
}

pub fn identity_inquiry() -> Vec<u8> {
    vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]
}

/// Edit-buffer load (command 03). Used by audition; never writes stored memory.
pub fn edit_buffer_frame(payload: &Payload) -> Vec<u8> {
    let mut v = Vec::with_capacity(EDIT_BUFFER_FRAME_LEN);
    v.extend([0xF0, SEQUENTIAL_ID, P6_MODEL_ID, CMD_EDIT_BUFFER_DATA]);
    v.extend(pack(payload.bytes()));
    v.push(0xF7);
    debug_assert_eq!(v.len(), EDIT_BUFFER_FRAME_LEN);
    v
}

/// Addressed stored-program frame for FILE export (any 000-999 address is representable
/// in a file). Transmission to hardware is only possible through the guarded write engine.
pub fn program_file_frame(addr: StoredAddress, payload: &Payload) -> Vec<u8> {
    let mut v = Vec::with_capacity(PROGRAM_FRAME_LEN);
    v.extend([
        0xF0,
        SEQUENTIAL_ID,
        P6_MODEL_ID,
        CMD_PROGRAM_DATA,
        addr.bank,
        addr.program,
    ]);
    v.extend(pack(payload.bytes()));
    v.push(0xF7);
    debug_assert_eq!(v.len(), PROGRAM_FRAME_LEN);
    v
}

/// Stored-program write frame. Crate-private: only `deployment::writer` may transmit it.
pub(crate) fn stored_write_frame(slot: UserSlot, payload: &Payload) -> Vec<u8> {
    program_file_frame(slot.address(), payload)
}

/// True if this outgoing frame would store a program (command 02 to a P6).
pub fn is_stored_write(frame: &[u8]) -> bool {
    frame.len() >= 4
        && frame[0] == 0xF0
        && frame[1] == SEQUENTIAL_ID
        && frame[2] == P6_MODEL_ID
        && frame[3] == CMD_PROGRAM_DATA
}

fn decode_payload(packed: &[u8]) -> Result<(Payload, bool), MessageError> {
    let u = unpack(packed).map_err(|e| MessageError::BadPacking(e.to_string()))?;
    let p = Payload::from_slice(&u.raw)
        .ok_or_else(|| MessageError::BadPacking(format!("unpacked {} bytes", u.raw.len())))?;
    Ok((p, u.canonical))
}

/// Validate and parse one complete frame (F0 ... F7).
pub fn parse_message(f: &[u8]) -> Result<P6Message, MessageError> {
    if f.len() < 3 || f[0] != 0xF0 || *f.last().unwrap() != 0xF7 {
        return Err(MessageError::NotSysEx);
    }
    // Universal non-realtime identity reply: F0 7E <ch> 06 02 <mfr> <family..> <version..> F7
    if f.len() >= 8 && f[1] == 0x7E && f[3] == 0x06 && f[4] == 0x02 {
        let manufacturer = f[5];
        if manufacturer != SEQUENTIAL_ID {
            return Err(MessageError::Unrelated);
        }
        let body = &f[6..f.len() - 1];
        let family: Vec<u8> = body.iter().take(4).copied().collect();
        if family.first() != Some(&P6_MODEL_ID) {
            return Err(MessageError::Unrelated);
        }
        let version = body.iter().skip(4).copied().collect();
        return Ok(P6Message::IdentityReply {
            manufacturer,
            family,
            version,
        });
    }
    if f.len() < 5 || f[1] != SEQUENTIAL_ID || f[2] != P6_MODEL_ID {
        return Err(MessageError::Unrelated);
    }
    let cmd = f[3];
    let need = |expected: usize| {
        if f.len() == expected {
            Ok(())
        } else {
            Err(MessageError::BadLength {
                command: cmd,
                len: f.len(),
                expected,
            })
        }
    };
    let addr = |b: u8, p: u8| {
        StoredAddress::new(b, p).ok_or(MessageError::BadAddress {
            bank: b,
            program: p,
        })
    };
    match cmd {
        CMD_PROGRAM_DATA => {
            need(PROGRAM_FRAME_LEN)?;
            let address = addr(f[4], f[5])?;
            let (payload, canonical) = decode_payload(&f[6..f.len() - 1])?;
            Ok(P6Message::ProgramData {
                address,
                payload,
                canonical,
            })
        }
        CMD_EDIT_BUFFER_DATA => {
            need(EDIT_BUFFER_FRAME_LEN)?;
            let (payload, canonical) = decode_payload(&f[4..f.len() - 1])?;
            Ok(P6Message::EditBufferData { payload, canonical })
        }
        CMD_REQUEST_PROGRAM => {
            need(7)?;
            Ok(P6Message::ProgramRequest {
                address: addr(f[4], f[5])?,
            })
        }
        CMD_REQUEST_EDIT_BUFFER => {
            need(5)?;
            Ok(P6Message::EditBufferRequest)
        }
        other => Ok(P6Message::OtherP6 { command: other }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::payload::synthetic_payload;

    #[test]
    fn frame_lengths_and_roundtrip() {
        let p = synthetic_payload(3, "Lead 1");
        let a = StoredAddress::new(4, 99).unwrap();
        let f = program_file_frame(a, &p);
        assert_eq!(f.len(), 1178);
        assert_eq!(&f[..6], &[0xF0, 0x01, 0x2D, 0x02, 4, 99]);
        match parse_message(&f).unwrap() {
            P6Message::ProgramData {
                address,
                payload,
                canonical,
            } => {
                assert_eq!(address, a);
                assert_eq!(payload, p);
                assert!(canonical);
            }
            m => panic!("{m:?}"),
        }
        let e = edit_buffer_frame(&p);
        assert_eq!(e.len(), 1176);
        assert!(matches!(
            parse_message(&e).unwrap(),
            P6Message::EditBufferData { .. }
        ));
        assert!(!is_stored_write(&e));
        assert!(is_stored_write(&f));
    }

    #[test]
    fn requests() {
        assert_eq!(
            request_program(StoredAddress::new(1, 5).unwrap()),
            vec![0xF0, 1, 0x2D, 5, 1, 5, 0xF7]
        );
        assert_eq!(request_edit_buffer(), vec![0xF0, 1, 0x2D, 6, 0xF7]);
        assert_eq!(identity_inquiry(), vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]);
    }

    #[test]
    fn rejects_bad_frames() {
        let p = synthetic_payload(3, "x");
        let mut f = program_file_frame(StoredAddress::new(0, 0).unwrap(), &p);
        f.remove(10);
        assert!(matches!(
            parse_message(&f),
            Err(MessageError::BadLength { .. })
        ));
        let mut f = program_file_frame(StoredAddress::new(0, 0).unwrap(), &p);
        f[4] = 10;
        assert!(matches!(
            parse_message(&f),
            Err(MessageError::BadAddress { .. })
        ));
        assert_eq!(
            parse_message(&[0xF0, 0x42, 0x00, 0xF7]),
            Err(MessageError::Unrelated)
        );
    }

    #[test]
    fn identity_reply_variable_length() {
        let r = [
            0xF0, 0x7E, 0x00, 0x06, 0x02, 0x01, 0x2D, 0x01, 0x00, 0x00, 0x05, 0x01, 0xF7,
        ];
        match parse_message(&r).unwrap() {
            P6Message::IdentityReply {
                family, version, ..
            } => {
                assert_eq!(family, vec![0x2D, 0x01, 0x00, 0x00]);
                assert_eq!(version, vec![0x05, 0x01]);
            }
            m => panic!("{m:?}"),
        }
        let short = [0xF0, 0x7E, 0x00, 0x06, 0x02, 0x01, 0x2D, 0x01, 0xF7];
        assert!(parse_message(&short).is_ok());
    }
}
