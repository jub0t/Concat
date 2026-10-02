// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The window, one pane at a time.
//!
//! Each pane owns its state and is changed only by its own messages: a
//! Slint callback and a worker's report are the same thing, one [`Msg`]
//! posted to [`crate::studio::Studio::handle`], which routes it to the pane
//! and publishes. A worker's result for a project that has since closed
//! is dropped in one place, `host::deliver`, by the project epoch the work
//! was started in (`host::spawn_in_project`), rather than guarded against
//! in every closure.
//!
//! The panes move here one at a time from the window's controller; the
//! export sheet is the first, and the shape the rest follow.

pub mod captions;
pub mod export;
pub mod media_bin;
pub mod monitor;
pub mod project;
pub mod relink;
pub mod settings;
pub mod speech;
pub mod start;
pub mod timeline;

/// One thing that happened, to one pane.
#[derive(Debug)]
pub enum Msg {
    /// To the export sheet.
    Export(export::ExportMsg),
    /// To the settings sheet.
    Settings(settings::SettingsMsg),
    /// To the captions sheet.
    Captions(captions::CaptionsMsg),
    /// To the speech sheet.
    Speech(speech::SpeechMsg),
    /// To the missing media dialog.
    Relink(relink::RelinkMsg),
    /// To the project sheet.
    Project(project::ProjectMsg),
    /// To the launch screen's form.
    Start(start::StartMsg),
    /// To the media bin.
    Media(media_bin::MediaMsg),
    /// To the monitor.
    Monitor(monitor::MonitorMsg),
    /// To the timeline's view.
    Timeline(timeline::TimelineMsg),
}

/// What a message says in the log: the activity trail of what the person
/// did - a sheet opened, a switch flipped, an import, an export begun and
/// ended - for whoever is chasing a bug with the log in hand. None for
/// the messages nobody wants: every preview frame, every pointer move,
/// every progress tick. Typing is said by name and never quoted - a
/// project's name, a server's token - and an outcome by its path or its
/// count rather than its data.
pub fn activity(msg: &Msg) -> Option<(log::Level, String)> {
    use captions::CaptionsMsg;
    use export::ExportMsg;
    use log::Level::{Debug, Info};
    use media_bin::MediaMsg;
    use monitor::MonitorMsg;
    use relink::RelinkMsg;
    use settings::SettingsMsg;
    use speech::SpeechMsg;
    use timeline::TimelineMsg;

    match msg {
        Msg::Monitor(MonitorMsg::Frame(..) | MonitorMsg::Request | MonitorMsg::ScopePoll)
        | Msg::Timeline(
            TimelineMsg::Hovered(_)
            | TimelineMsg::HoverEnded
            | TimelineMsg::Scrolled(_)
            | TimelineMsg::Resized(_),
        )
        | Msg::Export(ExportMsg::Progress { .. })
        | Msg::Captions(CaptionsMsg::Progress(_))
        | Msg::Speech(SpeechMsg::Progress(_))
        | Msg::Settings(SettingsMsg::ModelProgress { .. } | SettingsMsg::InstallProgress { .. }) => {
            return None;
        }
        Msg::Export(ExportMsg::Start) => return Some((Info, "export: started".to_owned())),
        Msg::Export(ExportMsg::Finished(Ok(path))) => {
            return Some((Info, format!("export: finished, {path}")));
        }
        Msg::Export(ExportMsg::Finished(Err(error))) => {
            return Some((Info, format!("export: failed, {error}")));
        }
        Msg::Captions(CaptionsMsg::Begin) => return Some((Info, "captions: started".to_owned())),
        Msg::Captions(CaptionsMsg::Finished(Ok(segments))) => {
            return Some((
                Info,
                format!("captions: finished, {} segments", segments.len()),
            ));
        }
        Msg::Captions(CaptionsMsg::Finished(Err(error))) => {
            return Some((Info, format!("captions: failed, {error}")));
        }
        Msg::Speech(SpeechMsg::Begin) => return Some((Info, "speech: started".to_owned())),
        Msg::Speech(SpeechMsg::Finished(result)) => {
            return Some((
                Info,
                match result.as_ref() {
                    Ok(summary) => format!("speech: finished, {}", summary.path),
                    Err(error) => format!("speech: failed, {error}"),
                },
            ));
        }
        Msg::Media(MediaMsg::Import(paths)) => {
            let names: Vec<String> = paths
                .iter()
                .map(|path| {
                    path.file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect();
            return Some((
                Info,
                format!(
                    "import: {} file(s): {}",
                    paths.len(),
                    cap(&names.join(", "), 200)
                ),
            ));
        }
        Msg::Media(MediaMsg::Imported(results)) => {
            let failed = results.iter().filter(|result| result.is_err()).count();
            return Some((
                Info,
                format!("import: {} added, {failed} failed", results.len() - failed),
            ));
        }
        Msg::Relink(RelinkMsg::Show(missing)) => {
            return Some((Info, format!("relink: {} file(s) missing", missing.len())));
        }
        _ => {}
    }
    let text = format!("{msg:?}");
    let (outer, body) = text
        .split_once('(')
        .map(|(outer, rest)| (outer, rest.strip_suffix(')').unwrap_or(rest)))
        .unwrap_or((text.as_str(), ""));
    let inner: String = body
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    let outer = outer.to_ascii_lowercase();
    // Typing: by name, never quoted. A name, a path, a token are nothing
    // the log needs, and a keystroke each is not worth a line at Normal.
    if inner.ends_with("Edited") || inner == "ServerTokenGenerated" {
        return Some((Debug, format!("{outer}: {inner}")));
    }
    // The view moving under the person, and a sheet's own housekeeping,
    // are for Debug; a choice made is for Normal.
    let level = match inner.as_str() {
        "Select" | "Band" | "Zoomed" | "ZoomIn" | "ZoomOut" | "ZoomToFit" | "PageChanged"
        | "Restore" | "Reset" | "Opened" | "Closed" | "SnapToggled" | "MagneticToggled"
        | "UpdatesFetched" => Debug,
        _ => Info,
    };
    Some((level, format!("{outer}: {}", cap(body, 160))))
}

/// `text`, or its first `max` characters and an ellipsis.
fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut cut: String = text.chars().take(max).collect();
        cut.push('…');
        cut
    }
}
