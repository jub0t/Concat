// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The words around merging clips into one rendered clip: the name the
//! edit goes by in the log and history, and the English lines the window
//! shows while a merge starts, runs, finishes, fails or is cancelled.

use concat_project::Command;
use concat_project::commands::NewMedia;
use concat_project::model::{MediaKind, MediaOrigin};

use super::{command_kind, command_words};

const ENGLISH: &str = include_str!("../../locales/en.json");

fn merge_of(path: &str) -> Command {
    Command::ConsolidateClips {
        clip_ids: vec!["c1".to_owned(), "c2".to_owned()],
        item: NewMedia {
            path: path.to_owned(),
            name: "Merged clips".to_owned(),
            duration: Some(4.0),
            kind: MediaKind::Video,
            width: Some(1920),
            height: Some(1080),
            frame_rate: Some(30.0),
            frame_rate_fraction: Some("30/1".to_owned()),
            video_codec: Some("h264".to_owned()),
            audio_codec: Some("aac".to_owned()),
            has_audio: true,
            audio_tracks: Vec::new(),
            origin: Some(MediaOrigin::Processed),
            color_space: Default::default(),
        },
    }
}

#[test]
fn the_edit_is_named_for_what_it_does_and_the_file_it_made() {
    let command = merge_of("/projects/Talk/media/merged-17/merged.mp4");
    assert_eq!(command_kind(&command), "Merge into one clip");
    assert_eq!(
        command_words(&command),
        "Merge into one clip, now merged.mp4"
    );
}

#[test]
fn every_line_a_merge_shows_is_in_english() {
    let english: serde_json::Value = serde_json::from_str(ENGLISH).expect("en.json reads");
    let line = |key: &str| -> String {
        english
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("{key} is missing from en.json"))
            .to_owned()
    };
    for (key, words) in [
        ("studio.mergeSelected", "Merge selected clips"),
        ("studio.mergeCancelProgress", "Cancel merging ({0}%)"),
        (
            "studio.mergeStarted",
            "Merging selected clips. Click Merge again to cancel.",
        ),
        (
            "studio.mergeCancelled",
            "Merging cancelled. The original clips are unchanged.",
        ),
        (
            "studio.mergeFinished",
            "Selected clips merged. Undo restores the original clips.",
        ),
        ("studio.mergeFailed", "Could not merge clips: {0}"),
        (
            "studio.mergeCouldNotReplace",
            "Could not replace the selected clips.",
        ),
        (
            "studio.mergeExportBusy",
            "Wait for the current export before merging clips.",
        ),
        (
            "studio.mergeTrackLocked",
            "Unlock the track before merging clips.",
        ),
        ("studio.mergedClips", "Merged clips"),
    ] {
        assert_eq!(line(key), words, "{key}");
    }
}
