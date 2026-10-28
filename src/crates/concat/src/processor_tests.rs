// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Regression tests for inspector switches and audio treatments.

use slint::ComponentHandle;

fn test_studio(path: &std::path::Path) -> super::Studio {
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
    let mut studio = super::Studio::new(host);
    studio.session = Some(concat_host::Session::open(path.to_str().unwrap(), settings).unwrap());
    studio.selection = vec!["first".into()];
    studio
}

#[test]
fn adding_normalization_in_the_inspector_does_not_navigate() {
    i_slint_backend_testing::init_no_event_loop();
    let dir = tempfile::tempdir().unwrap();
    let mut studio = test_studio(dir.path());
    studio.inspector_jump = (7, "Video", "Enhance");
    studio.chain_add(true, "concat.loudness");
    assert_eq!(
        studio.clip("first").unwrap().filters[0].id,
        "concat.loudness"
    );
    assert_eq!(studio.inspector_jump, (7, "Video", "Enhance"));
    studio.chain_set_param(true, 0, "target", -16.0);
    studio.clip_commit();
    studio.commit_now();
    studio.chain_toggle(true, 0);
    let (rows, _) = super::chain_rows(&studio.clip("first").unwrap().filters);
    assert!(!rows[0].on);
    assert_eq!(
        studio.clip("first").unwrap().filters[0].params["target"],
        -16.0
    );
    studio.chain_toggle(true, 0);
    assert!(studio.clip("first").unwrap().filters[0].enabled);
    assert_eq!(studio.clip("first").unwrap().filters.len(), 1);
    assert_eq!(studio.inspector_jump, (7, "Video", "Enhance"));
    studio.selection = vec!["second".into()];
    studio.chain_add(true, "concat.loudness");
    assert_eq!(studio.inspector_jump, (7, "Video", "Enhance"));
    studio.undo();
    assert!(studio.clip("second").unwrap().filters.is_empty());
    assert_eq!(studio.clip("first").unwrap().filters.len(), 1);
    studio.redo();
    assert_eq!(studio.clip("second").unwrap().filters.len(), 1);
    // Library cards still reveal the effect they add.
    studio.apply_catalogue("concat.denoise", false);
    assert_eq!(studio.inspector_jump, (8, "Audio", "AudioEffects"));
}

#[allow(missing_docs)]
mod ui {
    slint::slint! {
        import { SwitchRow } from "ui/inspector/controls.slint";
        import { ProcessorRows, AppliedEntryData } from "ui/inspector/clip-inspector.slint";
        export component Treatments inherits Window {
            in property <[AppliedEntryData]> applied;
            callback added(string);
            callback toggled(int);
            public pure function is-on(id: string) -> bool { return rows.is-on(id); }
            public function toggle(id: string) { rows.toggle(id); }
            rows := ProcessorRows {
                applied: root.applied;
                add(id) => { root.added(id); }
                toggle-entry(index) => { root.toggled(index); }
            }
        }
        export component BoundSwitch inherits Window {
            width: 320px;
            height: 40px;
            in-out property <bool> selected-on;
            out property <bool> shown-on: row.checked;
            row := SwitchRow {
                label: "Normalize loudness";
                checked: root.selected-on;
                toggled(value) => { root.selected-on = value; }
            }
        }
    }
}

#[test]
fn treatment_switches_read_enabled_state_and_reuse_bypassed_entries() {
    use std::{cell::Cell, rc::Rc};
    i_slint_backend_testing::init_no_event_loop();
    let window = ui::Treatments::new().unwrap();
    let toggled = Rc::new(Cell::new(-1));
    window.on_toggled({
        let toggled = toggled.clone();
        move |index| toggled.set(index)
    });
    let added = Rc::new(Cell::new(0));
    window.on_added({
        let added = added.clone();
        move |_| added.set(added.get() + 1)
    });
    for id in ["concat.loudness", "concat.denoise", "concat.enhance-voice"] {
        assert!(!window.invoke_is_on(id.into()));
        window.invoke_toggle(id.into());
        assert_eq!(added.get(), 1);
        for on in [false, true, false] {
            window.set_applied(slint::ModelRc::new(slint::VecModel::from(vec![
                ui::AppliedEntryData {
                    id: "concat.other".into(),
                    on: true,
                    ..Default::default()
                },
                ui::AppliedEntryData {
                    id: id.into(),
                    on,
                    ..Default::default()
                },
            ])));
            assert_eq!(window.invoke_is_on(id.into()), on);
            window.invoke_toggle(id.into());
            assert_eq!(toggled.replace(-1), 1);
            assert_eq!(
                added.get(),
                1,
                "a bypassed treatment must not be duplicated"
            );
        }
        window.set_applied(Default::default());
        added.set(0);
    }
}

#[test]
fn normalization_switch_follows_selection_and_undo_after_a_click() {
    i_slint_backend_testing::init_no_event_loop();
    let window = ui::BoundSwitch::new().unwrap();
    window.show().unwrap();
    assert!(!window.get_shown_on());
    for event in [
        slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(290.0, 20.0),
        },
        slint::platform::WindowEvent::PointerPressed {
            position: slint::LogicalPosition::new(290.0, 20.0),
            button: slint::platform::PointerEventButton::Left,
        },
        slint::platform::WindowEvent::PointerReleased {
            position: slint::LogicalPosition::new(290.0, 20.0),
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        window.window().dispatch_event(event);
    }
    assert!(
        window.get_selected_on(),
        "the click must enable normalization"
    );
    assert!(window.get_shown_on());
    // Selecting an untreated clip, or undoing the treatment, publishes false.
    window.set_selected_on(false);
    assert!(!window.get_shown_on());
    // Returning to the normalized clip, or redoing, publishes true.
    window.set_selected_on(true);
    assert!(window.get_shown_on());
}
