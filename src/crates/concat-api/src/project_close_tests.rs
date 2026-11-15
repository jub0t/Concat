// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Saving and closing a project through the API when the disk refuses the
//! write: the edit stays in the session, the folder stays held, and a later
//! try lands it.

use super::*;

/// An API with its own scratch config and data folders, and a sink that
/// drops events.
fn scratch_api() -> (Api, tempfile::TempDir) {
    let scratch = tempfile::tempdir().expect("scratch");
    let dirs = AppDirs {
        config: scratch.path().join("config"),
        data: scratch.path().join("data"),
    };
    let api = Api::with_dirs(dirs, Arc::new(|_event: Event| {}));
    (api, scratch)
}

fn answered(response: Response) -> Reply {
    match response {
        Response::Result(reply) => reply,
        Response::Error(error) => panic!("refused: {error}"),
    }
}

fn refused(response: Response) -> bool {
    matches!(response, Response::Error(_))
}

/// A new project called `name`, open in `api`; its folder.
fn new_project(api: &mut Api, scratch: &tempfile::TempDir, name: &str) -> String {
    answered(api.dispatch(Request::ProjectCreate {
        location: scratch.path().to_string_lossy().into_owned(),
        name: name.to_owned(),
        video: None,
    }));
    scratch.path().join(name).to_string_lossy().into_owned()
}

fn add_track(api: &mut Api, path: &str) {
    answered(api.dispatch(Request::EditApply {
        path: path.to_owned(),
        command: Box::new(Command::AddTrack { bottom: false }),
    }));
}

fn tracks_open(api: &mut Api, path: &str) -> usize {
    match answered(api.dispatch(Request::ProjectGet {
        path: path.to_owned(),
    })) {
        Reply::View(view) => view.project.active().tracks.len(),
        _ => panic!("not a view"),
    }
}

/// Puts a directory where the manifest goes, so a save cannot land.
fn block(path: &str) {
    let manifest = projects::manifest_path(Path::new(path));
    if manifest.is_file() {
        std::fs::remove_file(&manifest).expect("moves the manifest away");
    }
    std::fs::create_dir(&manifest).expect("blocks the manifest");
}

fn unblock(path: &str) {
    std::fs::remove_dir(Path::new(path).join("concat.json")).expect("clears the block");
}

#[test]
fn a_save_the_disk_refuses_keeps_the_edit_and_a_later_save_lands() {
    let (mut api, scratch) = scratch_api();
    let path = new_project(&mut api, &scratch, "Refused");
    let before = tracks_open(&mut api, &path);
    add_track(&mut api, &path);

    block(&path);
    assert!(refused(api.dispatch(Request::ProjectSave {
        path: path.clone(),
        name: None,
    })));
    assert_eq!(
        tracks_open(&mut api, &path),
        before + 1,
        "the edit is still held"
    );

    unblock(&path);
    answered(api.dispatch(Request::ProjectSave {
        path: path.clone(),
        name: None,
    }));
    // Read back by a session of its own, as the next open would.
    let probe = Session::open_info(&projects::open(&path).expect("settings")).expect("opens");
    assert_eq!(probe.project().active().tracks.len(), before + 1);
}

#[test]
fn a_close_whose_save_fails_keeps_the_project_open_and_held() {
    let (mut api, scratch) = scratch_api();
    let path = new_project(&mut api, &scratch, "Kept");
    let before = tracks_open(&mut api, &path);
    add_track(&mut api, &path);

    block(&path);
    assert!(refused(api.dispatch(Request::ProjectClose {
        path: path.clone(),
        save: true,
    })));
    assert_eq!(
        tracks_open(&mut api, &path),
        before + 1,
        "still open, edit and all"
    );
    assert_eq!(api.open.holder(&path), Some(Holder::Api), "still held");

    unblock(&path);
    answered(api.dispatch(Request::ProjectClose {
        path: path.clone(),
        save: true,
    }));
    assert_eq!(api.open.holder(&path), None, "let go once saved");
    assert!(refused(
        api.dispatch(Request::ProjectGet { path: path.clone() })
    ));

    answered(api.dispatch(Request::ProjectOpen { path: path.clone() }));
    assert_eq!(
        tracks_open(&mut api, &path),
        before + 1,
        "the edit reached the disk"
    );
}

#[test]
fn a_close_without_saving_drops_the_edit_and_lets_the_folder_go() {
    let (mut api, scratch) = scratch_api();
    let path = new_project(&mut api, &scratch, "Dropped");
    let before = tracks_open(&mut api, &path);
    answered(api.dispatch(Request::ProjectSave {
        path: path.clone(),
        name: None,
    }));
    let on_disk = std::fs::read(projects::manifest_path(Path::new(&path))).expect("saved");
    add_track(&mut api, &path);

    answered(api.dispatch(Request::ProjectClose {
        path: path.clone(),
        save: false,
    }));
    assert_eq!(api.open.holder(&path), None);
    assert!(
        !Path::new(&path).join(lock::FILE).exists(),
        "the lock file goes with the session"
    );
    assert_eq!(
        std::fs::read(projects::manifest_path(Path::new(&path))).expect("reads"),
        on_disk,
        "nothing was written"
    );

    answered(api.dispatch(Request::ProjectOpen { path: path.clone() }));
    assert_eq!(tracks_open(&mut api, &path), before);
}
