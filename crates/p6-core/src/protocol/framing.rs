//! Streaming SysEx frame assembler shared by file import and MIDI reception.
//!
//! Handles concatenated frames, arbitrary fragmentation, interleaved real-time
//! status bytes (0xF8-0xFF, which are never payload bytes), nested `F0`
//! (abandons the prior frame), unexpected status bytes inside a frame (abandon)
//! and bounded frame storage.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum MalformedReason {
    /// A new F0 arrived before the previous frame's F7.
    NestedStart,
    /// A non-real-time status byte appeared inside a SysEx frame.
    UnexpectedStatus(u8),
    /// The frame exceeded the configured maximum length.
    Oversize,
    /// Input ended (or was reset) inside a frame.
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameEvent {
    /// A complete frame including leading F0 and trailing F7.
    Frame { bytes: Vec<u8>, start_offset: u64 },
    /// An invalid/abandoned frame. `bytes` holds what was retained (bounded).
    Malformed { reason: MalformedReason, bytes: Vec<u8>, start_offset: u64 },
}

pub fn is_realtime(b: u8) -> bool {
    b >= 0xF8
}

/// Stateful assembler. Feed bytes in any fragmentation; collect events.
#[derive(Debug)]
pub struct FrameAssembler {
    max_frame: usize,
    buf: Vec<u8>,
    in_frame: bool,
    oversize: bool,
    frame_start: u64,
    offset: u64,
}

/// Default frame bound for MIDI reception (largest valid P6 frame is 1178 bytes).
pub const MIDI_MAX_FRAME: usize = 4096;
/// Frame bound for file import. Unknown frames up to this are retained for reporting.
pub const FILE_MAX_FRAME: usize = 1 << 20;

impl FrameAssembler {
    pub fn new(max_frame: usize) -> Self {
        Self { max_frame, buf: Vec::new(), in_frame: false, oversize: false, frame_start: 0, offset: 0 }
    }

    pub fn in_frame(&self) -> bool {
        self.in_frame
    }

    /// Discard any partial frame (timeout/disconnect). Returns a Truncated event if one was pending.
    pub fn reset(&mut self) -> Option<FrameEvent> {
        let ev = if self.in_frame {
            Some(FrameEvent::Malformed { reason: MalformedReason::Truncated, bytes: std::mem::take(&mut self.buf), start_offset: self.frame_start })
        } else {
            None
        };
        self.buf.clear();
        self.in_frame = false;
        self.oversize = false;
        ev
    }

    /// End of input: report a truncated frame if one is open.
    pub fn finish(&mut self) -> Option<FrameEvent> {
        self.reset()
    }

    pub fn push(&mut self, data: &[u8], out: &mut Vec<FrameEvent>) {
        for &b in data {
            let off = self.offset;
            self.offset += 1;
            if is_realtime(b) {
                continue;
            }
            match b {
                0xF0 => {
                    if self.in_frame {
                        out.push(FrameEvent::Malformed {
                            reason: MalformedReason::NestedStart,
                            bytes: std::mem::take(&mut self.buf),
                            start_offset: self.frame_start,
                        });
                    }
                    self.buf.clear();
                    self.buf.push(0xF0);
                    self.in_frame = true;
                    self.oversize = false;
                    self.frame_start = off;
                }
                0xF7 => {
                    if self.in_frame {
                        self.in_frame = false;
                        if self.oversize {
                            out.push(FrameEvent::Malformed {
                                reason: MalformedReason::Oversize,
                                bytes: std::mem::take(&mut self.buf),
                                start_offset: self.frame_start,
                            });
                        } else {
                            self.buf.push(0xF7);
                            out.push(FrameEvent::Frame { bytes: std::mem::take(&mut self.buf), start_offset: self.frame_start });
                        }
                    }
                    // Stray F7 outside a frame is irrelevant data.
                }
                0x80..=0xEF | 0xF1..=0xF6 => {
                    if self.in_frame {
                        self.in_frame = false;
                        out.push(FrameEvent::Malformed {
                            reason: MalformedReason::UnexpectedStatus(b),
                            bytes: std::mem::take(&mut self.buf),
                            start_offset: self.frame_start,
                        });
                    }
                }
                _ => {
                    if self.in_frame {
                        if self.buf.len() + 1 >= self.max_frame {
                            self.oversize = true;
                        } else {
                            self.buf.push(b);
                        }
                    }
                }
            }
        }
    }
}

/// Split a complete byte buffer (e.g. a file) into events.
pub fn split_all(data: &[u8], max_frame: usize) -> Vec<FrameEvent> {
    let mut a = FrameAssembler::new(max_frame);
    let mut out = Vec::new();
    a.push(data, &mut out);
    if let Some(e) = a.finish() {
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frames(ev: &[FrameEvent]) -> Vec<Vec<u8>> {
        ev.iter()
            .filter_map(|e| match e {
                FrameEvent::Frame { bytes, .. } => Some(bytes.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn concatenated_and_realtime() {
        let data = [0xF0, 0x01, 0xF8, 0x02, 0xF7, 0x00, 0xF0, 0x03, 0xFE, 0xF7];
        let ev = split_all(&data, 64);
        assert_eq!(frames(&ev), vec![vec![0xF0, 0x01, 0x02, 0xF7], vec![0xF0, 0x03, 0xF7]]);
    }

    #[test]
    fn every_split_boundary() {
        let data: Vec<u8> = [vec![0xF0, 0x01, 0x2D, 0x06, 0xF7], vec![0xF0, 0x01, 0x02, 0x03, 0xF7]].concat();
        for cut in 0..=data.len() {
            let mut a = FrameAssembler::new(64);
            let mut out = Vec::new();
            a.push(&data[..cut], &mut out);
            a.push(&data[cut..], &mut out);
            assert_eq!(frames(&out).len(), 2, "cut {cut}");
            assert!(a.finish().is_none());
        }
    }

    #[test]
    fn nested_start_abandons_prior() {
        let ev = split_all(&[0xF0, 0x01, 0xF0, 0x02, 0xF7], 64);
        assert!(matches!(ev[0], FrameEvent::Malformed { reason: MalformedReason::NestedStart, .. }));
        assert_eq!(frames(&ev), vec![vec![0xF0, 0x02, 0xF7]]);
    }

    #[test]
    fn truncated_and_status() {
        let ev = split_all(&[0xF0, 0x01, 0x02], 64);
        assert!(matches!(ev[0], FrameEvent::Malformed { reason: MalformedReason::Truncated, .. }));
        let ev = split_all(&[0xF0, 0x01, 0x90, 0x40, 0x40, 0xF7], 64);
        assert!(matches!(ev[0], FrameEvent::Malformed { reason: MalformedReason::UnexpectedStatus(0x90), .. }));
        assert_eq!(ev.len(), 1);
    }

    #[test]
    fn bounded_storage() {
        let mut data = vec![0xF0];
        data.extend(std::iter::repeat_n(0x11, 100));
        data.push(0xF7);
        let ev = split_all(&data, 16);
        assert!(matches!(&ev[0], FrameEvent::Malformed { reason: MalformedReason::Oversize, bytes, .. } if bytes.len() < 16));
    }

    #[test]
    fn offsets_recorded() {
        let ev = split_all(&[0x00, 0x00, 0xF0, 0x01, 0xF7], 16);
        assert!(matches!(ev[0], FrameEvent::Frame { start_offset: 2, .. }));
    }
}
