//! Three-way reconciliation of a staged bank against a fresh hardware observation.
//! O = old baseline, S = staged New, H = fresh hardware. Compared by payload hash.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SlotResolution {
    /// S == O: no staged change; adopt hardware.
    AdoptHardware,
    /// H == O: hardware unchanged; keep staged.
    KeepStaged,
    /// H == S: both agree.
    Agree,
    /// Staged slot is empty: explicit incomplete intent; needs review.
    EmptyStaged,
    /// All three differ.
    Conflict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictChoice {
    KeepNew,
    UseSynth,
}

pub fn resolve(o: Option<&str>, s: Option<&str>, h: &str) -> SlotResolution {
    match s {
        None => SlotResolution::EmptyStaged,
        Some(s) if s == h => SlotResolution::Agree,
        Some(s) if Some(s) == o => SlotResolution::AdoptHardware,
        Some(_) if o == Some(h) => SlotResolution::KeepStaged,
        Some(_) => SlotResolution::Conflict,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn table() {
        assert_eq!(
            resolve(Some("o"), Some("o"), "h"),
            SlotResolution::AdoptHardware
        );
        assert_eq!(
            resolve(Some("o"), Some("s"), "o"),
            SlotResolution::KeepStaged
        );
        assert_eq!(resolve(Some("o"), Some("x"), "x"), SlotResolution::Agree);
        assert_eq!(resolve(Some("o"), Some("s"), "h"), SlotResolution::Conflict);
        assert_eq!(resolve(Some("o"), None, "h"), SlotResolution::EmptyStaged);
        assert_eq!(resolve(None, Some("s"), "h"), SlotResolution::Conflict);
    }
}
