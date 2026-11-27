// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The words around merging clips into one rendered clip: the name the
//! edit goes by in the log and history, and the English lines the window
//! shows while a merge starts, runs, finishes, fails or is cancelled.

use concat_doc::commands::Command;
use concat_doc::model::{Media, MediaKind, MediaOrigin, Size};

use super::{command_kind, command_words};
use crate::doc_view;

const ENGLISH: &str = include_str!("../../locales/en.json");

fn merge_of(path: &str) -> Command {
    Command::ConsolidateClips {
        items: vec![
            doc_view::parse("c1").expect("an id"),
            doc_view::parse("c2").expect("an id"),
        ],
        file: Media {
            path: path.to_owned(),
            name: "Merged clips".to_owned(),
            kind: MediaKind::Video,
            length: Some(doc_view::len(4.0)),
            rate: Some(concat_doc::Rate::FPS_30),
            size: Size::new(1920, 1080),
            has_sound: true,
            video_codec: Some("h264".to_owned()),
            audio_codec: Some("aac".to_owned()),
            origin: Some(MediaOrigin::Processed),
            ..Media::default()
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
