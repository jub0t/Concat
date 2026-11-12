// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How a project document reaches the disk: whole or not at all, in the
//! order the documents were captured, and never at the cost of what was
//! there before. Also who may hold a project folder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, mpsc};

use serde_json::{Value, json};

use crate::lock::{self, LockError, ProjectLock};
use crate::projects::{self, ProjectInfo, Saved};
use crate::session::Session;

/// A document with a label to tell it apart and some bulk, so a write takes
/// long enough for a reader to catch one under way.
fn page(label: &str, bulk: usize) -> Value {
    json!({
        "label": label,
        "bulk": (0..bulk).map(|n| format!("{label}-{n}")).collect::<Vec<_>>(),
    })
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The names in a folder.
fn names(folder: &Path) -> BTreeSet<String> {
    std::fs::read_dir(folder)
        .expect("lists the folder")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect()
}

/// A project folder under a fresh scratch directory, made.
fn film(scratch: &tempfile::TempDir) -> PathBuf {
    let root = scratch.path().join("Film");
    std::fs::create_dir_all(&root).expect("the project folder");
    root
}

fn label_on_disk(root: &Path) -> String {
    projects::read_document(&text(root))
        .expect("reads")
        .expect("a document is there")["label"]
        .as_str()
        .expect("a label")
        .to_owned()
}

fn info_for(root: &Path) -> ProjectInfo {
    ProjectInfo {
        path: text(root),
        name: "Probe".to_owned(),
        width: 1280,
        height: 720,
        rate_num: 25,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
        opened_at: 0,
    }
}

fn track_count(session: &Session) -> usize {
    session.project().active().tracks.len()
}

// ── the write ──

#[test]
fn a_save_writes_the_whole_document_and_leaves_nothing_else() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    std::fs::write(root.join("notes.txt"), b"mine").expect("a file of the person's");
    let before = names(&root);

    let document = page("first", 50);
    projects::save(&text(&root), &document).expect("saves");
    assert_eq!(
        projects::read_document(&text(&root)).expect("reads"),
        Some(document.clone())
    );

    // The same document again: still exactly that document.
    projects::save(&text(&root), &document).expect("saves again");
    assert_eq!(
        projects::read_document(&text(&root)).expect("reads"),
        Some(document)
    );

    let mut expected = before;
    expected.insert("concat.json".to_owned());
    assert_eq!(names(&root), expected, "only the manifest was added");
    assert_eq!(
        std::fs::read(root.join("notes.txt")).expect("still there"),
        b"mine"
    );
}

#[test]
fn a_save_into_a_folder_that_is_not_there_makes_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = scratch.path().join("deep").join("er").join("Film");
    let document = page("made", 3);
    projects::save(&text(&root), &document).expect("saves");
    assert!(root.is_dir());
    assert_eq!(
        projects::read_document(&text(&root)).expect("reads"),
        Some(document)
    );
}

#[test]
fn a_manifest_under_its_older_name_is_written_under_that_name() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    std::fs::write(
        root.join("wolfcut.json"),
        serde_json::to_vec(&page("old", 1)).expect("encodes"),
    )
    .expect("an older project");

    let document = page("new", 2);
    projects::save(&text(&root), &document).expect("saves");
    let written: Value = serde_json::from_slice(
        &std::fs::read(root.join("wolfcut.json")).expect("reads the older name"),
    )
    .expect("parses");
    assert_eq!(written, document);
    assert!(
        !root.join("concat.json").exists(),
        "no second manifest appears beside it"
    );
    assert_eq!(names(&root), BTreeSet::from(["wolfcut.json".to_owned()]));
}

#[test]
fn leftover_temporaries_neither_stop_a_save_nor_are_read() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let pid = std::process::id();
    let mut leftovers = vec![
        "concat.json.saving".to_owned(),
        "concat.json.saving.1.0".to_owned(),
        "concat.json.saving.1.1".to_owned(),
    ];
    leftovers.extend((0..8).map(|n| format!("concat.json.saving.{pid}.{n}")));
    for name in &leftovers {
        std::fs::write(root.join(name), b"{ \"label\": \"half").expect("a crash's leftover");
    }

    let document = page("whole", 4);
    projects::save(&text(&root), &document).expect("saves past the leftovers");
    assert_eq!(
        projects::read_document(&text(&root)).expect("reads"),
        Some(document)
    );
    for name in &leftovers {
        // Whatever happens to them, none of them became the document.
        if let Ok(bytes) = std::fs::read(root.join(name)) {
            assert_eq!(bytes, b"{ \"label\": \"half");
        }
    }
}

// ── order ──

#[test]
fn the_plain_save_is_ordered_with_captured_ones() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    // Captured first, written last.
    let early = projects::ticket();
    projects::save(&text(&root), &page("plain", 1)).expect("saves");
    assert_eq!(
        projects::save_ordered(&text(&root), &page("early", 1), early).expect("not an error"),
        Saved::Superseded
    );
    assert_eq!(label_on_disk(&root), "plain");
}

#[test]
fn an_earlier_capture_written_late_is_skipped_not_failed() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let path = text(&root);
    let first = projects::ticket();
    let second = projects::ticket();
    let third = projects::ticket();

    assert_eq!(
        projects::save_ordered(&path, &page("third", 1), third).expect("writes"),
        Saved::Written
    );
    assert_eq!(
        projects::save_ordered(&path, &page("first", 1), first).expect("skips"),
        Saved::Superseded
    );
    assert_eq!(
        projects::save_ordered(&path, &page("second", 1), second).expect("skips"),
        Saved::Superseded
    );
    assert_eq!(label_on_disk(&root), "third");

    let fourth = projects::ticket();
    assert_eq!(
        projects::save_ordered(&path, &page("fourth", 1), fourth).expect("writes"),
        Saved::Written,
        "a newer capture still lands"
    );
    assert_eq!(label_on_disk(&root), "fourth");
    assert_eq!(names(&root), BTreeSet::from(["concat.json".to_owned()]));
}

#[test]
fn many_writers_at_once_leave_the_newest_capture() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let path = text(&root);
    const WRITERS: usize = 12;
    let captures: Vec<(Value, projects::Ticket)> = (0..WRITERS)
        .map(|n| (page(&format!("w{n}"), 200), projects::ticket()))
        .collect();
    let start = Arc::new(Barrier::new(WRITERS));
    let results: Vec<Saved> = std::thread::scope(|scope| {
        let handles: Vec<_> = captures
            .iter()
            .rev()
            .map(|(document, ticket)| {
                let start = Arc::clone(&start);
                let path = path.clone();
                scope.spawn(move || {
                    start.wait();
                    projects::save_ordered(&path, document, *ticket).expect("no failure here")
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("a writer"))
            .collect()
    });
    // The newest capture was spawned first (the list is reversed).
    assert_eq!(
        results[0],
        Saved::Written,
        "the newest capture always lands"
    );
    assert_eq!(label_on_disk(&root), format!("w{}", WRITERS - 1));
    assert_eq!(names(&root), BTreeSet::from(["concat.json".to_owned()]));
}

#[test]
fn a_reader_never_sees_half_a_document() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let path = text(&root);
    projects::save(&path, &page("w-start", 2000)).expect("the first save");

    let running = Arc::new(AtomicBool::new(true));
    let reader = {
        let running = Arc::clone(&running);
        let path = path.clone();
        std::thread::spawn(move || {
            let mut seen = 0usize;
            while running.load(Ordering::Relaxed) {
                match projects::read_document(&path) {
                    Ok(Some(document)) => {
                        let label = document["label"].as_str().expect("a whole document");
                        assert!(label.starts_with("w-"), "{label}");
                        assert_eq!(document["bulk"].as_array().map(Vec::len), Some(2000));
                        seen += 1;
                    }
                    Ok(None) => panic!("the manifest went missing during a save"),
                    // Windows may refuse an open while the file is being
                    // replaced; that is not a torn read.
                    Err(error) if cfg!(windows) => {
                        let _ = error;
                    }
                    Err(error) => panic!("read a broken manifest: {error}"),
                }
            }
            seen
        })
    };
    std::thread::scope(|scope| {
        for writer in 0..4 {
            let path = path.clone();
            scope.spawn(move || {
                for round in 0..15 {
                    let document = page(&format!("w-{writer}-{round}"), 2000);
                    let ticket = projects::ticket();
                    projects::save_ordered(&path, &document, ticket).expect("saves");
                }
            });
        }
    });
    running.store(false, Ordering::Relaxed);
    let seen = reader
        .join()
        .expect("the reader never saw a broken document");
    assert!(seen > 0, "the reader read at least once");
    assert_eq!(names(&root), BTreeSet::from(["concat.json".to_owned()]));
}

#[test]
fn order_is_kept_per_folder() {
    let scratch = tempfile::tempdir().expect("scratch");
    let one = scratch.path().join("One");
    let two = scratch.path().join("Two");
    let for_two = projects::ticket();
    let for_one = projects::ticket();
    assert_eq!(
        projects::save_ordered(&text(&one), &page("one", 1), for_one).expect("writes"),
        Saved::Written
    );
    assert_eq!(
        projects::save_ordered(&text(&two), &page("two", 1), for_two).expect("writes"),
        Saved::Written,
        "another folder's newer write does not hold this one back"
    );
    assert_eq!(label_on_disk(&two), "two");
}

#[test]
fn one_folder_spelled_two_ways_shares_one_order() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let older = projects::ticket();
    let newer = projects::ticket();
    let dotted = root.join(".");
    projects::save_ordered(&text(&dotted), &page("newer", 1), newer).expect("writes");
    assert_eq!(
        projects::save_ordered(&text(&root), &page("older", 1), older).expect("skips"),
        Saved::Superseded
    );
    assert_eq!(label_on_disk(&root), "newer");
}

#[cfg(unix)]
#[test]
fn a_folder_reached_through_a_link_shares_one_order() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let link = scratch.path().join("Shortcut");
    std::os::unix::fs::symlink(&root, &link).expect("a link to the folder");
    let older = projects::ticket();
    let newer = projects::ticket();
    projects::save_ordered(&text(&link), &page("newer", 1), newer).expect("writes");
    assert_eq!(
        projects::save_ordered(&text(&root), &page("older", 1), older).expect("skips"),
        Saved::Superseded
    );
    assert_eq!(label_on_disk(&root), "newer");
}

#[test]
fn a_failed_write_does_not_count_as_written() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let path = text(&root);
    let older = projects::ticket();
    let newer = projects::ticket();
    std::fs::create_dir(root.join("concat.json")).expect("a block at the manifest");
    projects::save_ordered(&path, &page("newer", 1), newer).expect_err("cannot land");
    std::fs::remove_dir(root.join("concat.json")).expect("the block goes");
    assert_eq!(
        projects::save_ordered(&path, &page("older", 1), older).expect("writes"),
        Saved::Written,
        "nothing newer reached the disk, so the older capture is the best there is"
    );
    assert_eq!(label_on_disk(&root), "older");
}

#[test]
fn a_capture_is_a_value_that_later_edits_do_not_reach() {
    let scratch = tempfile::tempdir().expect("scratch");
    let info = projects::create(
        &text(scratch.path()),
        "Snapshot",
        1280,
        720,
        25,
        1,
        [0, 0, 0, 255],
    )
    .expect("creates");
    let mut session = Session::open_info(&info).expect("opens");
    let before = track_count(&session);
    let (path, captured) = session.prepare_save(None);
    session
        .apply(concat_project::Command::AddTrack { bottom: false })
        .expect("adds a track");
    assert_ne!(session.prepare_save(None).1, captured);

    projects::save(&path, &captured).expect("writes the capture");
    let reopened = Session::open_info(&info).expect("reopens");
    assert_eq!(track_count(&reopened), before);
}

#[test]
fn autosaves_queued_before_a_close_never_land_after_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let info = projects::create(
        &text(scratch.path()),
        "Queued",
        1280,
        720,
        25,
        1,
        [0, 0, 0, 255],
    )
    .expect("creates");
    let mut session = Session::open_info(&info).expect("opens");
    let add = || concat_project::Command::AddTrack { bottom: false };

    // Two autosaves are captured, each before another edit, and wait on a
    // worker that is slow to start.
    let (path, first) = session.prepare_save(None);
    let first_ticket = projects::ticket();
    session.apply(add()).expect("an edit");
    let (_, second) = session.prepare_save(None);
    let second_ticket = projects::ticket();
    session.apply(add()).expect("another edit");
    let edited = track_count(&session);

    let (go, wait) = mpsc::channel::<()>();
    let worker = {
        let path = path.clone();
        std::thread::spawn(move || {
            wait.recv().expect("released");
            let a = projects::save_ordered(&path, &second, second_ticket);
            let b = projects::save_ordered(&path, &first, first_ticket);
            (a, b)
        })
    };

    // The close captures last and writes at once.
    let (_, closing) = session.prepare_save(None);
    let closing_ticket = projects::ticket();
    let closed = projects::save_ordered(&path, &closing, closing_ticket);
    go.send(()).expect("releases the worker");
    let (late_second, late_first) = worker.join().expect("the worker ends");

    assert_eq!(closed, Ok(Saved::Written));
    assert_eq!(late_second, Ok(Saved::Superseded));
    assert_eq!(late_first, Ok(Saved::Superseded));
    drop(session);
    let reopened = Session::open_info(&info).expect("reopens");
    assert_eq!(
        track_count(&reopened),
        edited,
        "the last edit is what is on disk"
    );
}

// ── failures ──

#[test]
fn a_directory_where_the_manifest_goes_fails_and_is_left_alone() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let manifest = root.join("concat.json");
    std::fs::create_dir(&manifest).expect("a directory in the way");
    std::fs::write(manifest.join("keep.txt"), b"keep").expect("something in it");
    let before = names(&root);

    let error = projects::save(&text(&root), &page("blocked", 1)).expect_err("cannot land");
    assert!(
        error.contains("concat.json"),
        "the error names the file it failed on: {error}"
    );
    assert!(manifest.is_dir());
    assert_eq!(
        std::fs::read(manifest.join("keep.txt")).expect("kept"),
        b"keep"
    );
    assert_eq!(names(&root), before, "no temporary is left behind");
}

#[cfg(unix)]
#[test]
fn a_read_only_folder_fails_and_keeps_the_saved_document() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempfile::tempdir().expect("scratch");
    let root = film(&scratch);
    let path = text(&root);
    projects::save(&path, &page("kept", 10)).expect("the first save");
    let manifest = projects::manifest_path(&root);
    let bytes = std::fs::read(&manifest).expect("reads");
    let before = names(&root);

    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o555)).expect("read-only");
    // An account that ignores permissions cannot show this.
    let privileged = std::fs::write(root.join("probe"), b"").is_ok();
    if !privileged {
        let error = projects::save(&path, &page("refused", 10)).expect_err("cannot write");
        assert!(!error.is_empty());
        assert_eq!(std::fs::read(&manifest).expect("reads"), bytes);
        assert_eq!(names(&root), before, "no temporary is left behind");
    }
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).expect("writable");
}

#[test]
fn a_file_where_the_folder_goes_fails_and_is_left_alone() {
    let scratch = tempfile::tempdir().expect("scratch");
    let root = scratch.path().join("Film");
    std::fs::write(&root, b"not a folder").expect("a file in the way");
    projects::save(&text(&root), &page("nowhere", 1)).expect_err("cannot make the folder");
    assert_eq!(std::fs::read(&root).expect("kept"), b"not a folder");
}

#[test]
fn opening_never_mistakes_a_broken_manifest_for_an_empty_project() {
    let scratch = tempfile::tempdir().expect("scratch");

    let missing = film(&scratch);
    assert_eq!(
        projects::read_document(&text(&missing)).expect("fine"),
        None
    );

    let broken = scratch.path().join("Broken");
    std::fs::create_dir_all(&broken).expect("folder");
    std::fs::write(
        broken.join("concat.json"),
        b"{ \"concat\": \"0.2\", \"name\": \"Cut",
    )
    .expect("a manifest cut short");
    assert!(projects::read_document(&text(&broken)).is_err());
    assert!(Session::open_info(&info_for(&broken)).is_err());

    let fresh = projects::create(
        &text(scratch.path()),
        "Fresh",
        1280,
        720,
        25,
        1,
        [0, 0, 0, 255],
    )
    .expect("creates");
    let session = Session::open_info(&fresh).expect("a settings-only project opens");
    assert!(session.project().active().clips.is_empty());
}

// ── who holds a folder ──

#[test]
fn the_lock_file_says_who_holds_the_folder_and_goes_with_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let held = ProjectLock::acquire(scratch.path(), "api").expect("taken");
    assert_eq!(held.path(), scratch.path().join(lock::FILE).as_path());
    let words = std::fs::read_to_string(held.path()).expect("readable while held");
    let lines: Vec<&str> = words.lines().collect();
    assert_eq!(lines.len(), 3, "{words:?}");
    assert_eq!(lines[0], std::process::id().to_string());
    assert_eq!(lines[1], "api");
    assert!(lines[2].parse::<u128>().expect("milliseconds") > 0);
    drop(held);
    assert!(!scratch.path().join(lock::FILE).exists());
}

#[test]
fn a_lock_file_nobody_holds_is_taken_over_whatever_it_says() {
    let scratch = tempfile::tempdir().expect("scratch");
    std::fs::write(scratch.path().join(lock::FILE), b"not a pid\n\n").expect("a leftover");
    let held = ProjectLock::acquire(scratch.path(), "window").expect("taken over");
    let words = std::fs::read_to_string(held.path()).expect("rewritten");
    assert!(words.starts_with(&format!("{}\nwindow\n", std::process::id())));
}

#[test]
fn a_folder_that_cannot_hold_a_lock_file_says_so() {
    let scratch = tempfile::tempdir().expect("scratch");
    std::fs::create_dir(scratch.path().join(lock::FILE)).expect("a directory in the way");
    match ProjectLock::acquire(scratch.path(), "window") {
        Err(LockError::Io(why)) => assert!(!why.is_empty()),
        other => panic!("expected an I/O refusal, got {other:?}"),
    }
}

#[test]
fn a_second_taker_is_refused_and_told_who_holds_it() {
    let scratch = tempfile::tempdir().expect("scratch");
    let _held = ProjectLock::acquire(scratch.path(), "window").expect("taken");
    match ProjectLock::acquire(scratch.path(), "api") {
        Err(LockError::Held(owner)) => {
            assert_eq!(owner.pid, Some(std::process::id()));
            assert_eq!(owner.role, "window");
        }
        other => panic!("expected a refusal naming the holder, got {other:?}"),
    }
}
