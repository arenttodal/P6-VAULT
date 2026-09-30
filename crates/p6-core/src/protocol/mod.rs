//! Prophet-6 SysEx protocol (Operation Manual v2.1, Appendix C).

pub mod framing;
pub mod messages;
pub mod packing;
pub mod payload;

pub const SEQUENTIAL_ID: u8 = 0x01;
pub const P6_MODEL_ID: u8 = 0x2D;

pub const CMD_PROGRAM_DATA: u8 = 0x02;
pub const CMD_EDIT_BUFFER_DATA: u8 = 0x03;
pub const CMD_REQUEST_PROGRAM: u8 = 0x05;
pub const CMD_REQUEST_EDIT_BUFFER: u8 = 0x06;

/// Unpacked program payload length.
pub const PAYLOAD_LEN: usize = 1024;
/// Packed payload length on the wire.
pub const PACKED_LEN: usize = 1171;
/// Full stored-program frame: F0 01 2D 02 bank prog <1171> F7.
pub const PROGRAM_FRAME_LEN: usize = 1178;
/// Full edit-buffer frame: F0 01 2D 03 <1171> F7.
pub const EDIT_BUFFER_FRAME_LEN: usize = 1176;

pub const USER_SLOTS: usize = 500;
