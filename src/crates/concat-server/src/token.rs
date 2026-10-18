// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The token a connection presents: where one comes from when nobody
//! gave one, and how a presented one is checked.
//!
//! The check takes the same time whether the first byte is wrong or the
//! last, so a caller on the socket learns nothing from how quickly it is
//! refused. A library would do; this is short enough to read, and its
//! test says what it promises.

use std::collections::HashMap;
use std::hint::black_box;

/// The fewest characters a chosen token may have.
pub const MIN_CHARS: usize = 20;
/// The least a chosen token may be worth, in bits, by [`estimated_bits`].
pub const MIN_BITS: f64 = 64.0;

/// A fresh token: 128 bits from the operating system's randomness, as
/// 32 hex digits.
pub fn mint() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| format!("could not mint a token: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Refuses a token a person could guess or a script could walk through:
/// fewer than [`MIN_CHARS`] characters, or fewer than [`MIN_BITS`] by
/// [`estimated_bits`]. A minted token passes by a wide margin; "secret",
/// "aaaaaaaaaaaaaaaaaaaaaaaa" and "abababababababababab" do not.
pub fn check(token: &str) -> Result<(), String> {
    let chars = token.chars().count();
    if chars < MIN_CHARS {
        return Err(format!(
            "the token is {chars} characters; a token needs at least {MIN_CHARS}, or leave it empty and one is made"
        ));
    }
    if estimated_bits(token) < MIN_BITS {
        return Err(
            "the token repeats itself too much to be hard to guess; leave it empty and one is made"
                .to_owned(),
        );
    }
    Ok(())
}

/// A pessimistic guess at a token's strength: its length times the
/// Shannon entropy of its own characters. Repetition costs - a token of
/// one letter is worth nothing - and no dictionary is consulted, which is
/// why the floor in [`check`] is set where a random token clears it with
/// room and a phrase needs to be long.
pub fn estimated_bits(token: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    let mut total = 0usize;
    for char in token.chars() {
        *counts.entry(char).or_default() += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    let total = total as f64;
    let per_char: f64 = counts
        .values()
        .map(|&count| {
            let share = count as f64 / total;
            -share * share.log2()
        })
        .sum();
    per_char * total
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
    fn a_weak_token_is_refused_and_a_minted_one_is_not() {
        for _ in 0..64 {
            let minted = mint().expect("mints");
            assert!(check(&minted).is_ok(), "{minted}: {}", estimated_bits(&minted));
        }
        for weak in [
            "",
            "secret",
            "open sesame",
            "password12345678",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "abababababababababababababab",
            "abcabcabcabcabcabcabc",
        ] {
            assert!(check(weak).is_err(), "{weak:?} passed");
        }
        assert!(check("correct horse battery staple, purple").is_ok());
        assert!(check("Xk9#mQ2$vL7&pR4!tW8z").is_ok());
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
