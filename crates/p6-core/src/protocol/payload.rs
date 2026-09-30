//! The 1024-byte unpacked program payload, names, fingerprints and the typed
//! parameter decoder.
//!
//! All offsets here are zero-based within the UNPACKED payload, taken from
//! Sequential's "P6 packed parameter data assignments" chart [S2]. NRPN numbers
//! are a different namespace and must never be used as offsets.

use super::PAYLOAD_LEN;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fmt;
use std::sync::Arc;

/// Exact 1024 unpacked program bytes, including sequence and reserved bytes. Immutable.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Payload(Arc<[u8; PAYLOAD_LEN]>);

impl fmt::Debug for Payload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Payload({} {})",
            self.display_name().unwrap_or_default(),
            &self.exact_hash()[..12]
        )
    }
}

pub const NAME_RANGE: std::ops::Range<usize> = 107..127;
pub const FORMAT_VERSION_OFFSET: usize = 105;
pub const EDITOR_BYTE_OFFSET: usize = 106;

impl Payload {
    pub fn from_slice(b: &[u8]) -> Option<Self> {
        let arr: [u8; PAYLOAD_LEN] = b.try_into().ok()?;
        Some(Self(Arc::new(arr)))
    }
    pub fn bytes(&self) -> &[u8; PAYLOAD_LEN] {
        &self.0
    }
    pub fn get(&self, off: usize) -> u8 {
        self.0[off]
    }
    pub fn exact_hash(&self) -> String {
        hex::encode(Sha256::digest(&self.0[..]))
    }
    /// SHA-256 with the name bytes 107..127 zeroed. "Identical except name"; not an acoustic similarity.
    pub fn name_independent_hash(&self) -> String {
        let mut b = *self.0;
        b[NAME_RANGE].fill(0);
        hex::encode(Sha256::digest(b))
    }
    pub fn raw_name(&self) -> &[u8] {
        &self.0[NAME_RANGE]
    }
    pub fn format_version(&self) -> u8 {
        self.0[FORMAT_VERSION_OFFSET]
    }
    /// Name for display: trailing padding trimmed, control chars sanitized. None if unusable.
    pub fn display_name(&self) -> Option<String> {
        let raw = self.raw_name();
        if raw.iter().any(|&b| b >= 0x80) {
            return None;
        }
        let s: String = raw
            .iter()
            .map(|&b| {
                if (0x20..0x7F).contains(&b) {
                    b as char
                } else {
                    ' '
                }
            })
            .collect();
        let t = s.trim_end().to_string();
        if t.trim().is_empty() {
            None
        } else {
            Some(t)
        }
    }
    /// Whether the documented chart layout is usable for this payload. We only
    /// decline when the name field contains bytes that cannot be chart ASCII
    /// (bit 7 set), which indicates a different/unsupported layout.
    pub fn layout(&self) -> Layout {
        if self.raw_name().iter().any(|&b| b >= 0x80) {
            Layout::Unsupported
        } else {
            Layout::Chart2016
        }
    }
    pub fn decode(&self) -> Option<DecodedParams> {
        (self.layout() == Layout::Chart2016).then(|| DecodedParams::from_payload(self))
    }
    /// Number of differing bytes and the first few differing offsets.
    pub fn diff(&self, other: &Payload) -> PayloadDiff {
        let offs: Vec<usize> = (0..PAYLOAD_LEN)
            .filter(|&i| self.0[i] != other.0[i])
            .collect();
        PayloadDiff {
            count: offs.len(),
            first_offsets: offs.into_iter().take(16).collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PayloadDiff {
    pub count: usize,
    pub first_offsets: Vec<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Layout {
    /// Sequential payload assignment chart, 24 May 2016 [S2]. Hardware validation pending.
    Chart2016,
    Unsupported,
}

pub const DECODER_VERSION: u32 = 1;

/// Decoder field map (unpacked offsets). Raw values are reported unchanged; the
/// `max` column is the documented NRPN range used only as normalization evidence.
pub struct Field {
    pub name: &'static str,
    pub offset: usize,
    pub max: u8,
}

macro_rules! fields {
    ($($id:ident = $off:expr, $max:expr;)*) => {
        pub mod off { $(pub const $id: usize = $off;)* }
        pub const FIELDS: &[Field] = &[$(Field { name: stringify!($id), offset: $off, max: $max },)*];
    };
}

fields! {
    OSC1_PITCH = 0, 120; OSC2_PITCH = 1, 120; OSC2_FINE = 2, 254;
    OSC1_SHAPE = 3, 254; OSC2_SHAPE = 4, 254; OSC1_PW = 5, 254; OSC2_PW = 6, 254;
    OSC1_LEVEL = 7, 127; OSC2_LEVEL = 8, 127; SUB_LEVEL = 9, 127; NOISE_LEVEL = 10, 127;
    SYNC = 11, 1; OSC2_KEYBOARD = 12, 1; OSC2_LOW_FREQ = 13, 1;
    GLIDE_RATE = 14, 127; GLIDE_MODE = 15, 1; GLIDE_ON = 16, 1; PITCH_BEND = 17, 12; SLOP = 18, 127;
    LP_CUTOFF = 19, 164; LP_RESONANCE = 20, 255; HP_CUTOFF = 23, 164; HP_RESONANCE = 24, 255;
    VOICE_VOLUME = 27, 127; PAN = 28, 127;
    LP_ENV_AMT = 29, 254; HP_ENV_AMT = 30, 254; VCA_ENV_AMT = 31, 127;
    FILT_ATTACK = 35, 127; FILT_DECAY = 37, 127; FILT_SUSTAIN = 39, 127; FILT_RELEASE = 41, 127;
    AMP_ATTACK = 36, 127; AMP_DECAY = 38, 127; AMP_SUSTAIN = 40, 127; AMP_RELEASE = 42, 127;
    FX1_TYPE = 44, 12; FX2_TYPE = 45, 12; FX1_ON = 46, 1; FX2_ON = 47, 1; FX1_MIX = 48, 127; FX2_MIX = 49, 127;
    DISTORTION = 58, 127; LFO_FREQ = 59, 254; LFO_SHAPE = 62, 4; LFO_AMOUNT = 63, 254;
    POLYMOD_ENV_AMT = 77, 254; POLYMOD_OSC2_AMT = 78, 254;
    UNISON_ON = 84, 1; UNISON_MODE = 85, 16; KEY_MODE = 86, 5; TEMPO = 87, 255;
    ARP_MODE = 89, 4; ARP_RANGE = 90, 2; ARP_ON = 91, 1; ARP_DIVISION = 92, 12;
    SEQ_ON = 93, 1; SEQ_RECORD = 94, 1; SEQ_MODE = 95, 2; SEQ_PLAY_MODE = 96, 3;
    FORMAT_VERSION = 105, 255; EDITOR_BYTE = 106, 255;
}

/// Decoded view of the fields used by classification. Values are raw payload bytes.
#[derive(Debug, Clone, Serialize)]
pub struct DecodedParams {
    pub values: Vec<(&'static str, u8)>,
}

impl DecodedParams {
    fn from_payload(p: &Payload) -> Self {
        Self {
            values: FIELDS.iter().map(|f| (f.name, p.get(f.offset))).collect(),
        }
    }
    pub fn raw(&self, name: &str) -> u8 {
        self.values
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, v)| *v)
            .unwrap_or(0)
    }
    /// Value normalized to 0..=1 using the documented range, clamped only in the computed view.
    pub fn norm(&self, name: &str) -> f64 {
        let max = FIELDS
            .iter()
            .find(|f| f.name == name)
            .map(|f| f.max)
            .unwrap_or(127)
            .max(1);
        (self.raw(name) as f64 / max as f64).min(1.0)
    }
    pub fn flag(&self, name: &str) -> bool {
        self.raw(name) != 0
    }
}

/// Deterministic synthetic payload for tests/simulator. Clearly marked test data.
pub fn synthetic_payload(seed: u32, name: &str) -> Payload {
    let mut b = [0u8; PAYLOAD_LEN];
    let mut x = seed.wrapping_mul(2654435761).wrapping_add(12345);
    for (i, v) in b.iter_mut().enumerate() {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        *v = if i < 105 {
            (x % 128) as u8
        } else {
            (x & 0xFF) as u8
        };
    }
    // Flags are 0/1 in the chart; keep synthetic data plausible.
    for o in [
        off::SYNC,
        off::OSC2_KEYBOARD,
        off::OSC2_LOW_FREQ,
        off::GLIDE_ON,
        off::GLIDE_MODE,
        off::FX1_ON,
        off::FX2_ON,
        off::UNISON_ON,
        off::ARP_ON,
        off::SEQ_ON,
        off::SEQ_RECORD,
    ] {
        b[o] &= 1;
    }
    b[off::ARP_ON] = 0;
    b[off::SEQ_ON] = 0;
    b[off::SEQ_RECORD] = 0;
    b[FORMAT_VERSION_OFFSET] = 1;
    set_name(&mut b, name);
    Payload::from_slice(&b).unwrap()
}

pub fn set_name(b: &mut [u8; PAYLOAD_LEN], name: &str) {
    let mut field = [b' '; 20];
    for (d, s) in field
        .iter_mut()
        .zip(name.bytes().filter(|c| (0x20..0x7F).contains(c)))
    {
        *d = s;
    }
    b[NAME_RANGE].copy_from_slice(&field);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_at_107() {
        let p = synthetic_payload(1, "Fat Bass");
        assert_eq!(p.display_name().as_deref(), Some("Fat Bass"));
        assert_eq!(&p.bytes()[107..115], b"Fat Bass");
    }

    #[test]
    fn fingerprints() {
        let a = synthetic_payload(7, "Alpha");
        let mut b = *a.bytes();
        set_name(&mut b, "Beta");
        let b = Payload::from_slice(&b).unwrap();
        assert_ne!(a.exact_hash(), b.exact_hash());
        assert_eq!(a.name_independent_hash(), b.name_independent_hash());
        // A sequence-byte change (offset 236 lies in the sequence area 128..896) changes both.
        let mut c = *a.bytes();
        c[236] ^= 1;
        let c = Payload::from_slice(&c).unwrap();
        assert_ne!(a.exact_hash(), c.exact_hash());
        assert_ne!(a.name_independent_hash(), c.name_independent_hash());
    }

    #[test]
    fn no_nrpn_as_offset_regression() {
        // NRPN-era mistakes put names at 236-255 and arp/seq at other offsets.
        assert_eq!(NAME_RANGE, 107..127);
        assert_eq!(off::ARP_ON, 91);
        assert_eq!(off::SEQ_ON, 93);
        let mut b = [0u8; PAYLOAD_LEN];
        b[91] = 1;
        let p = Payload::from_slice(&b).unwrap();
        assert!(p.decode().unwrap().flag("ARP_ON"));
        assert!(!p.decode().unwrap().flag("SEQ_ON"));
    }

    #[test]
    fn unsupported_layout_retains_bytes() {
        let mut b = [0u8; PAYLOAD_LEN];
        b[110] = 0xC1;
        let p = Payload::from_slice(&b).unwrap();
        assert_eq!(p.layout(), Layout::Unsupported);
        assert!(p.decode().is_none());
        assert!(p.display_name().is_none());
        assert_eq!(p.bytes()[110], 0xC1);
    }

    #[test]
    fn sanitizes_control_chars() {
        let mut b = [0u8; PAYLOAD_LEN];
        b[107..112].copy_from_slice(b"Ab\x01cd");
        let p = Payload::from_slice(&b).unwrap();
        assert_eq!(p.display_name().as_deref(), Some("Ab cd"));
    }
}
