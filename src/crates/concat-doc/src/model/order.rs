// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Where a lane is in its sequence's stack, as a key that sorts.
//!
//! A lane's place could be its index, but then adding a lane between two
//! renumbers every lane above it: one edit becomes many, and two writers
//! adding lanes at once collide. An [`OrderKey`] is a fraction between
//! zero and one written in base 36, so there is always room for another
//! between any two, and a new lane is one new key with every other lane
//! untouched. (After the scheme in David Greenspan's "Implementing
//! Fractional Indexing", 2020.)

use std::fmt;

/// The digits, lowest first. Their ASCII order is their order as
/// numbers, so two keys compare as their text does.
const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// A place in an order: the digits of a fraction after its point, in
/// base 36.
///
/// Never empty and never ending in `0`, so each fraction has one
/// spelling and there is always a key below it as well as above it.
/// Keys compare as fractions: `"1" < "15" < "2"`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OrderKey(String);

/// Why a text is not an order key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NotAnOrderKey;

impl fmt::Display for NotAnOrderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an order key is one or more of 0-9 and a-z, and does not end in 0")
    }
}

impl std::error::Error for NotAnOrderKey {}

fn value(digit: u8) -> u32 {
    // Only called on keys that were checked.
    DIGITS.iter().position(|d| *d == digit).unwrap_or(0) as u32
}

impl OrderKey {
    /// The key a file spelled, if it is one.
    pub fn parse(text: &str) -> Result<OrderKey, NotAnOrderKey> {
        let valid = !text.is_empty()
            && text.bytes().all(|byte| DIGITS.contains(&byte))
            && !text.ends_with('0');
        if valid {
            Ok(OrderKey(text.to_owned()))
        } else {
            Err(NotAnOrderKey)
        }
    }

    /// The key halfway through: what the first lane of a sequence gets.
    pub fn middle() -> OrderKey {
        OrderKey::between(None, None).unwrap_or_else(|| OrderKey("i".to_owned()))
    }

    /// A key above `low` and below `high`, where `None` is the end of
    /// the order on that side. `None` when `low` is not below `high`.
    ///
    /// The answer is as short as the scheme allows: one digit while
    /// there is a digit between, one more each time two keys meet.
    ///
    /// ```
    /// use concat_doc::model::OrderKey;
    ///
    /// let one = OrderKey::parse("1").unwrap();
    /// let two = OrderKey::parse("2").unwrap();
    /// let between = OrderKey::between(Some(&one), Some(&two)).unwrap();
    /// assert!(one < between && between < two);
    /// assert_eq!(between.as_str(), "1i");
    /// ```
    pub fn between(low: Option<&OrderKey>, high: Option<&OrderKey>) -> Option<OrderKey> {
        if let (Some(low), Some(high)) = (low, high)
            && low >= high
        {
            return None;
        }
        let low: Vec<u32> = low.map_or_else(Vec::new, |key| key.0.bytes().map(value).collect());
        let high: Option<Vec<u32>> = high.map(|key| key.0.bytes().map(value).collect());
        let digits = midpoint(&low, high.as_deref());
        Some(OrderKey(
            digits
                .into_iter()
                .map(|d| DIGITS[d as usize] as char)
                .collect(),
        ))
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Below every key: not a key, only a bound to search from.
    pub(crate) fn floor() -> OrderKey {
        OrderKey(String::new())
    }
}

/// Digits strictly between `low` (missing digits are zeros) and `high`
/// (`None` is one), not ending in zero. `low < high` as fractions.
fn midpoint(low: &[u32], high: Option<&[u32]>) -> Vec<u32> {
    if let Some(high) = high {
        let mut shared = 0;
        while shared < high.len() && low.get(shared).copied().unwrap_or(0) == high[shared] {
            shared += 1;
        }
        if shared > 0 {
            let mut out = high[..shared].to_vec();
            out.extend(midpoint(
                low.get(shared..).unwrap_or(&[]),
                Some(&high[shared..]),
            ));
            return out;
        }
    }
    let low_digit = low.first().copied().unwrap_or(0);
    let high_digit = high.map_or(36, |high| high[0]);
    if high_digit - low_digit > 1 {
        return vec![(low_digit + high_digit) / 2];
    }
    if let Some(high) = high
        && high.len() > 1
    {
        // `high` goes on past its first digit, so that digit alone is
        // below it, and it is above `low`, which starts lower.
        return vec![high[0]];
    }
    let mut out = vec![low_digit];
    out.extend(midpoint(low.get(1..).unwrap_or(&[]), None));
    out
}

impl fmt::Debug for OrderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

impl fmt::Display for OrderKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    fn key(text: &str) -> OrderKey {
        OrderKey::parse(text).unwrap()
    }

    #[test]
    fn a_key_is_digits_that_do_not_end_in_zero() {
        assert!(OrderKey::parse("i").is_ok());
        assert!(OrderKey::parse("0001").is_ok());
        assert_eq!(OrderKey::parse(""), Err(NotAnOrderKey));
        assert_eq!(OrderKey::parse("10"), Err(NotAnOrderKey));
        assert_eq!(OrderKey::parse("A"), Err(NotAnOrderKey));
        assert_eq!(OrderKey::parse("1.5"), Err(NotAnOrderKey));
    }

    #[test]
    fn keys_compare_as_fractions() {
        assert!(key("1") < key("15"));
        assert!(key("15") < key("2"));
        assert!(key("z") > key("yzzz"));
    }

    #[test]
    fn there_is_always_room_on_either_side_and_between() {
        assert_eq!(OrderKey::middle().as_str(), "i");
        let low = key("1");
        assert_eq!(OrderKey::between(None, Some(&low)).unwrap().as_str(), "0i");
        let high = key("z");
        assert_eq!(OrderKey::between(Some(&high), None).unwrap().as_str(), "zi");
        assert_eq!(OrderKey::between(Some(&high), Some(&low)), None);
        assert_eq!(OrderKey::between(Some(&low), Some(&low)), None);
    }

    proptest! {
        #[test]
        fn a_key_between_two_is_between_them_and_is_a_key(
            moves in prop::collection::vec((any::<bool>(), any::<prop::sample::Index>()), 1..60),
        ) {
            // Lanes added again and again at random places of a stack.
            let mut stack: Vec<OrderKey> = vec![OrderKey::middle()];
            for (at_end, index) in moves {
                let at = if at_end { stack.len() } else { index.index(stack.len() + 1) };
                let low = at.checked_sub(1).map(|below| &stack[below]);
                let high = stack.get(at);
                let made = OrderKey::between(low, high).unwrap();
                prop_assert!(OrderKey::parse(made.as_str()).is_ok());
                if let Some(low) = low { prop_assert!(*low < made); }
                if let Some(high) = high { prop_assert!(made < *high); }
                stack.insert(at, made);
            }
            prop_assert!(stack.windows(2).all(|pair| pair[0] < pair[1]));
        }
    }
}
