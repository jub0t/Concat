// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

use concat_host::media::{MediaSummary, VideoStreamInfo};
use concat_project::model::{ColorSpace, MediaKind};

fn test_studio(path: &std::path::Path) -> crate::studio::Studio {
    use concat_host::{AppDirs, playback, preview};
    use std::sync::Arc;
    struct Events;
    impl playback::PlaybackEvents for Events {
        fn position(&self, _: f64) {}
        fn error(&self, _: String) {}
    }
    let dirs = AppDirs {
        config: path.join("config"),
        data: path.join("data"),
    };
    let host = crate::host::Host {
        titles: concat_host::Titles::new(&dirs),
        cutouts: Arc::new(concat_host::Cutouts::new(&dirs.data)),
        stabilizers: Arc::new(concat_host::Stabilizers::new()),
        brushes: Arc::new(concat_host::Brushes::new(&dirs.data)),
        enhancers: Arc::new(concat_host::Enhancers::new(&dirs.data)),
        reversers: Arc::new(concat_host::Reversers::new()),
        dirs,
        playback: playback::Playback::start(Arc::new(Events)).unwrap(),
        monitor: preview::Monitor::new(),
        exporter: concat_host::export::Exporter::new(),
        transcriber: Arc::new(concat_speech::Transcriber::new()),
        speech: Arc::new(concat_speech::Speech::new()),
        server: None,
        open_projects: concat_api::OpenProjects::default(),
    };
    let mut project = concat_project::Project::new();
    project.media.push(
        serde_json::from_value(serde_json::json!({
            "id": "m1", "path": path.join("video.mp4"), "name": "video.mp4",
            "duration": 5.0, "kind": "video", "hasAudio": true
        }))
        .unwrap(),
    );
    for id in ["first", "second"] {
        let mut clip = concat_project::model::Clip::blank(
            id,
            "T1",
            concat_project::model::ClipKind::Video,
            id,
            0.0,
            5.0,
        );
        clip.media_id = "m1".into();
        project.active_mut().clips.push(clip.into());
    }
    let settings = concat_project::DocumentSettings {
        name: "Test".into(),
        width: 1920,
        height: 1080,
        rate_num: 30,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
    };
    std::fs::write(
        path.join("concat.json"),
        concat_project::to_document(&settings, &project).to_string(),
    )
    .unwrap();
    let mut studio = crate::studio::Studio::new(host);
    studio.session = Some(concat_host::Session::open(path.to_str().unwrap(), settings).unwrap());
    studio.selection = vec!["first".into()];
    studio
}

fn video(path: &std::path::Path) -> MediaSummary {
    MediaSummary {
        path: path.to_string_lossy().into_owned(),
        duration: Some(2.0),
        kind: MediaKind::Video,
        video: Some(VideoStreamInfo {
            index: 0,
            codec: "h264".into(),
            width: 1080,
            height: 1920,
            frame_rate: 29.97,
            frame_rate_fraction: "30000/1001".into(),
            color_range: None,
            color_space: ColorSpace::default(),
        }),
        audio: None,
        audio_tracks: Vec::new(),
    }
}

#[test]
fn finder_import_creates_a_video_sized_project_with_sequential_clips() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let mut studio = test_studio(dir.path());
    studio.start.location = dir.path().join("projects").to_string_lossy().into_owned();
    crate::desktop_open::import(
        &mut studio,
        vec![
            video(&dir.path().join("Поездка.mp4")),
            video(&dir.path().join("two.mp4")),
        ],
    )
    .unwrap();
    assert_eq!(studio.project_name, "Поездка");
    let settings = studio.session.as_ref().unwrap().settings();
    assert_eq!((settings.width, settings.height), (1080, 1920));
    assert_eq!((settings.rate_num, settings.rate_den), (30000, 1001));
    assert_eq!(studio.project().media.len(), 2);
    let clips = &studio.timeline().clips;
    assert_eq!(clips.len(), 2);
    assert_eq!(clips[0].start, 0.0);
    assert!((clips[1].start - clips[0].duration).abs() < 0.001);
}

#[test]
fn finder_import_uses_a_free_folder_and_preserves_existing_contents() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let mut studio = test_studio(dir.path());
    studio.start.location = dir.path().join("projects").to_string_lossy().into_owned();
    let occupied = std::path::Path::new(&studio.start.location).join("video");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("keep.txt"), "keep").unwrap();
    crate::desktop_open::import(&mut studio, vec![video(&dir.path().join("video.mp4"))]).unwrap();
    assert_eq!(studio.project_name, "video (2)");
    assert_eq!(
        std::fs::read_to_string(occupied.join("keep.txt")).unwrap(),
        "keep"
    );
    assert!(!occupied.join("concat.json").exists());
}

#[test]
fn empty_finder_import_does_not_replace_the_open_edit() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let mut studio = test_studio(dir.path());
    let before = studio.session.as_ref().unwrap().path().to_owned();
    assert!(crate::desktop_open::import(&mut studio, Vec::new()).is_err());
    assert_eq!(studio.session.as_ref().unwrap().path(), before);
    assert_eq!(studio.timeline().clips.len(), 2);
}

#[test]
fn failed_save_keeps_the_current_project_when_finder_opens_a_video() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let mut studio = test_studio(dir.path());
    studio.start.location = dir.path().join("projects").to_string_lossy().into_owned();
    let manifest = dir.path().join("concat.json");
    std::fs::remove_file(&manifest).unwrap();
    std::fs::create_dir(&manifest).unwrap();
    let before = studio.session.as_ref().unwrap().path().to_owned();
    assert!(
        crate::desktop_open::import(&mut studio, vec![video(&dir.path().join("new.mp4"))]).is_err()
    );
    assert_eq!(studio.session.as_ref().unwrap().path(), before);
    assert_eq!(studio.timeline().clips.len(), 2);
}

#[cfg(target_os = "macos")]
#[test]
fn native_command_keys_reach_the_slint_focus_scope() {
    use slint::ComponentHandle;
    let called = std::rc::Rc::new(std::cell::Cell::new(false));
    i_slint_backend_testing::init_no_event_loop();
    let app = crate::ui::App::new().unwrap();
    app.global::<crate::ui::Editor>().on_key_chord({
        let called = called.clone();
        move |text, shift, alt, control, meta, _| {
            if text != "z" {
                return false;
            }
            assert_eq!(text, "z");
            assert!(control);
            assert!(!shift && !alt && !meta);
            called.set(true);
            true
        }
    });
    app.invoke_blur();
    crate::macos::forward_command(&app, "z", 1 << 20);
    assert!(called.get());
}
