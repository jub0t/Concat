// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The `.cube` file: the look-up table format every grading tool writes.
//!
//! A header of `LUT_3D_SIZE n`, optionally a `TITLE` and a `DOMAIN_MIN` /
//! `DOMAIN_MAX`, then `n³` rows of three numbers, red changing fastest.
//! Read with the same tolerance as a manifest: comments and blank lines
//! are skipped, a domain other than 0..1 is rescaled, and anything the
//! rows do not add up to is an error naming the line.

use concat_core::Lut;

/// Parses the text of a `.cube` file into a table.
pub fn parse(text: &str) -> Result<Lut, String> {
    let mut size: Option<u32> = None;
    let mut domain_min = [0.0f32; 3];
    let mut domain_max = [1.0f32; 3];
    let mut rgb: Vec<f32> = Vec::new();
    for (number, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let mut words = line.split_whitespace();
        let head = words.next().unwrap_or("");
        let at = number + 1;
        match head {
            "TITLE" => {}
            "LUT_1D_SIZE" => return Err(format!("line {at}: a 1D table; only 3D tables are read")),
            "LUT_3D_SIZE" => {
                let n: u32 = words
                    .next()
                    .and_then(|w| w.parse().ok())
                    .ok_or_else(|| format!("line {at}: LUT_3D_SIZE needs a number"))?;
                if !(2..=129).contains(&n) {
                    return Err(format!("line {at}: a table {n} a side is out of range"));
                }
                size = Some(n);
                rgb.reserve((n as usize).pow(3) * 3);
            }
            "DOMAIN_MIN" | "DOMAIN_MAX" => {
                let mut triple = [0.0f32; 3];
                for slot in &mut triple {
                    *slot = words
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| format!("line {at}: {head} needs three numbers"))?;
                }
                if head == "DOMAIN_MIN" {
                    domain_min = triple;
                } else {
                    domain_max = triple;
                }
            }
            _ => {
                let mut triple = [0.0f32; 3];
                let mut rest = line.split_whitespace();
                for slot in &mut triple {
                    *slot = rest
                        .next()
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(|| format!("line {at}: expected three numbers"))?;
                }
                rgb.extend_from_slice(&triple);
            }
        }
    }
    let size = size.ok_or_else(|| "no LUT_3D_SIZE".to_owned())?;
    let expected = (size as usize).pow(3) * 3;
    if rgb.len() != expected {
        return Err(format!(
            "{} rows for a table {size} a side, which wants {}",
            rgb.len() / 3,
            expected / 3
        ));
    }
    for (i, value) in rgb.iter_mut().enumerate() {
        let channel = i % 3;
        let span = domain_max[channel] - domain_min[channel];
        if span.abs() > f32::EPSILON {
            *value = (*value - domain_min[channel]) / span;
        }
    }
    Lut::from_rgb(size, &rgb).ok_or_else(|| "the table did not fit".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(n: u32) -> String {
        let mut text = format!("TITLE \"id\"\nLUT_3D_SIZE {n}\n# rows\n");
        for b in 0..n {
            for g in 0..n {
                for r in 0..n {
                    let s = 1.0 / (n - 1) as f32;
                    text.push_str(&format!(
                        "{} {} {}\n",
                        r as f32 * s,
                        g as f32 * s,
                        b as f32 * s
                    ));
                }
            }
        }
        text
    }

    #[test]
    fn an_identity_cube_reads_as_the_identity() {
        let lut = parse(&identity(9)).expect("parses");
        assert_eq!(lut.size, 9);
        let out = lut.sample([0.3, 0.6, 0.9]);
        assert!(
            (out[0] - 0.3).abs() < 0.01
                && (out[1] - 0.6).abs() < 0.01
                && (out[2] - 0.9).abs() < 0.01
        );
    }

    #[test]
    fn a_short_table_names_the_shortfall() {
        let mut text = identity(3);
        text.push_str("0 0 0\n");
        assert!(parse(&text).unwrap_err().contains("28 rows"));
        assert!(parse("LUT_1D_SIZE 4\n").unwrap_err().contains("1D"));
    }
}
