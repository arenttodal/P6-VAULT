//! Prophet-6 7-in-8 MSB packing.
//!
//! Each group of up to seven raw bytes is emitted as one prefix byte whose bit `i`
//! holds bit 7 of raw byte `i`, followed by the raw bytes masked with `0x7F`.
//! 1024 raw bytes = 146 full groups + one 2-byte group = 146 * 8 + 3 = 1171 packed bytes.

use super::{PACKED_LEN, PAYLOAD_LEN};

/// Pack raw bytes into the canonical 7-bit stream. Unused prefix bits are zero.
pub fn pack(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(packed_len_for(raw.len()));
    for chunk in raw.chunks(7) {
        let mut prefix = 0u8;
        for (i, b) in chunk.iter().enumerate() {
            prefix |= (b >> 7) << i;
        }
        out.push(prefix);
        out.extend(chunk.iter().map(|b| b & 0x7F));
    }
    out
}

/// Number of packed bytes needed for `raw_len` raw bytes.
pub const fn packed_len_for(raw_len: usize) -> usize {
    let full = raw_len / 7;
    let rem = raw_len % 7;
    full * 8 + if rem == 0 { 0 } else { rem + 1 }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UnpackError {
    #[error("packed data byte {value:#04x} at offset {offset} has bit 7 set")]
    HighBit { offset: usize, value: u8 },
    #[error("packed group at offset {offset} is empty (prefix without data)")]
    EmptyGroup { offset: usize },
}

/// Result of unpacking: raw bytes plus whether the input was the canonical encoding
/// (unused prefix bits zero). A noncanonical input cannot be reproduced byte-for-byte
/// by [`pack`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unpacked {
    pub raw: Vec<u8>,
    pub canonical: bool,
}

pub fn unpack(packed: &[u8]) -> Result<Unpacked, UnpackError> {
    let mut raw = Vec::with_capacity(packed.len() * 7 / 8 + 1);
    let mut canonical = true;
    let mut offset = 0;
    for group in packed.chunks(8) {
        if let Some((i, &v)) = group.iter().enumerate().find(|(_, &v)| v & 0x80 != 0) {
            return Err(UnpackError::HighBit {
                offset: offset + i,
                value: v,
            });
        }
        let prefix = group[0];
        let data = &group[1..];
        if data.is_empty() {
            return Err(UnpackError::EmptyGroup { offset });
        }
        for (i, b) in data.iter().enumerate() {
            raw.push(b | (((prefix >> i) & 1) << 7));
        }
        let used_mask: u8 = ((1u16 << data.len()) - 1) as u8;
        if prefix & !used_mask != 0 {
            canonical = false;
        }
        offset += group.len();
    }
    Ok(Unpacked { raw, canonical })
}

const _: () = assert!(packed_len_for(PAYLOAD_LEN) == PACKED_LEN);

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn golden_full_group() {
        let raw = [0x80, 0x01, 0xFF, 0x7F, 0x00, 0x81, 0x02];
        let packed = pack(&raw);
        assert_eq!(packed, vec![0x25, 0x00, 0x01, 0x7F, 0x7F, 0x00, 0x01, 0x02]);
        let u = unpack(&packed).unwrap();
        assert_eq!(u.raw, raw);
        assert!(u.canonical);
    }

    #[test]
    fn golden_short_group() {
        let raw = [0xFF, 0x80];
        assert_eq!(pack(&raw), vec![0x03, 0x7F, 0x00]);
        assert_eq!(unpack(&[0x03, 0x7F, 0x00]).unwrap().raw, raw);
    }

    #[test]
    fn payload_lengths() {
        let raw = vec![0xAAu8; PAYLOAD_LEN];
        let p = pack(&raw);
        assert_eq!(p.len(), 1171);
        assert!(p.iter().all(|b| *b < 0x80));
        assert_eq!(unpack(&p).unwrap().raw.len(), 1024);
    }

    #[test]
    fn noncanonical_final_prefix_detected() {
        // final group has 2 data bytes; set unused bit 5 in prefix
        let mut p = pack(&vec![0u8; PAYLOAD_LEN]);
        let final_prefix = p.len() - 3;
        p[final_prefix] |= 0x20;
        let u = unpack(&p).unwrap();
        assert!(!u.canonical);
        assert_eq!(u.raw, vec![0u8; PAYLOAD_LEN]);
        assert_ne!(pack(&u.raw), p);
    }

    #[test]
    fn rejects_high_bit() {
        assert!(matches!(
            unpack(&[0x00, 0x80]),
            Err(UnpackError::HighBit { offset: 1, .. })
        ));
    }

    #[test]
    fn boundary_values() {
        for v in [0x00u8, 0x7F, 0x80, 0xFF] {
            let raw = vec![v; PAYLOAD_LEN];
            assert_eq!(unpack(&pack(&raw)).unwrap().raw, raw);
        }
    }

    proptest! {
        #[test]
        fn roundtrip_full_payload(raw in proptest::collection::vec(any::<u8>(), PAYLOAD_LEN)) {
            let p = pack(&raw);
            prop_assert_eq!(p.len(), PACKED_LEN);
            let u = unpack(&p).unwrap();
            prop_assert!(u.canonical);
            prop_assert_eq!(&u.raw, &raw);
            prop_assert_eq!(pack(&u.raw), p);
        }

        #[test]
        fn roundtrip_any_len(raw in proptest::collection::vec(any::<u8>(), 1..200)) {
            let p = pack(&raw);
            prop_assert_eq!(p.len(), packed_len_for(raw.len()));
            prop_assert_eq!(unpack(&p).unwrap().raw, raw);
        }
    }
}
