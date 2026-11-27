// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Project files damaged at random, tens of thousands of them: none may
//! crash the reader, and whatever it opens must save to a file that opens
//! to the same save again.
//!
//! The seeds are files in the format: the small project of its
//! documentation, a project holding one of everything the model has,
//! projects made by runs of random edits, and a file as a later build
//! writes it, with fields, choices and records this build does not know,
//! records that need features it lacks, and what was derived from them.
//! Each run takes a seed and does a few things to it that a file from
//! another build, a half-written save or a hand edit could: drops a
//! field, swaps a value for one of another kind or an absurd size,
//! repeats or removes an entry, renames a key, copies a value onto its
//! neighbour so two rows or entries share an id. Other runs cut the text
//! short or change one of its bytes. The damage is drawn from a fixed
//! sequence, so a failure names the seed and the run that made it and is
//! the same on every machine.
//!
//! Two promises are held. The reader never panics, whatever it is given;
//! where there is nothing to open it says why. And a save is a fixed
//! point: what the reader opened, written out, read back and written
//! again, is the same bytes both times - so nothing it kept is lost or
//! changed by a second save, which is what makes "a build never deletes
//! what it cannot read" true past the first one.

mod edits;
mod fixture;

use std::panic::{AssertUnwindSafe, catch_unwind};

use concat_doc::Origin;
use concat_doc::file;
use proptest::strategy::{Strategy, ValueTree};
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use serde_json::{Value, json};

/// Damaged documents tried per seed, by value and by byte.
const RUNS: u64 = 2500;
const BYTE_RUNS: u64 = 500;

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

/// The seeds, by name.
fn seeds() -> Vec<(String, String)> {
    let mut seeds = vec![
        (
            "later".to_owned(),
            include_str!("files/later.json").to_owned(),
        ),
        ("everything".to_owned(), file::write(&edits::everything())),
        ("start".to_owned(), file::write(&edits::start())),
    ];
    // Projects made by runs of random edits, from fixed seeds.
    for n in 0..5_u8 {
        let rng = TestRng::from_seed(RngAlgorithm::ChaCha, &[n + 1; 32]);
        let mut runner = TestRunner::new_with_rng(Config::default(), rng);
        let ops = proptest::collection::vec(edits::op(), 30..60)
            .new_tree(&mut runner)
            .expect("a run of edits")
            .current();
        let mut doc = edits::start();
        for op in &ops {
            let mut draft = doc.begin("Random", Origin(0));
            edits::run(&mut draft, op);
            if let Ok(commit) = draft.commit() {
                doc = commit.into_document();
            }
        }
        seeds.push((format!("edits {n}"), file::write(&doc)));
    }
    seeds
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
    match dice.roll(18) {
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
        15 => json!("c281474976710655"),
        16 => json!({"needs": ["x"]}),
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

/// The text cut short, or one byte of it changed.
fn damage_bytes(text: &str, dice: &mut Dice) -> Vec<u8> {
    let mut bytes = text.as_bytes().to_vec();
    let at = dice.roll(bytes.len().max(1));
    match dice.roll(3) {
        0 => bytes.truncate(at),
        1 => {
            const ODD: &[u8] = b"{}[]\",:0-.e\\x \xff";
            if let Some(byte) = bytes.get_mut(at) {
                *byte = ODD[dice.roll(ODD.len())];
            }
        }
        _ => {
            const EXTRA: &[u8] = b",\"}]1";
            bytes.insert(at.min(bytes.len()), EXTRA[dice.roll(EXTRA.len())]);
        }
    }
    bytes
}

/// Whether `bytes` opened, with something set aside or put right, and
/// what is wrong with how it is read and saved, if anything.
fn held(bytes: &[u8]) -> Result<Opening, String> {
    let opened = catch_unwind(AssertUnwindSafe(|| file::read(bytes)))
        .map_err(|_| "the reader panicked".to_owned())?;
    let Ok(opened) = opened else {
        return Ok(Opening::Refused);
    };
    let noted = !opened.notes.is_empty();
    let first = catch_unwind(AssertUnwindSafe(|| file::write(&opened.document)))
        .map_err(|_| "the writer panicked on what the reader opened".to_owned())?;
    let again = catch_unwind(AssertUnwindSafe(|| file::read(first.as_bytes())))
        .map_err(|_| "the reader panicked on its own save".to_owned())?
        .map_err(|why| format!("the reader's own save does not open: {why}\n{first}"))?;
    let second = catch_unwind(AssertUnwindSafe(|| file::write(&again.document)))
        .map_err(|_| "the writer panicked on the second save".to_owned())?;
    if first != second {
        let from = first
            .bytes()
            .zip(second.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(first.len().min(second.len()));
        let cut = |text: &str| -> String {
            text.chars()
                .skip(from.saturating_sub(160))
                .take(400)
                .collect()
        };
        return Err(format!(
            "a second save differs from the first.\nfirst:  …{}\nsecond: …{}",
            cut(&first),
            cut(&second)
        ));
    }
    Ok(if noted {
        Opening::Noted
    } else {
        Opening::Clean
    })
}

/// How a file opened.
#[derive(PartialEq, Debug)]
enum Opening {
    /// It did not: it was refused, said why.
    Refused,
    /// With nothing to say.
    Clean,
    /// With something set aside or put right, and said.
    Noted,
}

#[test]
fn every_seed_reads_and_saves_to_a_fixed_point() {
    for (name, text) in seeds() {
        assert!(
            file::read(text.as_bytes()).is_ok(),
            "seed {name} does not open as it is"
        );
        let expected = if name == "later" {
            Opening::Noted
        } else {
            Opening::Clean
        };
        assert_eq!(
            held(text.as_bytes()),
            Ok(expected),
            "seed {name}, undamaged"
        );
        // Respelled as another program writes JSON, keys in another order,
        // a file this build wrote reads as the same document. (Text kept
        // as it was read keeps the other program's spelling.)
        if name != "later" {
            let respelled = serde_json::from_str::<Value>(&text).unwrap().to_string();
            let opened = file::read(respelled.as_bytes()).unwrap();
            assert_eq!(opened.notes, [], "seed {name}, respelled");
            assert_eq!(
                file::write(&opened.document),
                text,
                "seed {name}, respelled"
            );
        }
    }
}

#[test]
fn damaged_files_never_crash_the_reader_and_save_to_a_fixed_point() {
    // The reader's own panics would print thousands of lines; one summary
    // is said at the end instead.
    let quiet = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let mut failures: Vec<String> = Vec::new();
    let (mut opened, mut noted, mut tried) = (0_u64, 0_u64, 0_u64);
    let (mut bytes_opened, mut bytes_tried) = (0_u64, 0_u64);
    for (number, (name, text)) in seeds().iter().enumerate() {
        let seed: Value = serde_json::from_str(text).expect("a seed is JSON");
        for run in 0..RUNS {
            let mut dice = Dice(0x9E37_79B9_7F4A_7C15 ^ ((number as u64) << 32) ^ (run + 1));
            let mut document = seed.clone();
            for _ in 0..=dice.roll(4) {
                damage(&mut document, &mut dice);
            }
            let damaged = serde_json::to_string(&document).expect("JSON writes");
            tried += 1;
            match held(damaged.as_bytes()) {
                Ok(Opening::Clean) => opened += 1,
                Ok(Opening::Noted) => {
                    opened += 1;
                    noted += 1;
                }
                Ok(Opening::Refused) => {}
                Err(why) if failures.len() < 5 => {
                    failures.push(format!("seed {name}, run {run}: {why}\n{damaged}"));
                }
                Err(_) => {}
            }
        }
        for run in 0..BYTE_RUNS {
            let mut dice = Dice(0x5851_F42D_4C95_7F2D ^ ((number as u64) << 32) ^ (run + 1));
            let damaged = damage_bytes(text, &mut dice);
            bytes_tried += 1;
            match held(&damaged) {
                Ok(Opening::Clean | Opening::Noted) => bytes_opened += 1,
                Ok(Opening::Refused) => {}
                Err(why) if failures.len() < 5 => {
                    failures.push(format!("seed {name}, byte run {run}: {why}"));
                }
                Err(_) => {}
            }
        }
    }
    std::panic::set_hook(quiet);
    eprintln!(
        "{opened} of {tried} damaged documents opened and were saved twice, {noted} of them \
         with something set aside or put right; {bytes_opened} of {bytes_tried} with damaged bytes"
    );
    // Damage that sets nothing aside does not test setting aside.
    assert!(
        noted * 4 > tried,
        "only {noted} of {tried} damaged documents had something set aside"
    );
    // Damage that leaves nothing openable proves nothing: most must open.
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

/// A file that is not an object, or is empty, or is only noise, is
/// nothing to open, said without a panic.
#[test]
fn what_is_no_document_is_nothing_to_open() {
    for value in [
        Value::Null,
        json!(1),
        json!("concat"),
        json!([]),
        json!({}),
        json!({ "concat-project": "one" }),
        json!({ "concat-project": 1, "items": 4 }),
        json!({ "concat-project": 1, "mint": [], "view": 3, "media": [], "fonts": {} }),
    ] {
        let text = value.to_string();
        assert!(held(text.as_bytes()).is_ok(), "{text}");
    }
}
