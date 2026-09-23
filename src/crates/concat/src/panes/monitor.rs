// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The monitor: the frame at the playhead, one request at a time.
//!
//! The pane owns the picture, the busy and wanted flags that serialise
//! requests, the quality tier per timeline, and the worker that draws a
//! frame. What goes into the frame - the flattened clips with their
//! titles, a look being auditioned, a cutout being painted - is the
//! controller's to say, and the pane asks it for that list. A request
//! while one is out waits for it, and the newest wins; a frame that
//! could not be drawn says so once per project and then stops repeating
//! itself.
//!
//! The stage's gestures - pressing, dragging and releasing a picture on
//! the monitor - are not here: one [`crate::studio::Gesture`] spans the
//! stage and the lanes, so they live with the timeline's.

use std::collections::HashMap;
use std::sync::Arc;

use concat_host::preview::FrameSpec;
use concat_project::model::Project;

use crate::host::{spawn_detached, spawn_in_project};
use crate::i18n::tf;
use crate::panes::Msg;
use crate::studio::Studio;

/// Everything that can happen to the monitor.
pub enum MonitorMsg {
    /// The frame at the playhead is wanted.
    Request,
    /// The worker is done: a frame, or why not.
    Frame(Result<Picture, String>, FrameSpec),
    /// The quality picker: 0 full, 1 half, 2 quarter.
    QualityChanged(i32),
    /// A project opened: the monitor starts clean and may complain again.
    Opened,
    /// The project closed: nothing to show.
    Closed,
}

impl std::fmt::Debug for MonitorMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Request => write!(f, "Request"),
            Self::Frame(result, spec) => write!(
                f,
                "Frame({}, {spec:?})",
                match result {
                    Ok(Picture::Sources(_)) => "sources".to_owned(),
                    Ok(Picture::Pixels(_, w, h)) => format!("{w}x{h} pixels"),
                    Err(error) => format!("error {error:?}"),
                }
            ),
            Self::QualityChanged(index) => write!(f, "QualityChanged({index})"),
            Self::Opened => write!(f, "Opened"),
            Self::Closed => write!(f, "Closed"),
        }
    }
}

/// A monitor frame on its way to the window.
pub enum Picture {
    /// Decoded and placed, waiting to be drawn on the window's own device -
    /// which happens back on this thread, never on the worker that decoded
    /// it. See `Monitor::texture_of`.
    Sources(concat_export::PreviewSources),
    /// Raw RGBA, to be uploaded.
    Pixels(Vec<u8>, u32, u32),
}

/// The monitor's state.
#[derive(Default)]
pub struct MonitorPane {
    /// The last frame drawn.
    pub image: slint::Image,
    busy: bool,
    wanted: bool,
    /// Said once per project: a monitor that cannot decode says so, and
    /// then stops repeating itself.
    failed: bool,
    /// 0 Full, 1 Half, 2 Quarter of the output size, by timeline id. Kept
    /// per timeline because the cost it trades against is the timeline's
    /// frame - a 4K cut wants the quarter setting that a 1080p cut beside
    /// it does not - and the trade is the window's, not the document's,
    /// so it is remembered here and not saved.
    quality: HashMap<String, usize>,
}

impl MonitorPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: MonitorMsg, studio: &mut Studio) {
        match msg {
            MonitorMsg::Request => self.request(studio),
            MonitorMsg::Frame(result, spec) => {
                self.busy = false;
                let picture = match result {
                    // Drawn here and not on the worker: this is the event
                    // loop, the one thread the window's renderer submits
                    // from, and a second thread submitting beside it hangs
                    // the GPU - see `Monitor::texture_of`.
                    Ok(Picture::Sources(sources)) => studio
                        .host
                        .monitor
                        .texture_of(&sources, spec)
                        .and_then(|texture| {
                            slint::Image::try_from(texture)
                                .map_err(|error| format!("preview texture: {error}"))
                        }),
                    Ok(Picture::Pixels(bytes, width, height)) => {
                        let buffer =
                            slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                                &bytes, width, height,
                            );
                        Ok(slint::Image::from_rgba8(buffer))
                    }
                    Err(error) => Err(error),
                };
                match picture {
                    Ok(image) => self.image = image,
                    Err(error) => {
                        log::warn!("preview: {error}");
                        if !self.failed {
                            self.failed = true;
                            studio.notify(&tf("Preview failed: {0}", &[&error]), true);
                        }
                    }
                }
                if self.wanted {
                    self.request(studio);
                }
            }
            MonitorMsg::QualityChanged(index) => {
                let id = studio.project().active_timeline_id.clone();
                self.quality.insert(id, (index.max(0) as usize).min(2));
                self.request(studio);
            }
            MonitorMsg::Opened => self.failed = false,
            MonitorMsg::Closed => self.image = slint::Image::default(),
        }
    }

    /// The quality tier for the project's active timeline: 0 full, 1
    /// half, 2 quarter. Half until chosen.
    pub fn quality_of(&self, project: &Project) -> usize {
        self.quality
            .get(&project.active_timeline_id)
            .copied()
            .unwrap_or(1)
    }

    /// The frame's size at a quality tier: the output scaled, rounded to
    /// even dimensions, never smaller than two pixels a side.
    pub fn frame_size(quality: usize, (width, height): (u32, u32)) -> (u32, u32) {
        let scale = match quality {
            0 => 1.0,
            1 => 0.5,
            _ => 0.25,
        };
        let side = |px: u32| ((f64::from(px) * scale).round() as u32).max(2) & !1;
        (side(width), side(height))
    }

    /// Asks the engine for the frame at the playhead - or under the pointer,
    /// while the preview axis has it - one at a time.
    fn request(&mut self, studio: &mut Studio) {
        if studio.on_start || studio.session.is_none() {
            return;
        }
        if self.busy {
            self.wanted = true;
            return;
        }
        let (width, height) =
            Self::frame_size(self.quality_of(studio.project()), studio.output_size());
        let Some((clips, settings)) = studio.preview_clips() else {
            return;
        };
        let spec = FrameSpec {
            time: f64::from(studio.preview_time()),
            width,
            height,
            moving: studio.playing,
        };
        let monitor = studio.host.monitor.clone();
        self.busy = true;
        self.wanted = false;
        spawn_in_project(
            move || {
                // On the window's device the frame stays a texture; without
                // one it comes back as pixels and is uploaded here.
                let frame = if monitor.has_gpu() {
                    monitor
                        .frame_sources(Arc::clone(&clips), &settings, spec)
                        .map(Picture::Sources)
                } else {
                    monitor
                        .frame(Arc::clone(&clips), &settings, spec)
                        .map(|bytes| Picture::Pixels(bytes, width, height))
                };
                // Decode-ahead for whatever comes next, on a worker of its
                // own, so the frame goes to the window without waiting for
                // it and the next frame can start meanwhile.
                {
                    let monitor = monitor.clone();
                    let settings = settings.clone();
                    spawn_detached(move || monitor.prefetch(clips, &settings, spec, 2));
                }
                frame
            },
            move |studio, _, _, result| {
                studio.handle(Msg::Monitor(MonitorMsg::Frame(result, spec)));
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_even_sided_and_never_vanishes() {
        assert_eq!(MonitorPane::frame_size(0, (1920, 1080)), (1920, 1080));
        assert_eq!(MonitorPane::frame_size(1, (1920, 1080)), (960, 540));
        assert_eq!(MonitorPane::frame_size(2, (1920, 1080)), (480, 270));
        // An odd output at half is rounded to even, as the encoder and the
        // chroma planes need.
        assert_eq!(MonitorPane::frame_size(1, (1001, 1001)), (500, 500));
        // Nothing gets smaller than two pixels a side, whatever the tier.
        assert_eq!(MonitorPane::frame_size(2, (1, 1)), (2, 2));
        assert_eq!(MonitorPane::frame_size(2, (0, 0)), (2, 2));
        // A tier past the picker's three is the smallest one.
        assert_eq!(MonitorPane::frame_size(9, (400, 400)), (100, 100));
    }

    #[test]
    fn quality_is_half_until_chosen_and_kept_per_timeline() {
        let mut pane = MonitorPane::default();
        let mut project = Project {
            active_timeline_id: "a".to_owned(),
            ..Project::default()
        };
        assert_eq!(pane.quality_of(&project), 1);
        pane.quality.insert("a".to_owned(), 2);
        assert_eq!(pane.quality_of(&project), 2);
        project.active_timeline_id = "b".to_owned();
        assert_eq!(pane.quality_of(&project), 1, "another timeline has its own");
    }
}
