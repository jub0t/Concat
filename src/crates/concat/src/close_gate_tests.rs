// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Every way out of the window - its close button, the system's close,
//! Close Window, a version switch, a restart - saves the project first and
//! goes only when that save landed.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use concat_host::{Session, projects};

use crate::close_window;

/// A folder of this test's own under the temporary directory, removed when
/// the test ends however it ends.
struct Folder(PathBuf);

impl Folder {
    fn new(tag: &str) -> Folder {
        let path = std::env::temp_dir().join(format!(
            "concat-gate-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch folder");
        Folder(path)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn the_way_out_runs_only_after_the_close_succeeded() {
    let steps = RefCell::new(Vec::new());
    let outcome = close_window(
        || {
            steps.borrow_mut().push("close");
            Ok(())
        },
        || steps.borrow_mut().push("leave"),
    );
    assert_eq!(outcome, Ok(()));
    assert_eq!(*steps.borrow(), ["close", "leave"]);
}

#[test]
fn a_failed_close_never_leaves_and_hands_back_its_reason() {
    let left = Cell::new(0);
    let outcome = close_window(
        || Err("the disk went away".to_owned()),
        || left.set(left.get() + 1),
    );
    assert_eq!(outcome, Err("the disk went away".to_owned()));
    assert_eq!(left.get(), 0);
}

#[test]
fn a_relaunch_waiting_on_the_close_never_starts_when_the_save_fails() {
    // A version switch or a restart puts its start of the next run inside
    // the way out.
    let started = Cell::new(false);
    let hidden = Cell::new(false);
    let relaunch = || started.set(true);
    let outcome = close_window(
        || Err("could not save".to_owned()),
        || {
            relaunch();
            hidden.set(true);
        },
    );
    assert!(outcome.is_err());
    assert!(!started.get(), "no second run over this one");
    assert!(!hidden.get(), "the window stays");
}

/// Saves the session in `slot` and lets it go only when the save landed,
/// as closing a project does.
fn save_and_let_go(slot: &RefCell<Option<Session>>) -> Result<(), String> {
    let mut held = slot.borrow_mut();
    if let Some(session) = held.as_mut() {
        session.save(None)?;
    }
    *held = None;
    Ok(())
}

#[test]
fn a_close_that_cannot_save_keeps_the_session_and_a_retry_saves_it() {
    let scratch = Folder::new("retry");
    let info = projects::create(
        &scratch.0.to_string_lossy(),
        "Gate",
        1280,
        720,
        25,
        1,
        [0, 0, 0, 255],
    )
    .expect("creates");
    let root = PathBuf::from(&info.path);

    let mut session = Session::open_info(&info).expect("opens");
    session.save(None).expect("a first save");
    let saved_tracks = session.project().active().tracks.len();
    session
        .apply(concat_project::Command::AddTrack { bottom: false })
        .expect("an edit not yet on disk");
    let edited_tracks = session.project().active().tracks.len();

    // The project folder is taken away and a file stands in its place, as
    // when a drive is swapped under the app: nothing can be written there.
    let aside = scratch.0.join("Gate aside");
    std::fs::rename(&root, &aside).expect("moves the folder away");
    std::fs::write(&root, b"in the way").expect("a file where the folder was");

    let slot = RefCell::new(Some(session));
    let left = Cell::new(false);
    let outcome = close_window(|| save_and_let_go(&slot), || left.set(true));
    assert!(outcome.is_err(), "the save could not land");
    assert!(!left.get(), "so nothing left");
    assert_eq!(
        slot.borrow()
            .as_ref()
            .expect("the session is still held")
            .project()
            .active()
            .tracks
            .len(),
        edited_tracks,
        "with its edit"
    );
    assert_eq!(std::fs::read(&root).expect("untouched"), b"in the way");

    // The folder comes back; the next way out saves and leaves.
    std::fs::remove_file(&root).expect("the file goes");
    std::fs::rename(&aside, &root).expect("the folder is back");
    let on_disk = Session::open_info(&info).expect("reads what is saved");
    assert_eq!(
        on_disk.project().active().tracks.len(),
        saved_tracks,
        "the failed close wrote nothing"
    );
    drop(on_disk);

    let outcome = close_window(|| save_and_let_go(&slot), || left.set(true));
    assert_eq!(outcome, Ok(()));
    assert!(left.get());
    assert!(slot.borrow().is_none(), "let go once saved");
    let reopened = Session::open_info(&info).expect("reopens");
    assert_eq!(reopened.project().active().tracks.len(), edited_tracks);
    assert!(Path::new(&info.path).join("concat.json").is_file());
}
