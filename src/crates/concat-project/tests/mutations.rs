// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Project files damaged at random, thousands of them: none may crash the
//! reader, and whatever it opens must save to a file that opens to the
//! same save again.
//!
//! The seeds in `tests/seeds` are projects the corpus saved - clips, a
//! title, a mask, effects, a speed curve, a transition, lanes, entries from
//! a newer build - with their media's paths made plain. Each run takes a
//! seed and does a few things to it that a file from another build, a
//! half-written save or a hand edit could: drops a field, swaps a value for
//! one of another kind or an absurd size, repeats or removes an entry,
//! renames a key, copies an id onto its neighbour. The damage is drawn from
//! a fixed sequence, so a failure names the seed and the run that made it
//! and is the same on every machine.
//!
//! Two promises are held. The reader never panics, whatever it is given;
//! where there is nothing to open it says so by returning nothing. And a
//! save is a fixed point: what the reader opened, written out, read back
//! and written again, is the same document both times - so nothing it kept
//! is lost or changed by a second save, which is what makes "a build never
//! deletes what it cannot read" true past the first one.

use std::panic::{AssertUnwindSafe, catch_unwind};

use concat_project::{DocumentSettings, from_document, to_document};
use serde_json::{Map, Value, json};

/// Damaged documents tried per seed.
const RUNS: u64 = 4000;

/// The seeds, by name.
const SEEDS: [(&str, &str); 9] = [
    ("curve", include_str!("seeds/curve.json")),
    ("dissolve", include_str!("seeds/dissolve.json")),
    ("fades", include_str!("seeds/fades.json")),
    ("lanes", include_str!("seeds/lanes.json")),
    ("mask", include_str!("seeds/mask.json")),
    ("mute", include_str!("seeds/mute.json")),
    ("pip", include_str!("seeds/pip.json")),
    ("title", include_str!("seeds/title.json")),
    ("unknown", include_str!("seeds/unknown.json")),
];

/// A sequence of numbers that is the same on every machine (xorshift64*).
struct Dice(u64);

impl Dice {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A number under `sides`, which is over 0.
    fn roll(&mut self, sides: usize) -> usize {
        (self.next() % sides as u64) as usize
    }
}

fn settings() -> DocumentSettings {
    DocumentSettings {
        name: "mutations".to_owned(),
        width: 320,
        height: 180,
        rate_num: 30,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
    }
}

/// Every place in `value`, as the keys and indices that lead to it; the
/// root is the empty path.
fn places(value: &Value, here: &mut Vec<String>, all: &mut Vec<Vec<String>>) {
    all.push(here.clone());
    match value {
        Value::Object(object) => {
            for (key, inner) in object {
                here.push(key.clone());
                places(inner, here, all);
                here.pop();
            }
        }
        Value::Array(items) => {
            for (index, inner) in items.iter().enumerate() {
                here.push(index.to_string());
                places(inner, here, all);
                here.pop();
            }
        }
        _ => {}
    }
}

fn at<'a>(value: &'a mut Value, path: &[String]) -> Option<&'a mut Value> {
    path.iter().try_fold(value, |value, step| match value {
        Value::Object(object) => object.get_mut(step),
        Value::Array(items) => items.get_mut(step.parse::<usize>().ok()?),
        _ => None,
    })
}

/// A value no field expects where it is.
fn odd(dice: &mut Dice) -> Value {
    match dice.roll(16) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(0),
        3 => json!(-1),
        4 => json!(1e308),
        5 => json!(-1e308),
        6 => json!(1e-320),
        7 => json!(u64::MAX),
        8 => json!(i64::MIN),
        9 => json!(""),
        10 => json!("NaN"),
        11 => json!("x".repeat(4096)),
        12 => json!([]),
        13 => json!({}),
        14 => json!([[[[[[[[1]]]]]]]]),
        _ => json!(0.5),
    }
}

/// Does one thing to `document` at a place drawn from it.
fn damage(document: &mut Value, dice: &mut Dice) {
    let mut all = Vec::new();
    places(document, &mut Vec::new(), &mut all);
    let path = all[dice.roll(all.len())].clone();
    let Some((last, parent_path)) = path.split_last() else {
        return;
    };
    let kind = dice.roll(8);
    let replacement = odd(dice);
    let Some(parent) = at(document, parent_path) else {
        return;
    };
    match parent {
        Value::Object(object) => match kind {
            0 | 1 => {
                object.remove(last);
            }
            2..=4 => {
                object.insert(last.clone(), replacement);
            }
            5 => {
                if let Some(value) = object.remove(last) {
                    object.insert(format!("{last}Later"), value);
                }
            }
            6 => {
                // A field from another build beside it.
                object.insert("fromTheFuture".to_owned(), replacement);
            }
            _ => {
                // Its neighbour's value, so two entries can share an id.
                let keys: Vec<String> = object.keys().cloned().collect();
                let other = &keys[dice.roll(keys.len())];
                if let Some(value) = object.get(other).cloned() {
                    object.insert(last.clone(), value);
                }
            }
        },
        Value::Array(items) => {
            let Ok(index) = last.parse::<usize>() else {
                return;
            };
            match kind {
                0 | 1 => {
                    items.remove(index);
                }
                2 | 3 => items[index] = replacement,
                4 | 5 => {
                    let copy = items[index].clone();
                    items.insert(index, copy);
                }
                6 => items.truncate(index),
                _ => items.push(replacement),
            }
        }
        _ => {}
    }
}

/// What is wrong with how `document` is read and saved, if anything.
fn held(document: &Value) -> Result<(), String> {
    let settings = settings();
    let opened = catch_unwind(AssertUnwindSafe(|| from_document(document)))
        .map_err(|_| "the reader panicked".to_owned())?;
    let Some(project) = opened else {
        return Ok(());
    };
    let first = catch_unwind(AssertUnwindSafe(|| to_document(&settings, &project)))
        .map_err(|_| "the writer panicked on what the reader opened".to_owned())?;
    let again = catch_unwind(AssertUnwindSafe(|| from_document(&first)))
        .map_err(|_| "the reader panicked on its own save".to_owned())?
        .ok_or("the reader's own save does not open")?;
    let second = catch_unwind(AssertUnwindSafe(|| to_document(&settings, &again)))
        .map_err(|_| "the writer panicked on the second save".to_owned())?;
    if first != second {
        let (one, two) = (first.to_string(), second.to_string());
        let from = one
            .bytes()
            .zip(two.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(one.len().min(two.len()));
        let cut = |text: &str| -> String {
            text.chars()
                .skip(from.saturating_sub(120))
                .take(360)
                .collect()
        };
        return Err(format!(
            "a second save differs from the first.\nfirst:  …{}\nsecond: …{}",
            cut(&one),
            cut(&two)
        ));
    }
    Ok(())
}

#[test]
fn every_seed_reads_and_saves_to_a_fixed_point() {
    for (name, text) in SEEDS {
        let seed: Value = serde_json::from_str(text).expect("a seed is JSON");
        assert!(
            from_document(&seed).is_some(),
            "seed {name} does not open as it is"
        );
        held(&seed).unwrap_or_else(|why| panic!("seed {name}, undamaged: {why}"));
    }
}

#[test]
fn damaged_documents_never_crash_the_reader_and_save_to_a_fixed_point() {
    // The reader's own panics would print thousands of lines; one summary
    // is said at the end instead.
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut failures: Vec<String> = Vec::new();
    let mut opened = 0u64;
    for (number, (name, text)) in SEEDS.iter().enumerate() {
        let seed: Value = serde_json::from_str(text).expect("a seed is JSON");
        for run in 0..RUNS {
            let mut dice = Dice(0x9E37_79B9_7F4A_7C15 ^ ((number as u64) << 32) ^ (run + 1));
            let mut document = seed.clone();
            for _ in 0..=dice.roll(4) {
                damage(&mut document, &mut dice);
            }
            if from_document_quietly(&document) {
                opened += 1;
            }
            if let Err(why) = held(&document)
                && failures.len() < 5
            {
                failures.push(format!("seed {name}, run {run}: {why}"));
            }
        }
    }
    std::panic::set_hook(quiet);
    // Damage that leaves nothing openable proves nothing: most must open.
    let tried = RUNS * SEEDS.len() as u64;
    eprintln!("{opened} of {tried} damaged documents opened, and were saved twice");
    assert!(
        opened * 2 > tried,
        "only {opened} of {tried} damaged documents opened; the damage is too heavy to test saving"
    );
    assert!(
        failures.is_empty(),
        "{} of the first failures:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

/// Whether `document` opens, a panic counting as not.
fn from_document_quietly(document: &Value) -> bool {
    catch_unwind(AssertUnwindSafe(|| from_document(document)))
        .ok()
        .flatten()
        .is_some()
}

/// A save writes each number in as few digits as name it, and the reader
/// must come back to that same number: a clip's start a hair off after a
/// reopen is, on a frame's edge, a cut a frame over. Two hundred thousand
/// numbers with every bit of their precision in use, in the range a
/// project's times are in; about one in twelve came back a last digit off
/// before the reader was told to be exact.
#[test]
fn every_number_a_save_writes_is_read_back_as_it_was() {
    let mut dice = Dice(0x9E37_79B9_7F4A_7C15);
    let mut off = Vec::new();
    for _ in 0..200_000 {
        let value = (dice.next() >> 11) as f64 / (1u64 << 53) as f64 * 10_000.0;
        let text = serde_json::to_string(&json!({ "start": value })).expect("writes");
        let back: Value = serde_json::from_str(&text).expect("reads");
        if back["start"].as_f64().map(f64::to_bits) != Some(value.to_bits()) {
            off.push(text);
        }
    }
    assert!(
        off.is_empty(),
        "{} numbers came back changed, the first {}",
        off.len(),
        off[0]
    );
}

/// A document that is not an object, or is empty, or is only noise, is
/// nothing to open, said without a panic.
#[test]
fn what_is_no_document_is_nothing_to_open() {
    for value in [
        Value::Null,
        json!(1),
        json!("concat"),
        json!([]),
        Value::Object(Map::new()),
        json!({ "version": "two" }),
        json!({ "timelines": 4 }),
        json!({ "timelines": [], "media": {}, "clips": "none" }),
    ] {
        held(&value).unwrap_or_else(|why| panic!("{value}: {why}"));
    }
}
