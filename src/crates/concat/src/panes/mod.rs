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
pub mod voiceover;

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
    /// To the voiceover take.
    Voiceover(voiceover::VoiceoverMsg),
}

/// What a message says in the log: the activity trail of what the person
/// did - a sheet opened, a switch flipped, an import, an export begun and
/// ended - for whoever is chasing a bug with the log in hand. None for
/// the messages nobody wants: every preview frame, every pointer move,
/// every progress tick. Typing is said by name and never quoted - a
/// project's name, a server's token - and an outcome by its path or its
/// count rather than its data.
pub fn activity(msg: &Msg) -> Option<Activity> {
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
        Msg::Monitor(MonitorMsg::Frame(..) | MonitorMsg::Request)
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
        // A wheel tick that zooms by nothing - a trackpad's idle events -
        // did nothing, and says nothing.
        Msg::Timeline(TimelineMsg::Zoomed { factor, .. }) if (factor - 1.0).abs() < 1e-6 => {
            return None;
        }
        Msg::Export(ExportMsg::Start) => {
            return Some(Activity::once(Info, "export: started".to_owned()));
        }
        Msg::Export(ExportMsg::Finished(Ok(path))) => {
            return Some(Activity::once(Info, format!("export: finished, {path}")));
        }
        Msg::Export(ExportMsg::Finished(Err(error))) => {
            return Some(Activity::once(Info, format!("export: failed, {error}")));
        }
        Msg::Captions(CaptionsMsg::Begin) => {
            return Some(Activity::once(Info, "captions: started".to_owned()));
        }
        Msg::Captions(CaptionsMsg::Finished(Ok(segments))) => {
            return Some(Activity::once(
                Info,
                format!("captions: finished, {} segments", segments.len()),
            ));
        }
        Msg::Captions(CaptionsMsg::Finished(Err(error))) => {
            return Some(Activity::once(Info, format!("captions: failed, {error}")));
        }
        Msg::Speech(SpeechMsg::Begin) => {
            return Some(Activity::once(Info, "speech: started".to_owned()));
        }
        Msg::Speech(SpeechMsg::Finished(result)) => {
            return Some(Activity::once(
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
            return Some(Activity::once(
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
            return Some(Activity::once(
                Info,
                format!("import: {} added, {failed} failed", results.len() - failed),
            ));
        }
        Msg::Relink(RelinkMsg::Show(missing)) => {
            return Some(Activity::once(
                Info,
                format!("relink: {} file(s) missing", missing.len()),
            ));
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
        return Some(Activity {
            level: Debug,
            line: format!("{outer}: {inner}"),
            burst: Some(format!("{outer}: {inner}")),
        });
    }
    // The view moving under the person is a burst - a wheel's worth of
    // zooms is one line with a count, not a line a tick - and for Debug
    // with a sheet's own housekeeping; a choice made is for Normal.
    let continuous = matches!(
        inner.as_str(),
        "Select" | "Band" | "Zoomed" | "ZoomIn" | "ZoomOut" | "ZoomToFit"
    );
    let level = match inner.as_str() {
        "Select" | "Band" | "Zoomed" | "ZoomIn" | "ZoomOut" | "ZoomToFit" | "PageChanged"
        | "Restore" | "Reset" | "Opened" | "Closed" | "SnapToggled" | "MagneticToggled"
        | "UpdatesFetched" => Debug,
        _ => Info,
    };
    Some(Activity {
        level,
        line: format!("{outer}: {}", cap(body, 160)),
        burst: continuous.then(|| format!("{outer}: {inner}")),
    })
}

/// One line of the activity trail: its level, its words, and for a
/// message that comes in bursts - a wheel's zooms, a drag's selections,
/// a field's keystrokes - the key the burst is folded under, so a run of
/// them is one line and a count rather than a line a tick.
pub struct Activity {
    pub level: log::Level,
    pub line: String,
    pub burst: Option<String>,
}

impl Activity {
    fn once(level: log::Level, line: impl Into<String>) -> Activity {
        Activity {
            level,
            line: line.into(),
            burst: None,
        }
    }
}

/// A burst of one kind of message as it is being folded: its key, the
/// latest line, how many so far, when it began and when the last came.
pub struct Burst {
    pub key: String,
    pub line: String,
    pub count: u32,
    pub began: std::time::Instant,
    pub last: std::time::Instant,
}

impl Burst {
    /// How long after the last of a burst the next of its kind is a new
    /// burst rather than more of the same.
    pub const GAP: std::time::Duration = std::time::Duration::from_secs(2);

    /// The burst's line for the log: the first was said as it came; the
    /// rest are a count over the time they took.
    pub fn flush(&self) {
        if self.count > 1 {
            log::debug!(
                "{} (×{} over {:.1}s)",
                self.line,
                self.count,
                self.last.duration_since(self.began).as_secs_f64()
            );
        }
    }
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
