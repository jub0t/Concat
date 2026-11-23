// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Every case in `src/corpus/cases`, a test each, so nextest runs each in
//! a process of its own with a timeout of its own and names the one that
//! failed. A case's report and its failed frames are written under
//! `target/corpus/<case>/`, which CI keeps when the run is red.

use std::path::PathBuf;

/// `target/corpus`, beside the build that is running.
fn out_dir() -> PathBuf {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    tmp.parent().unwrap_or(&tmp).join("corpus")
}

/// The engine's warnings and errors, on the test's own output.
struct Warnings;

impl log::Log for Warnings {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn
    }

    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("{} {}: {}", record.level(), record.target(), record.args());
        }
    }

    fn flush(&self) {}
}

/// Runs one case and fails with what its report found. The case's
/// directory is its name with dashes for the underscores.
fn check(name: &str) {
    // Once a process; a second set is refused and ignored.
    if log::set_logger(&Warnings).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
    let report = concat_corpus::run_case(name, &concat_corpus::cases_dir(), &out_dir());
    let result = out_dir().join(name).join("result.json");
    eprintln!(
        "{name}: {} checks in {:.1} s, the worst code block {:.1} levels off its level; {}",
        report.checks.len(),
        report.seconds,
        report.code_worst,
        result.display()
    );
    assert!(
        report.passed,
        "{name}:\n{}\nThe report is {}",
        report.summary(),
        result.display()
    );
}

/// One test for one case.
macro_rules! corpus_case {
    ($name:ident) => {
        #[test]
        fn $name() {
            check(&stringify!($name).replace('_', "-"));
        }
    };
}

corpus_case!(batch);
corpus_case!(codes);
corpus_case!(curve);
corpus_case!(cut);
corpus_case!(detach);
corpus_case!(dissolve);
corpus_case!(effect_key);
corpus_case!(exposure);
corpus_case!(fades);
corpus_case!(fit);
corpus_case!(fps60);
corpus_case!(gap);
corpus_case!(hidden);
corpus_case!(key_clear_one);
corpus_case!(keys_clear);
corpus_case!(lanes);
corpus_case!(layer);
corpus_case!(mask);
corpus_case!(merge);
corpus_case!(missing);
corpus_case!(mono);
corpus_case!(moved);
corpus_case!(mute);
corpus_case!(ntsc);
corpus_case!(opacity);
corpus_case!(pip);
corpus_case!(rates);
corpus_case!(reattach);
corpus_case!(relink);
corpus_case!(remove_media);
corpus_case!(remove_timeline);
corpus_case!(remove_track);
corpus_case!(resize);
corpus_case!(ripple);
corpus_case!(ripple_trim);
corpus_case!(rotate);
corpus_case!(select_timeline);
corpus_case!(sequence);
corpus_case!(shape);
corpus_case!(sound);
corpus_case!(speed);
corpus_case!(speed_ripple);
corpus_case!(split_speed);
corpus_case!(still);
corpus_case!(text_style);
corpus_case!(timeline_video);
corpus_case!(timelines);
corpus_case!(title);
corpus_case!(trim);
corpus_case!(unknown);
corpus_case!(vignette);
corpus_case!(volume);

/// The stored pictures stay small enough to live in the repository, and
/// every one of them is a picture some case names.
#[test]
fn the_golden_pictures_stay_in_budget() {
    let audit = concat_corpus::golden::audit(&concat_corpus::cases_dir(), Default::default())
        .expect("the cases can be listed");
    assert!(audit.problems.is_empty(), "{}", audit.problems.join("\n"));
}

#[test]
fn every_case_dir_has_a_test() {
    let source = include_str!("corpus.rs");
    let tested: Vec<String> = source
        .lines()
        .filter_map(|line| line.strip_prefix("corpus_case!("))
        .filter_map(|rest| rest.strip_suffix(");"))
        .map(|name| name.replace('_', "-"))
        .collect();
    let cases = concat_corpus::case::names(&concat_corpus::cases_dir())
        .unwrap_or_else(|error| panic!("{error}"));
    let untested: Vec<&String> = cases.iter().filter(|case| !tested.contains(case)).collect();
    assert!(
        untested.is_empty(),
        "these cases have no corpus_case! line in tests/corpus.rs: {untested:?}"
    );
    let missing: Vec<&String> = tested.iter().filter(|name| !cases.contains(name)).collect();
    assert!(
        missing.is_empty(),
        "these corpus_case! lines have no case directory: {missing:?}"
    );
}

#[test]
fn every_case_file_reads() {
    let cases = concat_corpus::cases_dir();
    for name in concat_corpus::case::names(&cases).unwrap_or_else(|error| panic!("{error}")) {
        if let Err(error) = concat_corpus::case::Case::load(&cases, &name) {
            panic!("{error}");
        }
    }
}
