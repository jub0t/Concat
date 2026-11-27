// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Live mode through the API: a call on the project the window has open
//! borrows the window's session, edits it there, and sends it home - after
//! a refusal and an error too - while every other project stays the API's
//! own.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;

/// A window for the API to borrow from: a session in a slot, whether it
/// lets callers edit it, whether the person is mid-gesture, and what came
/// home with the last loan.
#[derive(Default)]
struct Window {
    session: Mutex<Option<Session>>,
    live: AtomicBool,
    busy: AtomicBool,
    loans: AtomicUsize,
    last: Mutex<Option<Returned>>,
}

/// The link the API is given: the window, shared.
struct Link(Arc<Window>);

impl LiveLink for Link {
    fn lend(&self, path: &str) -> Result<Loan, ApiError> {
        let window = Arc::clone(&self.0);
        if !window.live.load(Ordering::SeqCst) {
            return Err(ApiError::new(
                ErrorCode::Refused,
                format!("{path} is open in the window, which does not let agents edit it"),
            ));
        }
        if window.busy.load(Ordering::SeqCst) {
            return Err(ApiError::new(ErrorCode::Busy, "mid-gesture"));
        }
        let session = window
            .session
            .lock()
            .expect("the slot")
            .take()
            .ok_or_else(|| ApiError::new(ErrorCode::NotOpen, "nothing open"))?;
        window.loans.fetch_add(1, Ordering::SeqCst);
        Ok(Loan::new(session, move |session, returned| {
            *window.session.lock().expect("the slot") = Some(session);
            *window.last.lock().expect("the record") = Some(returned);
        }))
    }
}

impl Window {
    /// How many lanes the window's timeline has, and whether it has
    /// something to undo.
    fn state(&self) -> (usize, bool) {
        let slot = self.session.lock().expect("the slot");
        let session = slot.as_ref().expect("the window's project is home");
        (lanes(&session.document()), session.can_undo())
    }
}

fn answered(response: Response) -> Reply {
    match response {
        Response::Result(reply) => reply,
        Response::Error(error) => panic!("refused: {error}"),
    }
}

fn error_of(response: Response) -> ApiError {
    match response {
        Response::Error(error) => error,
        Response::Result(_) => panic!("answered where a refusal was due"),
    }
}

/// How many lanes the timeline showing has.
fn lanes(document: &Model) -> usize {
    let showing = document.view.showing.expect("a timeline shows");
    document.lanes.index().of(showing).count()
}

/// The scene: a project made and let go by a scratch API, then open in a
/// window that lends it (or not); an API that shares the window's register
/// and knows the window; the project's folder.
struct Scene {
    api: Api,
    window: Arc<Window>,
    open: OpenProjects,
    path: String,
    scratch: tempfile::TempDir,
}

fn scene(live: bool) -> Scene {
    let scratch = tempfile::tempdir().expect("scratch");
    let dirs = AppDirs {
        config: scratch.path().join("config"),
        data: scratch.path().join("data"),
    };
    // A project, made and closed again so nothing holds it.
    let mut maker = Api::with_dirs(dirs.clone(), Arc::new(|_event: Event| {}));
    answered(maker.dispatch(Request::ProjectCreate {
        location: scratch.path().to_string_lossy().into_owned(),
        name: "on screen".to_owned(),
        video: None,
    }));
    let path = scratch
        .path()
        .join("on screen")
        .to_string_lossy()
        .into_owned();
    answered(maker.dispatch(Request::ProjectClose {
        path: path.clone(),
        save: true,
    }));
    drop(maker);

    // The window opens it, as the launch screen does: claimed, then read.
    let open = OpenProjects::default();
    open.claim(&path, Holder::Window)
        .expect("the window claims it");
    let info = projects::open(&path).expect("the project's info");
    let session = Session::open_info(&info).expect("the window's session");
    let window = Arc::new(Window::default());
    *window.session.lock().expect("the slot") = Some(session);
    window.live.store(live, Ordering::SeqCst);

    let mut api = Api::with_dirs(dirs, Arc::new(|_event: Event| {}));
    api.share_open_projects(open.clone());
    api.share_live(Arc::new(Link(Arc::clone(&window))));
    Scene {
        api,
        window,
        open,
        path,
        scratch,
    }
}

fn add_track(path: &str) -> Request {
    Request::EditApply {
        path: path.to_owned(),
        command: Box::new(Command::AddTrack { bottom: false }),
    }
}

#[test]
fn an_edit_lands_in_the_windows_session_as_a_step_to_undo() {
    let mut scene = scene(true);
    let (before, _) = scene.window.state();

    answered(scene.api.dispatch(Request::ProjectOpen {
        path: scene.path.clone(),
    }));
    answered(scene.api.dispatch(add_track(&scene.path)));

    let (after, can_undo) = scene.window.state();
    assert_eq!(after, before + 1, "the lane is on the window's timeline");
    assert!(can_undo, "and a step of the window's undo");
    assert!(
        scene.api.open_session(&scene.path).is_err(),
        "the API keeps no session of its own for the window's project"
    );
    assert_eq!(
        scene.open.holder(&scene.path),
        Some(Holder::Window),
        "the project is still the window's"
    );
}

#[test]
fn undo_through_the_api_takes_the_windows_last_step() {
    let mut scene = scene(true);
    let (before, _) = scene.window.state();
    answered(scene.api.dispatch(add_track(&scene.path)));
    answered(scene.api.dispatch(Request::EditUndo {
        path: scene.path.clone(),
    }));
    assert_eq!(scene.window.state().0, before);
}

#[test]
fn reading_the_project_reads_what_the_window_has() {
    let mut scene = scene(true);
    answered(scene.api.dispatch(add_track(&scene.path)));
    let (lanes_on_screen, _) = scene.window.state();
    match answered(scene.api.dispatch(Request::ProjectGet {
        path: scene.path.clone(),
    })) {
        Reply::View(view) => assert_eq!(
            lanes(&view.document().expect("the view carries the document")),
            lanes_on_screen
        ),
        _ => panic!("not a view"),
    }
}

#[test]
fn closing_saves_and_leaves_the_project_open_in_the_window() {
    let mut scene = scene(true);
    answered(scene.api.dispatch(add_track(&scene.path)));
    answered(scene.api.dispatch(Request::ProjectClose {
        path: scene.path.clone(),
        save: true,
    }));
    assert!(scene.window.session.lock().expect("the slot").is_some());
    assert_eq!(scene.open.holder(&scene.path), Some(Holder::Window));
    assert_eq!(
        *scene.window.last.lock().expect("the record"),
        Some(Returned { saved: true }),
        "the window is told nothing is unsaved"
    );
    // What was saved is the window's edit: read back from disk.
    let info = projects::open(&scene.path).expect("the project's info");
    let reopened = Session::open_info(&info).expect("reads what is saved");
    assert_eq!(lanes(&reopened.document()), scene.window.state().0);
}

#[test]
fn a_window_that_does_not_lend_says_so_and_keeps_its_project() {
    let mut scene = scene(false);
    let error = error_of(scene.api.dispatch(Request::ProjectOpen {
        path: scene.path.clone(),
    }));
    assert_eq!(error.code, ErrorCode::Refused);
    assert!(error.message.contains("does not let agents"));
    assert!(scene.window.session.lock().expect("the slot").is_some());
    assert_eq!(scene.window.loans.load(Ordering::SeqCst), 0);
}

#[test]
fn a_window_mid_gesture_is_busy() {
    let mut scene = scene(true);
    scene.window.busy.store(true, Ordering::SeqCst);
    let error = error_of(scene.api.dispatch(add_track(&scene.path)));
    assert_eq!(error.code, ErrorCode::Busy);
}

#[test]
fn the_session_goes_home_after_a_call_that_fails() {
    let mut scene = scene(true);
    let (before, _) = scene.window.state();
    // A clip that is not there cannot be removed.
    let error = error_of(scene.api.dispatch(Request::EditApply {
        path: scene.path.clone(),
        command: Box::new(Command::RemoveClips {
            clip_ids: vec!["not-a-clip".to_owned()],
            ripple: false,
        }),
    }));
    assert_ne!(error.code, ErrorCode::NotOpen);
    assert_eq!(scene.window.state().0, before, "home, and unchanged");
    assert_eq!(scene.window.loans.load(Ordering::SeqCst), 1);
}

#[test]
fn another_project_is_the_apis_own() {
    let mut scene = scene(true);
    answered(scene.api.dispatch(Request::ProjectCreate {
        location: scene.scratch.path().to_string_lossy().into_owned(),
        name: "off screen".to_owned(),
        video: None,
    }));
    let other = scene
        .scratch
        .path()
        .join("off screen")
        .to_string_lossy()
        .into_owned();
    answered(scene.api.dispatch(add_track(&other)));
    assert!(scene.api.open_session(&other).is_ok());
    assert_eq!(scene.open.holder(&other), Some(Holder::Api));
    assert_eq!(
        scene.window.loans.load(Ordering::SeqCst),
        0,
        "the window was never asked"
    );
}

#[test]
fn a_file_is_not_a_project() {
    let probe = Request::MediaProbe {
        path: "/tmp/clip.mp4".to_owned(),
    };
    assert_eq!(probe.project(), None);
    assert_eq!(add_track("/p").project(), Some("/p"));
    let peaks = Request::MediaPeaks {
        path: None,
        file: "/tmp/clip.mp4".to_owned(),
        stream: None,
        per_second: None,
    };
    assert_eq!(peaks.project(), None);
}

#[test]
fn the_api_says_it_can_go_live() {
    let mut scene = scene(true);
    match answered(scene.api.dispatch(Request::Version)) {
        Reply::Version(version) => {
            assert!(version.capabilities.iter().any(|name| name == "live"));
        }
        _ => panic!("not the version"),
    }
}
