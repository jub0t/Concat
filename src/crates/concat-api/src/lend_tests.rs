// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A project the window has open, lent to the API and given back: who holds
//! the folder at each step, and when the window is told.

use std::sync::atomic::{AtomicUsize, Ordering};

use super::project_close_tests::{answered, new_project, refused, scratch_api};
use super::*;

/// A window that lets go of the folder when asked, or says no, and counts
/// what it was told.
struct FakeWindow {
    open: OpenProjects,
    /// Whether `lend` lets go of the folder.
    lets_go: bool,
    /// What `lend` answers when it refuses.
    refusal: Option<String>,
    /// Whether another process takes the folder the moment it is let go;
    /// its lock, once it has.
    races: bool,
    raced: Mutex<Option<lock::ProjectLock>>,
    lends: AtomicUsize,
    returns: AtomicUsize,
}

impl FakeWindow {
    fn new(open: &OpenProjects) -> FakeWindow {
        FakeWindow {
            open: open.clone(),
            lets_go: true,
            refusal: None,
            races: false,
            raced: Mutex::new(None),
            lends: AtomicUsize::new(0),
            returns: AtomicUsize::new(0),
        }
    }

    fn returns(&self) -> usize {
        self.returns.load(Ordering::SeqCst)
    }
}

impl Lender for FakeWindow {
    fn lend(&self, path: &str) -> Result<(), String> {
        self.lends.fetch_add(1, Ordering::SeqCst);
        if let Some(why) = &self.refusal {
            return Err(why.clone());
        }
        if self.lets_go {
            self.open.release(path);
        }
        if self.races {
            // A second handle on the lock file is another process to the
            // operating system.
            let lock = lock::ProjectLock::acquire(Path::new(path), "window").expect("free");
            *self.raced.lock().expect("unpoisoned") = Some(lock);
        }
        Ok(())
    }

    fn give_back(&self, _path: &str) {
        self.returns.fetch_add(1, Ordering::SeqCst);
    }
}

/// An API sharing a register with a window that holds a saved project of
/// its own; the project's folder.
fn held_by_the_window(name: &str) -> (Api, OpenProjects, String, tempfile::TempDir) {
    let (mut api, scratch) = scratch_api();
    let open = OpenProjects::default();
    api.share_open_projects(open.clone());
    let path = new_project(&mut api, &scratch, name);
    answered(api.dispatch(Request::ProjectClose {
        path: path.clone(),
        save: true,
    }));
    open.claim(&path, Holder::Window)
        .expect("the window has it");
    (api, open, path, scratch)
}

fn close(api: &mut Api, path: &str) -> Response {
    api.dispatch(Request::ProjectClose {
        path: path.to_owned(),
        save: false,
    })
}

#[test]
fn the_window_lends_its_project_and_gets_it_back_on_close() {
    let (mut api, open, path, _scratch) = held_by_the_window("Lent");
    let window = Arc::new(FakeWindow::new(&open));
    api.set_lender(window.clone());

    answered(api.dispatch(Request::ProjectOpen { path: path.clone() }));
    assert_eq!(open.holder(&path), Some(Holder::Api));
    assert_eq!(window.lends.load(Ordering::SeqCst), 1);
    assert_eq!(window.returns(), 0, "not before the close");

    answered(close(&mut api, &path));
    assert_eq!(open.holder(&path), None, "let go before the window is told");
    assert_eq!(window.returns(), 1);
}

#[test]
fn a_window_that_will_not_lend_keeps_its_project() {
    let (mut api, open, path, _scratch) = held_by_the_window("Kept");
    let mut window = FakeWindow::new(&open);
    window.refusal = Some("the save failed".to_owned());
    let window = Arc::new(window);
    api.set_lender(window.clone());

    match api.dispatch(Request::ProjectOpen { path: path.clone() }) {
        Response::Error(error) => {
            assert_eq!(error.code, ErrorCode::Refused);
            assert!(error.message.contains("the save failed"), "{error}");
        }
        Response::Result(_) => panic!("opened a project the window kept"),
    }
    assert_eq!(open.holder(&path), Some(Holder::Window));
    assert_eq!(window.returns(), 0, "nothing was lent");
}

#[test]
fn a_window_that_says_yes_and_holds_on_is_refused_and_owed_nothing() {
    let (mut api, open, path, _scratch) = held_by_the_window("Held");
    let mut window = FakeWindow::new(&open);
    window.lets_go = false;
    let window = Arc::new(window);
    api.set_lender(window.clone());

    assert!(refused(
        api.dispatch(Request::ProjectOpen { path: path.clone() })
    ));
    assert_eq!(open.holder(&path), Some(Holder::Window));
    assert_eq!(window.returns(), 0);
    // Nothing is left over to give back later.
    drop(api);
    assert_eq!(window.returns(), 0);
}

#[test]
fn a_lent_project_another_process_takes_first_goes_back_to_the_window() {
    let (mut api, open, path, _scratch) = held_by_the_window("Raced");
    let mut window = FakeWindow::new(&open);
    window.races = true;
    let window = Arc::new(window);
    api.set_lender(window.clone());

    assert!(refused(
        api.dispatch(Request::ProjectOpen { path: path.clone() })
    ));
    assert_eq!(open.holder(&path), None, "the API did not get it");
    assert_eq!(window.returns(), 1, "so the window is told to take it back");
    drop(api);
    assert_eq!(window.returns(), 1, "and only once");
}

#[test]
fn a_project_the_window_never_held_is_not_given_back() {
    let (mut api, scratch) = scratch_api();
    let open = OpenProjects::default();
    api.share_open_projects(open.clone());
    let window = Arc::new(FakeWindow::new(&open));
    api.set_lender(window.clone());
    let path = new_project(&mut api, &scratch, "Own");

    answered(close(&mut api, &path));
    assert_eq!(window.lends.load(Ordering::SeqCst), 0);
    assert_eq!(window.returns(), 0);
}

#[test]
fn an_api_that_goes_gives_back_what_it_was_lent() {
    let (mut api, open, path, _scratch) = held_by_the_window("Stopped");
    let window = Arc::new(FakeWindow::new(&open));
    api.set_lender(window.clone());
    answered(api.dispatch(Request::ProjectOpen { path: path.clone() }));

    drop(api);
    assert_eq!(open.holder(&path), None);
    assert_eq!(window.returns(), 1);
}
