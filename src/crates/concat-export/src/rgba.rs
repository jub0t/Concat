// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A colour as four straight channels, `[r, g, b, a]`, and the two ways a
//! settings file spells one: an array of three or four numbers, or
//! `#rrggbb` / `#rrggbbaa`.

use serde::{Deserialize, Deserializer, Serializer};

/// Opaque black: what shows where nothing covers the frame, unless a
/// project says otherwise.
pub const OPAQUE_BLACK: [u8; 4] = [0, 0, 0, 255];

/// Reads `[r, g, b]`, `[r, g, b, a]` or a `#` hex string.
pub fn array<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 4], D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Spelled {
        Channels(Vec<u8>),
        Hex(String),
    }
    match Spelled::deserialize(deserializer)? {
        Spelled::Channels(channels) => match channels.as_slice() {
            [r, g, b] => Ok([*r, *g, *b, 255]),
            [r, g, b, a] => Ok([*r, *g, *b, *a]),
            _ => Err(serde::de::Error::custom(
                "a colour is three or four channels",
            )),
        },
        Spelled::Hex(text) => {
            parse_hex(&text).ok_or_else(|| serde::de::Error::custom("not a #rrggbb colour"))
        }
    }
}

/// `#rrggbb` or `#rrggbbaa` as four channels.
pub fn parse_hex(text: &str) -> Option<[u8; 4]> {
    let digits = text.strip_prefix('#')?;
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    match digits.len() {
        6 => Some([channel(0)?, channel(2)?, channel(4)?, 255]),
        8 => Some([channel(0)?, channel(2)?, channel(4)?, channel(6)?]),
        _ => None,
    }
}

/// Four channels as `#rrggbb`, or `#rrggbbaa` when not opaque.
pub fn to_hex(rgba: [u8; 4]) -> String {
    let [r, g, b, a] = rgba;
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

/// Serde's `with` for a colour written as hex and read in either
/// spelling.
pub mod hex {
    use super::*;

    /// Writes `#rrggbb` or `#rrggbbaa`.
    pub fn serialize<S: Serializer>(rgba: &[u8; 4], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&to_hex(*rgba))
    }

    /// Reads either spelling.
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<[u8; 4], D::Error> {
        array(deserializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_spellings_read_and_hex_writes_short_when_opaque() {
        assert_eq!(parse_hex("#102030"), Some([16, 32, 48, 255]));
        assert_eq!(parse_hex("#10203040"), Some([16, 32, 48, 64]));
        assert_eq!(parse_hex("102030"), None);
        assert_eq!(to_hex([16, 32, 48, 255]), "#102030");
        assert_eq!(to_hex([16, 32, 48, 64]), "#10203040");
        let read: [u8; 4] = array(serde_json::json!([1, 2, 3])).expect("reads");
        assert_eq!(read, [1, 2, 3, 255]);
    }
}
