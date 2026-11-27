// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A project of an earlier Concat opens: its document is made and saved,
//! its manifest kept as it was beside it, and nothing is lost to a crash
//! between the writes or to an open that fails.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::projects;
use crate::session::{Session, document_path};

/// Every seed project of the earlier format, by name, as its bytes.
fn seeds() -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../concat-import/tests/seeds");
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("the seeds are there") {
        let path = entry.expect("an entry").path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let name = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            out.push((name, std::fs::read(&path).expect("reads")));
        }
    }
    out.sort();
    out
}

/// A project folder in `scratch` whose manifest is `manifest`.
fn folder(scratch: &Path, name: &str, manifest: &[u8]) -> PathBuf {
    let root = scratch.join(name);
    std::fs::create_dir_all(&root).expect("folder");
    std::fs::write(root.join("concat.json"), manifest).expect("writes the manifest");
    root
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Every file in a folder and its bytes.
fn contents(root: &Path) -> BTreeMap<String, Vec<u8>> {
    std::fs::read_dir(root)
        .expect("lists the folder")
        .map(|entry| {
            let entry = entry.expect("an entry");
            let bytes = std::fs::read(entry.path()).unwrap_or_default();
            (entry.file_name().to_string_lossy().into_owned(), bytes)
        })
        .collect()
}

fn open(root: &Path) -> Result<Session, String> {
    Session::open_info(&projects::open(&text(root)).expect("reads the settings"))
}

#[test]
fn every_seed_opens_keeps_its_file_and_reopens_as_a_document() {
    let scratch = tempfile::tempdir().expect("scratch");
    let seeds = seeds();
    assert!(seeds.len() >= 9, "found {} seeds", seeds.len());
    for (name, bytes) in seeds {
        let root = folder(scratch.path(), &name, &bytes);
        let before = projects::open(&text(&root)).expect("the settings read before it opens");
        let session = open(&root).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(session.imported().is_some(), "{name}: made by the import");
        assert!(document_path(&root).is_file(), "{name}: the document");
        assert_eq!(
            std::fs::read(root.join("concat.earlier.json")).expect("the copy"),
            bytes,
            "{name}: the copy is the original, byte for byte"
        );
        let manifest = projects::read_document(&text(&root))
            .expect("reads")
            .expect("a manifest");
        assert!(projects::is_settings_only(&manifest), "{name}: {manifest}");
        let after = projects::open(&text(&root)).expect("the settings read after");
        assert_eq!(after.name, before.name, "{name}: the name is kept");
        assert_eq!(
            (after.width, after.height, after.rate_num, after.rate_den),
            (
                before.width,
                before.height,
                before.rate_num,
                before.rate_den
            ),
            "{name}: the frame is kept"
        );

        let again = open(&root).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(again.imported().is_none(), "{name}: the normal path");
        assert!(
            again.document().same_content(&session.document()),
            "{name}: the same document"
        );
    }
}

#[test]
fn a_crash_before_the_manifest_is_rewritten_loses_nothing() {
    let scratch = tempfile::tempdir().expect("scratch");
    let (name, bytes) = seeds().into_iter().next().expect("a seed");
    let root = folder(scratch.path(), &name, &bytes);
    let first = open(&root).expect("opens");
    // The document saved and the manifest not yet rewritten.
    std::fs::write(root.join("concat.json"), &bytes).expect("the manifest as it was");
    let again = open(&root).expect("reopens");
    assert!(again.imported().is_none(), "the document is there");
    assert!(again.document().same_content(&first.document()));
    assert_eq!(
        std::fs::read(root.join("concat.earlier.json")).expect("the copy"),
        bytes
    );
}

#[test]
fn a_copy_already_kept_is_never_replaced() {
    let scratch = tempfile::tempdir().expect("scratch");
    let (name, bytes) = seeds().into_iter().next().expect("a seed");
    let root = folder(scratch.path(), &name, &bytes);
    std::fs::write(root.join("concat.earlier.json"), b"the person's own").expect("writes");
    open(&root).expect("opens");
    assert_eq!(
        std::fs::read(root.join("concat.earlier.json")).expect("kept"),
        b"the person's own"
    );
    assert_eq!(
        std::fs::read(root.join("concat.earlier.2.json")).expect("beside it"),
        bytes,
        "the original goes beside the copy already there"
    );

    // The same bytes already kept are left as they are.
    let again = folder(scratch.path(), "again", &bytes);
    std::fs::write(again.join("concat.earlier.json"), &bytes).expect("writes");
    open(&again).expect("opens");
    assert!(!again.join("concat.earlier.2.json").exists());
}

#[test]
fn a_project_whose_manifest_is_wolfcut_json_opens_too() {
    let scratch = tempfile::tempdir().expect("scratch");
    let (name, bytes) = seeds().into_iter().next().expect("a seed");
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
    let writer = value
        .as_object_mut()
        .and_then(|map| map.remove("concat"))
        .expect("names its writer");
    value["wolfcut"] = writer;
    let manifest = serde_json::to_vec_pretty(&value).expect("encodes");
    let root = scratch.path().join(&name);
    std::fs::create_dir_all(&root).expect("folder");
    std::fs::write(root.join("wolfcut.json"), &manifest).expect("writes");
    let session = open(&root).expect("opens");
    let notes = session.imported().expect("made by the import");
    assert!(
        notes
            .iter()
            .all(|note| !note.to_string().contains("wolfcut")),
        "the writer's name is read, not kept: {notes:?}"
    );
    assert_eq!(
        std::fs::read(root.join("wolfcut.earlier.json")).expect("the copy"),
        manifest
    );
    let rewritten = projects::read_document(&text(&root))
        .expect("reads")
        .expect("a manifest");
    assert!(projects::is_settings_only(&rewritten), "{rewritten}");
    assert!(
        !root.join("concat.json").exists(),
        "the manifest keeps its name"
    );
    assert!(open(&root).expect("reopens").imported().is_none());
}

#[test]
fn a_manifest_that_cannot_be_imported_fails_and_the_folder_is_untouched() {
    let scratch = tempfile::tempdir().expect("scratch");
    let manifest = br#"{"concat":"0.2.12","name":"Old","video":{"width":1920,"height":1080,"rateNum":30,"rateDen":1},"timelines":[]}"#;
    let root = folder(scratch.path(), "Old", manifest);
    let before = contents(&root);
    let error = open(&root).err().expect("refused");
    assert!(error.contains("earlier Concat"), "{error}");
    assert!(error.contains("left as it was"), "{error}");
    assert_eq!(contents(&root), before);
}

#[test]
fn the_import_says_what_it_carried_over_approximately() {
    let scratch = tempfile::tempdir().expect("scratch");
    let noted = seeds()
        .into_iter()
        .filter(|(name, bytes)| {
            let root = folder(scratch.path(), name, bytes);
            let session = open(&root).expect("opens");
            let notes = session.imported().expect("made by the import");
            notes.iter().all(|note| !note.to_string().is_empty()) && !notes.is_empty()
        })
        .count();
    assert!(noted > 0, "some seed carries a part over approximately");
}

#[test]
fn the_recents_list_reads_an_earlier_project_before_it_opens() {
    let scratch = tempfile::tempdir().expect("scratch");
    let config = scratch.path().join("config");
    std::fs::create_dir_all(&config).expect("config");
    let (name, bytes) = seeds().into_iter().next().expect("a seed");
    let root = folder(scratch.path(), &name, &bytes);
    let info = projects::open(&text(&root)).expect("reads the settings");
    assert_eq!(info.name, name);
    assert_eq!((info.width, info.height), (320, 180));
    assert_eq!((info.rate_num, info.rate_den), (30, 1));
    projects::remember(&config, &info).expect("remembers");
    let listed = projects::list(&config);
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, name);
    assert!(
        !document_path(&root).exists(),
        "reading the settings writes nothing"
    );
}
