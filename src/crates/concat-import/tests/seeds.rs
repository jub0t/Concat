// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Every seed project of the earlier format imports, and the document it
//! makes keeps every rule of the model.

use std::path::PathBuf;

fn seeds() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/seeds");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the seeds are there") {
        let path = entry.expect("an entry").path();
        if path.extension().is_some_and(|e| e == "json") {
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            out.push((name, std::fs::read_to_string(&path).expect("reads")));
        }
    }
    out.sort();
    out
}

#[test]
fn every_seed_imports_and_keeps_every_rule() {
    let seeds = seeds();
    assert!(seeds.len() >= 9, "found {} seeds", seeds.len());
    for (name, text) in seeds {
        let imported = concat_import::import(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        let document = imported.document;
        let again = concat_doc::Document::load(
            document.tables().clone(),
            document.mint().clone(),
            concat_doc::Revision::default(),
        );
        assert!(again.is_ok(), "{name}: {again:?}");
        for note in &imported.notes {
            eprintln!("{name}: {note}");
        }
    }
}
