// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The token a connection presents: where one comes from when nobody
//! gave one, and how a presented one is checked.
//!
//! The check takes the same time whether the first byte is wrong or the
//! last, so a caller on the socket learns nothing from how quickly it is
//! refused. A library would do; this is short enough to read, and its
//! test says what it promises.

use std::hint::black_box;

/// A fresh token: 128 bits from the operating system's randomness, as
/// 32 hex digits.
pub(crate) fn mint() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("could not mint a token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Whether `presented` is `expected`, in time that depends on the length
/// of `expected` and on nothing about where the two differ.
///
/// Every byte of `expected` is compared, against a zero where `presented`
/// has run out, and the differences are or'd together with the lengths'
/// difference; only at the end is the total looked at. `black_box` keeps
/// the compiler from turning the loop back into an early return.
pub(crate) fn matches(presented: &[u8], expected: &[u8]) -> bool {
    let mut difference = presented.len() ^ expected.len();
    for (index, byte) in expected.iter().enumerate() {
        let other = presented.get(index).copied().unwrap_or(0);
        difference = black_box(difference | usize::from(byte ^ other));
    }
    black_box(difference) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minted_token_is_thirty_two_hex_digits_and_never_the_same() {
        let first = mint().expect("mints");
        let second = mint().expect("mints");
        assert_eq!(first.len(), 32);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn the_compare_accepts_the_token_and_nothing_near_it() {
        assert!(matches(b"open sesame", b"open sesame"));
        assert!(matches(b"", b""));
        assert!(!matches(b"open sesamf", b"open sesame"), "last byte");
        assert!(!matches(b"npen sesame", b"open sesame"), "first byte");
        assert!(!matches(b"open sesam", b"open sesame"), "a prefix");
        assert!(!matches(b"open sesame!", b"open sesame"), "too long");
        assert!(!matches(b"", b"open sesame"), "nothing");
        assert!(!matches(b"open sesame", b""), "something for nothing");
        // Where the presented token is short, the missing bytes read as
        // zero: a zero there must still not be taken as a match.
        assert!(!matches(b"a", b"a\0"), "a zero is not a missing byte");
    }
}
