// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Finder documents become a new edit, rather than media in the current one.

use std::path::Path;

use concat_host::{media::MediaSummary, projects};
use concat_project::Command;

use crate::panes::{Msg, media_bin::MediaMsg};
use crate::studio::Studio;

#[cfg(target_os = "macos")]
thread_local! {
    static REQUESTS: std::cell::RefCell<(bool, std::collections::VecDeque<Vec<std::path::PathBuf>>)> = std::cell::RefCell::default();
}

#[cfg(target_os = "macos")]
pub(crate) fn enqueue(paths: Vec<std::path::PathBuf>) {
    if !paths.is_empty() {
        REQUESTS.with_borrow_mut(|(_, queue)| queue.push_back(paths));
    }
}

/// Probe outside AppKit's document callback and process batches in arrival
/// order. In particular a cold-launch event can wait here until Shell exists.
#[cfg(target_os = "macos")]
pub(crate) fn tick() {
    let next = REQUESTS.with_borrow_mut(|(busy, queue)| {
        if *busy {
            return None;
        }
        let paths = queue.pop_front()?;
        *busy = true;
        Some(paths)
    });
    let Some(paths) = next else { return };
    crate::host::spawn(
        move || {
            paths
                .iter()
                .map(|path| concat_host::media::probe(&path.to_string_lossy()))
                .collect::<Result<Vec<_>, _>>()
        },
        |studio, _, _, result| {
            if let Err(error) = result.and_then(|media| import(studio, media)) {
                studio.notify(&error, true);
            }
            REQUESTS.with_borrow_mut(|(busy, _)| *busy = false);
        },
    );
}

pub(crate) fn import(studio: &mut Studio, media: Vec<MediaSummary>) -> Result<(), String> {
    let first = media
        .first()
        .ok_or_else(|| crate::i18n::t("start.dropNothingToOpen"))?;
    let video = media
        .iter()
        .find_map(|item| item.video.as_ref())
        .ok_or_else(|| crate::i18n::t("start.dropNothingToOpen"))?;
    let base = projects::folder_name(
        Path::new(&first.path)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled project"),
    );
    let location = studio.start.location.trim();
    if location.is_empty() {
        return Err(crate::i18n::t("start.chooseWhereProjectFolder"));
    }
    let (num, den) = video
        .frame_rate_fraction
        .split_once('/')
        .and_then(|(num, den)| Some((num.parse::<i64>().ok()?, den.parse::<i64>().ok()?)))
        .filter(|&(num, den)| num > 0 && den > 0)
        .unwrap_or((30, 1));
    std::fs::create_dir_all(location).map_err(|error| format!("{location}: {error}"))?;
    // Reserve the folder atomically. Even an unrelated non-project directory
    // must survive, and two Finder requests must never share a destination.
    let (name, folder) = (1..)
        .find_map(|index| {
            let name = if index == 1 {
                base.clone()
            } else {
                format!("{base} ({index})")
            };
            let folder = Path::new(location).join(&name);
            match std::fs::create_dir(&folder) {
                Ok(()) => Some(Ok((name, folder))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(format!("{}: {error}", folder.display()))),
            }
        })
        .expect("an unbounded name sequence ends at a free folder or an IO error")?;
    let opened = projects::create(
        location,
        &name,
        video.width,
        video.height,
        num,
        den,
        [0, 0, 0, 255],
    )
    .and_then(|info| studio.open_project(info));
    if let Err(error) = opened {
        // Only the manifest and empty folder reserved above are ours. Never
        // recursively remove a directory someone might have added files to.
        let _ = std::fs::remove_file(folder.join("concat.json"));
        let _ = std::fs::remove_dir(&folder);
        return Err(error);
    }
    let count = media.len();
    studio.handle(Msg::Media(MediaMsg::Imported(
        media.into_iter().map(Ok).collect(),
    )));
    if studio.project().media.len() != count {
        return Err("Could not import the Finder documents".into());
    }
    let media_ids = studio
        .project()
        .media
        .iter()
        .map(|item| item.id.clone())
        .collect();
    studio.apply(Command::AddClipsInSequence {
        media_ids,
        track_id: None,
        start: 0.0,
        ripple: false,
    });
    studio.start.composing = false;
    studio.start.dropped.clear();
    // Persist the complete initial edit immediately; a Finder-opened project
    // must survive closing the window before the autosave timer first runs.
    if let Some(session) = studio.session.as_mut() {
        session.save(None)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "desktop_open_tests.rs"]
mod tests;
