//! Typed program addresses.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Any stored-program address the P6 can report: banks 0-9, programs 0-99 (000-999).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct StoredAddress {
    pub bank: u8,
    pub program: u8,
}

impl StoredAddress {
    pub fn new(bank: u8, program: u8) -> Option<Self> {
        (bank <= 9 && program <= 99).then_some(Self { bank, program })
    }
    pub fn absolute(self) -> u16 {
        self.bank as u16 * 100 + self.program as u16
    }
    pub fn from_absolute(n: u16) -> Option<Self> {
        (n < 1000).then_some(Self {
            bank: (n / 100) as u8,
            program: (n % 100) as u8,
        })
    }
    pub fn user_slot(self) -> Option<UserSlot> {
        UserSlot::new(self.absolute())
    }
}

impl fmt::Display for StoredAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03}", self.absolute())
    }
}

/// A validated user-memory destination 000-499. The only type accepted as a write target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct UserSlot(u16);

impl UserSlot {
    pub fn new(n: u16) -> Option<Self> {
        (n < 500).then_some(Self(n))
    }
    pub fn index(self) -> usize {
        self.0 as usize
    }
    pub fn get(self) -> u16 {
        self.0
    }
    pub fn address(self) -> StoredAddress {
        StoredAddress {
            bank: (self.0 / 100) as u8,
            program: (self.0 % 100) as u8,
        }
    }
    pub fn all() -> impl Iterator<Item = UserSlot> {
        (0..500).map(UserSlot)
    }
}

impl TryFrom<u16> for UserSlot {
    type Error = String;
    fn try_from(v: u16) -> Result<Self, String> {
        UserSlot::new(v).ok_or_else(|| format!("slot {v} is outside user memory 000-499"))
    }
}
impl From<UserSlot> for u16 {
    fn from(s: UserSlot) -> u16 {
        s.0
    }
}
impl fmt::Display for UserSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:03}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges() {
        assert!(UserSlot::new(499).is_some());
        assert!(UserSlot::new(500).is_none());
        assert_eq!(
            UserSlot::new(123).unwrap().address(),
            StoredAddress {
                bank: 1,
                program: 23
            }
        );
        assert!(StoredAddress::new(10, 0).is_none());
        assert!(StoredAddress::new(5, 0).unwrap().user_slot().is_none());
        assert_eq!(StoredAddress::new(9, 99).unwrap().to_string(), "999");
        assert!(serde_json::from_str::<UserSlot>("500").is_err());
    }
}
