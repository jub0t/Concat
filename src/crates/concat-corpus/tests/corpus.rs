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

/// Runs one case and fails with what its report found. The case's
/// directory is its name with dashes for the underscores.
fn check(name: &str) {
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

corpus_case!(codes);
corpus_case!(cut);
corpus_case!(still);

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
