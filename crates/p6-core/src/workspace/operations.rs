//! Pure 500-slot bank operations. The frontend never re-implements these.
//!
//! Every operation takes a bank of exactly 500 cells and returns a new bank of
//! exactly 500 cells, or an error with no partial effect.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const BANK_LEN: usize = 500;

/// A New-bank cell referencing an immutable payload. `entry_id` is stable across moves;
/// copies/replacements get fresh ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Entry {
    pub entry_id: String,
    pub blob_hash: String,
    pub occurrence_id: Option<String>,
}

pub type Bank = Vec<Option<Entry>>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "code")]
pub enum OpError {
    #[error("{count} programs starting at {start:03} would pass slot 499; the highest valid start is {highest_valid_start:03}")]
    SelectionOverflow { start: usize, count: usize, highest_valid_start: usize },
    #[error("invalid destination {0}")]
    InvalidDestination(usize),
    #[error("nothing selected")]
    EmptySelection,
    #[error("{0}")]
    InvalidRanges(String),
    #[error("bank must have 500 slots, has {0}")]
    BadBankLength(usize),
    #[error("slot {0:03} is empty")]
    EmptySlot(usize),
}

fn check(bank: &Bank) -> Result<(), OpError> {
    if bank.len() != BANK_LEN { Err(OpError::BadBankLength(bank.len())) } else { Ok(()) }
}

fn sorted_unique(slots: &[usize]) -> Result<Vec<usize>, OpError> {
    let s: BTreeSet<usize> = slots.iter().copied().collect();
    if s.is_empty() {
        return Err(OpError::EmptySelection);
    }
    if let Some(&bad) = s.iter().find(|&&x| x >= BANK_LEN) {
        return Err(OpError::InvalidDestination(bad));
    }
    Ok(s.into_iter().collect())
}

/// Replace `start..start+k` with `items` (library -> New, or paste). No insertion/shifting.
pub fn replace_at(bank: &Bank, start: usize, items: Vec<Entry>) -> Result<Bank, OpError> {
    check(bank)?;
    let k = items.len();
    if k == 0 {
        return Err(OpError::EmptySelection);
    }
    if k > BANK_LEN {
        return Err(OpError::SelectionOverflow { start, count: k, highest_valid_start: 0 });
    }
    if start + k > BANK_LEN {
        return Err(OpError::SelectionOverflow { start, count: k, highest_valid_start: BANK_LEN - k });
    }
    let mut out = bank.clone();
    for (i, e) in items.into_iter().enumerate() {
        out[start + i] = Some(e);
    }
    Ok(out)
}

fn split_selected(bank: &Bank, sel: &[usize]) -> (Vec<Option<Entry>>, Vec<Option<Entry>>) {
    let set: BTreeSet<usize> = sel.iter().copied().collect();
    let mut selected = Vec::new();
    let mut remaining = Vec::new();
    for (i, c) in bank.iter().enumerate() {
        if set.contains(&i) { selected.push(c.clone()) } else { remaining.push(c.clone()) }
    }
    (selected, remaining)
}

/// Move selected slots so the block starts at final slot `target`.
pub fn move_to_slot(bank: &Bank, slots: &[usize], target: usize) -> Result<Bank, OpError> {
    check(bank)?;
    let sel = sorted_unique(slots)?;
    if target + sel.len() > BANK_LEN {
        return Err(OpError::SelectionOverflow { start: target, count: sel.len(), highest_valid_start: BANK_LEN - sel.len() });
    }
    let (selected, mut remaining) = split_selected(bank, &sel);
    let tail = remaining.split_off(target);
    Ok(remaining.into_iter().chain(selected).chain(tail).collect())
}

/// Move selected slots to insertion gap `gap` (0 = before first row, 500 = after last) of the original bank.
pub fn move_to_gap(bank: &Bank, slots: &[usize], gap: usize) -> Result<Bank, OpError> {
    check(bank)?;
    if gap > BANK_LEN {
        return Err(OpError::InvalidDestination(gap));
    }
    let sel = sorted_unique(slots)?;
    let j = gap - sel.iter().filter(|&&s| s < gap).count();
    move_to_slot(bank, &sel, j)
}

/// Exchange two equal-length, contiguous, nonoverlapping ranges.
pub fn swap_ranges(bank: &Bank, a: usize, b: usize, len: usize) -> Result<Bank, OpError> {
    check(bank)?;
    if len == 0 {
        return Err(OpError::EmptySelection);
    }
    if a + len > BANK_LEN || b + len > BANK_LEN {
        return Err(OpError::InvalidRanges("a range passes slot 499".into()));
    }
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    if lo + len > hi {
        return Err(OpError::InvalidRanges(format!("ranges {a:03}+{len} and {b:03}+{len} overlap")));
    }
    let mut out = bank.clone();
    for i in 0..len {
        out.swap(a + i, b + i);
    }
    Ok(out)
}

/// Stable-sort the entries at `slots` by `key`, putting them back in ascending slot order.
pub fn sort_selected<K: Ord>(bank: &Bank, slots: &[usize], key: impl Fn(&Option<Entry>) -> K) -> Result<Bank, OpError> {
    check(bank)?;
    let sel = sorted_unique(slots)?;
    let mut items: Vec<(usize, Option<Entry>)> = sel.iter().enumerate().map(|(i, s)| (i, bank[*s].clone())).collect();
    items.sort_by(|x, y| key(&x.1).cmp(&key(&y.1)).then(x.0.cmp(&y.0)));
    let mut out = bank.clone();
    for (dst, (_, e)) in sel.iter().zip(items) {
        out[*dst] = e;
    }
    Ok(out)
}

/// Replace selected destinations with the baseline cells.
pub fn revert_selected(bank: &Bank, baseline: &Bank, slots: &[usize]) -> Result<Bank, OpError> {
    check(bank)?;
    check(baseline)?;
    let sel = sorted_unique(slots)?;
    let mut out = bank.clone();
    for s in sel {
        if baseline[s].is_none() {
            return Err(OpError::EmptySlot(s));
        }
        out[s] = baseline[s].clone();
    }
    Ok(out)
}

/// Slots whose cell differs between two banks.
pub fn changed_slots(a: &Bank, b: &Bank) -> Vec<usize> {
    (0..BANK_LEN.min(a.len()).min(b.len())).filter(|&i| a[i] != b[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(label: &str) -> Option<Entry> {
        Some(Entry { entry_id: label.into(), blob_hash: label.into(), occurrence_id: None })
    }
    /// 500-slot bank whose first 8 entries are A..H and the rest z###.
    fn bank8() -> Bank {
        (0..BANK_LEN)
            .map(|i| if i < 8 { e(&((b'A' + i as u8) as char).to_string()) } else { e(&format!("z{i}")) })
            .collect()
    }
    fn head(b: &Bank) -> String {
        b.iter().take(8).map(|c| c.as_ref().unwrap().entry_id.clone()).collect::<Vec<_>>().join(" ")
    }
    fn multiset(b: &Bank) -> Vec<String> {
        let mut v: Vec<String> = b.iter().map(|c| c.as_ref().unwrap().entry_id.clone()).collect();
        v.sort();
        v
    }

    // Golden examples from the spec use an 8-entry bank. We reproduce them with an 8-entry
    // model by embedding at the start; moves to targets beyond the tail differ, so we
    // validate the 8-element permutation directly with a local helper too.
    fn move8(sel: &[usize], target: usize) -> String {
        let letters: Vec<String> = "ABCDEFGH".chars().map(|c| c.to_string()).collect();
        let selected: Vec<String> = sel.iter().map(|&i| letters[i].clone()).collect();
        let remaining: Vec<String> = (0..8).filter(|i| !sel.contains(i)).map(|i| letters[i].clone()).collect();
        let mut r = remaining[..target].to_vec();
        r.extend(selected);
        r.extend(remaining[target..].iter().cloned());
        r.join(" ")
    }

    #[test]
    fn golden_examples_in_500() {
        let b = bank8();
        assert_eq!(head(&move_to_slot(&b, &[1, 3], 4).unwrap()), "A C E F B D G H");
        assert_eq!(head(&move_to_gap(&b, &[1, 3], 6).unwrap()), "A C E F B D G H");
        assert_eq!(head(&move_to_slot(&b, &[1, 2], 5).unwrap()), "A D E F G B C H");
        assert_eq!(move8(&[1, 3], 4), "A C E F B D G H");
        assert_eq!(move8(&[1, 2], 5), "A D E F G B C H");
    }

    #[test]
    fn moves_preserve_multiset_and_length() {
        let b = bank8();
        for (sel, t) in [(vec![0usize, 99, 100, 250, 499], 0usize), (vec![5, 6, 7], 497), (vec![3], 400)] {
            let r = move_to_slot(&b, &sel, t).unwrap();
            assert_eq!(r.len(), 500);
            assert_eq!(multiset(&r), multiset(&b));
            for (i, s) in sel.iter().enumerate() {
                assert_eq!(r[t + i], b[*s]);
            }
        }
        assert!(matches!(move_to_slot(&b, &[1, 2, 3], 498), Err(OpError::SelectionOverflow { highest_valid_start: 497, .. })));
    }

    #[test]
    fn contiguous_drop_inside_own_span_is_noop() {
        let b = bank8();
        for g in 2..=5 {
            assert_eq!(move_to_gap(&b, &[2, 3, 4], g).unwrap(), b, "gap {g}");
        }
        assert_eq!(move_to_gap(&b, &[0], 500).unwrap()[499], b[0]);
    }

    #[test]
    fn replace_overflow_atomic() {
        let b = bank8();
        let items: Vec<Entry> = (0..20).map(|i| e(&format!("n{i}")).unwrap()).collect();
        assert!(replace_at(&b, 480, items.clone()).is_ok());
        let err = replace_at(&b, 481, items).unwrap_err();
        assert_eq!(err, OpError::SelectionOverflow { start: 481, count: 20, highest_valid_start: 480 });
        let r = replace_at(&b, 2, vec![e("X").unwrap()]).unwrap();
        assert_eq!(head(&r), "A B X D E F G H");
    }

    #[test]
    fn swap_rules() {
        let b = bank8();
        assert_eq!(head(&swap_ranges(&b, 0, 4, 2).unwrap()), "E F C D A B G H");
        assert!(swap_ranges(&b, 0, 1, 2).is_err());
        assert!(swap_ranges(&b, 0, 499, 2).is_err());
    }

    #[test]
    fn sort_stable_in_place() {
        let b = bank8();
        // sort slots 1,3,5 descending by id: B,D,F -> F,D,B
        let r = sort_selected(&b, &[1, 3, 5], |c| std::cmp::Reverse(c.as_ref().unwrap().entry_id.clone())).unwrap();
        assert_eq!(head(&r), "A F C D E B G H");
    }

    #[test]
    fn revert() {
        let b = bank8();
        let n = replace_at(&b, 0, vec![e("X").unwrap(), e("Y").unwrap()]).unwrap();
        let r = revert_selected(&n, &b, &[1]).unwrap();
        assert_eq!(head(&r), "X B C D E F G H");
        assert_eq!(changed_slots(&b, &r), vec![0]);
    }
}
