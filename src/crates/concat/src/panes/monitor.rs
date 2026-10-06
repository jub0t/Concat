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

use crate::host::{spawn_detached, spawn_frame};
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
    /// When the request out now was sent.
    asked: Option<std::time::Instant>,
    /// How the frames of the current playback arrived.
    cadence: Cadence,
}

/// How smoothly a playback's frames reached the monitor: one line in the
/// log when it stops, so a stutter is a number and not an impression.
#[derive(Default, Debug, PartialEq)]
struct Cadence {
    shown: u32,
    /// Frames showing the same timeline frame as the one before.
    repeated: u32,
    /// Timeline frames that passed without ever being shown.
    skipped: u32,
    /// The longest a request waited for its frame.
    worst: std::time::Duration,
    last: Option<i64>,
}

impl Cadence {
    /// Counts a frame of the timeline frame `index` that took `wait`.
    fn count(&mut self, index: i64, wait: std::time::Duration) {
        self.shown += 1;
        self.worst = self.worst.max(wait);
        match self.last {
            Some(last) if index == last => self.repeated += 1,
            Some(last) if index > last + 1 => {
                self.skipped += u32::try_from(index - last - 1).unwrap_or(u32::MAX);
            }
            _ => {}
        }
        self.last = Some(index);
    }

    /// Logs what was counted and starts over.
    fn report(&mut self, fps: f64) {
        if self.shown > 0 {
            log::info!(
                "playback: {} frames shown at {fps:.3} fps, {} repeated, {} skipped, \
                 worst wait {} ms",
                self.shown,
                self.repeated,
                self.skipped,
                self.worst.as_millis(),
            );
        }
        *self = Self::default();
    }
}

impl MonitorPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: MonitorMsg, studio: &mut Studio) {
        match msg {
            MonitorMsg::Request => self.request(studio),
            MonitorMsg::Frame(result, spec) => {
                self.busy = false;
                let fps = studio.project().active().video.rate();
                let wait = self.asked.take().map(|at| at.elapsed()).unwrap_or_default();
                if spec.moving && result.is_ok() {
                    self.cadence
                        .count((spec.time * fps + 1e-6).floor() as i64, wait);
                } else if !spec.moving {
                    self.cadence.report(fps);
                }
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
                            studio.notify(&tf("monitor.previewFailed", &[&error]), true);
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
        let quality = self.quality_of(studio.project());
        let spec = FrameSpec {
            time: f64::from(studio.preview_time()),
            width,
            height,
            moving: studio.playing || studio.source.as_ref().is_some_and(|source| source.playing),
            // Playback reads the proxies at Half and Quarter; Full plays
            // the files themselves, or Full would be the proxy blurred up.
            proxy: (studio.playing || studio.source.as_ref().is_some_and(|source| source.playing))
                && quality > 0,
            color_space: studio.project().active().video.color_space,
        };
        if !studio.playing {
            self.cadence.report(studio.project().active().video.rate());
        }
        let monitor = studio.host.monitor.clone();
        self.busy = true;
        self.wanted = false;
        self.asked = Some(std::time::Instant::now());
        spawn_frame(
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
                // it and the next frame can start meanwhile. Playing, the
                // full window ahead: the scheduler adds only the frames it
                // has not queued yet, so asking for all of them each frame
                // costs nothing and keeps a slow decode from catching up
                // with the playhead.
                {
                    let monitor = monitor.clone();
                    let settings = settings.clone();
                    let ahead = if spec.moving {
                        concat_media::prefetch::AHEAD
                    } else {
                        2
                    };
                    spawn_detached(move || monitor.prefetch(clips, &settings, spec, ahead));
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
    fn a_cadence_counts_repeats_and_gaps() {
        let ms = std::time::Duration::from_millis;
        let mut cadence = Cadence::default();
        for (index, wait) in [(0, 10), (1, 12), (1, 40), (4, 8)] {
            cadence.count(index, ms(wait));
        }
        assert_eq!(cadence.shown, 4);
        assert_eq!(cadence.repeated, 1);
        assert_eq!(cadence.skipped, 2, "frames 2 and 3 never came");
        assert_eq!(cadence.worst, ms(40));
        cadence.report(30.0);
        assert_eq!(cadence, Cadence::default(), "a report starts over");
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
