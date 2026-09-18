// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The timeline's view: where it is looking, how close, with which tool,
//! and what the lanes know about a track that the document does not.
//!
//! The pane owns the scroll, the zoom, the tool, the snap and pan
//! switches, and each lane's lock and height. It also knows how wide the
//! lanes are on screen, which Slint reports as they change, and from
//! that it says which clips are worth publishing: the ones that intersect
//! the visible window plus one screen either side, so a long cut costs
//! the window what is on screen and not what is in the document.
//!
//! The gestures - a clip dragged, trimmed or razored, a picture moved on
//! the stage - stay with the controller: one gesture spans the lanes and
//! the stage, and an echo of the document, and moves next.

use std::collections::HashMap;

use crate::studio::Studio;
use crate::ui::{TimelineTool, TrackSize};

/// The closest the timeline zooms: half a millisecond a pixel.
pub const ZOOM_IN_LIMIT: f32 = 0.000_5;
/// The farthest: a second and a half a pixel.
pub const ZOOM_OUT_LIMIT: f32 = 1.5;
/// One step of the menu's zoom, as a factor.
const ZOOM_STEP: f32 = 1.4;

/// What the window knows about a lane that the document does not: whether
/// it is locked, and how tall to draw it.
#[derive(Clone, Copy)]
pub struct LaneView {
    pub locked: bool,
    pub size: TrackSize,
}

impl Default for LaneView {
    fn default() -> Self {
        Self {
            locked: false,
            size: TrackSize::Auto,
        }
    }
}

/// Everything that can happen to the view.
#[derive(Clone, Debug)]
pub enum TimelineMsg {
    ToolChanged(TimelineTool),
    PanChanged(bool),
    SnapChanged(bool),
    /// The menu's Snap row: the other way round.
    SnapToggled,
    /// The lanes scrolled sideways, to this many seconds from the start.
    Scrolled(f32),
    /// The wheel or a pinch: by a factor, about an instant; an anchor
    /// below zero keeps the left edge where it is.
    Zoomed { factor: f32, anchor: f32 },
    /// The whole cut across this many pixels.
    ZoomToFit(f32),
    ZoomIn,
    ZoomOut,
    /// The lanes are this wide on screen now, in logical pixels.
    Resized(f32),
    /// A project opened: the view starts at the beginning, nothing locked.
    Reset,
}

/// The timeline's view.
pub struct TimelinePane {
    /// Seconds from the start of the cut at the lanes' left edge.
    pub scroll_left: f32,
    pub seconds_per_pixel: f32,
    pub tool: TimelineTool,
    pub snap: bool,
    /// Hand tool: wheel and drag pan time instead of scrolling the stack.
    pub pan_mode: bool,
    pub lane_view: HashMap<String, LaneView>,
    /// The lanes' width on screen, in logical pixels; zero until Slint has
    /// said, and then every clip is published.
    pub width: f32,
}

impl Default for TimelinePane {
    fn default() -> Self {
        Self {
            scroll_left: 0.0,
            seconds_per_pixel: 0.05,
            tool: TimelineTool::Select,
            snap: true,
            pan_mode: false,
            lane_view: HashMap::new(),
            width: 0.0,
        }
    }
}

impl TimelinePane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: TimelineMsg, studio: &mut Studio) {
        match msg {
            TimelineMsg::ToolChanged(tool) => self.tool = tool,
            TimelineMsg::PanChanged(on) => self.pan_mode = on,
            TimelineMsg::SnapChanged(on) => self.snap = on,
            TimelineMsg::SnapToggled => self.snap = !self.snap,
            TimelineMsg::Scrolled(seconds) => self.scroll_left = seconds.max(0.0),
            TimelineMsg::Zoomed { factor, anchor } => self.zoom(factor, anchor),
            TimelineMsg::ZoomToFit(width) => {
                let span = studio.duration().max(1.0) * 1.05;
                if width > 1.0 {
                    self.seconds_per_pixel = (span / width).clamp(ZOOM_IN_LIMIT, ZOOM_OUT_LIMIT);
                    self.scroll_left = 0.0;
                }
            }
            TimelineMsg::ZoomIn => self.zoom(1.0 / ZOOM_STEP, -1.0),
            TimelineMsg::ZoomOut => self.zoom(ZOOM_STEP, -1.0),
            TimelineMsg::Resized(width) => self.width = width.max(0.0),
            TimelineMsg::Reset => {
                self.scroll_left = 0.0;
                self.lane_view.clear();
            }
        }
    }

    /// Zooms by a factor about an instant, so the instant under the pointer
    /// stays under it; an anchor below zero holds the left edge instead.
    fn zoom(&mut self, factor: f32, anchor: f32) {
        let before = self.seconds_per_pixel;
        if !(factor.is_finite() && factor > 0.0) {
            return;
        }
        let after = (before * factor).clamp(ZOOM_IN_LIMIT, ZOOM_OUT_LIMIT);
        self.seconds_per_pixel = after;
        if anchor >= 0.0 {
            self.scroll_left =
                (anchor - (anchor - self.scroll_left) * (after / before)).max(0.0);
        }
    }

    /// The span of the cut worth publishing: what the lanes show, plus one
    /// screen either side so a scroll never waits on a publish. None until
    /// the lanes have said how wide they are, which means everything.
    pub fn published_span(&self) -> Option<(f32, f32)> {
        if self.width <= 0.0 {
            return None;
        }
        let screen = self.width * self.seconds_per_pixel;
        Some((self.scroll_left - screen, self.scroll_left + 2.0 * screen))
    }

    /// Whether a clip at `start` for `duration` seconds falls in a span;
    /// every clip does when there is no span.
    pub fn shows(span: Option<(f32, f32)>, start: f32, duration: f32) -> bool {
        match span {
            None => true,
            Some((from, to)) => start + duration.max(0.0) >= from && start <= to,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(width: f32, scroll_left: f32, seconds_per_pixel: f32) -> TimelinePane {
        TimelinePane {
            width,
            scroll_left,
            seconds_per_pixel,
            ..TimelinePane::default()
        }
    }

    #[test]
    fn the_span_is_a_screen_either_side_of_the_view() {
        // 1000 px at 0.05 s/px is a 50 s screen, looking at 100..150.
        let span = pane(1000.0, 100.0, 0.05).published_span().expect("a span");
        assert_eq!(span, (50.0, 250.0));
        // Before the lanes have a width, everything is published.
        assert!(pane(0.0, 100.0, 0.05).published_span().is_none());
        assert!(pane(-5.0, 100.0, 0.05).published_span().is_none());
    }

    #[test]
    fn a_clip_is_shown_when_any_of_it_is_in_the_span() {
        let span = Some((50.0, 250.0));
        assert!(TimelinePane::shows(span, 40.0, 10.0), "ends on the edge");
        assert!(TimelinePane::shows(span, 250.0, 5.0), "starts on the edge");
        assert!(TimelinePane::shows(span, 0.0, 1000.0), "covers the span");
        assert!(!TimelinePane::shows(span, 0.0, 9.0));
        assert!(!TimelinePane::shows(span, 251.0, 100.0));
        // A negative duration is no duration, not a clip that reaches back.
        assert!(!TimelinePane::shows(span, 40.0, -100.0));
        assert!(TimelinePane::shows(None, 1e9, 0.0), "no span, everything");
    }

    #[test]
    fn zoom_keeps_the_anchor_under_the_pointer_and_stays_in_range() {
        let mut view = pane(1000.0, 10.0, 0.1);
        // The instant 20 s is at pixel 100; zooming in about it keeps it there.
        view.zoom(0.5, 20.0);
        assert!((view.seconds_per_pixel - 0.05).abs() < 1e-6);
        assert!((view.scroll_left - 15.0).abs() < 1e-6);
        assert!(((20.0 - view.scroll_left) / view.seconds_per_pixel - 100.0).abs() < 1e-3);
        // The limits hold, and a scroll never goes negative.
        view.zoom(1e9, -1.0);
        assert_eq!(view.seconds_per_pixel, ZOOM_OUT_LIMIT);
        view.zoom(1e-9, 0.0);
        assert_eq!(view.seconds_per_pixel, ZOOM_IN_LIMIT);
        assert!(view.scroll_left >= 0.0);
        // Nonsense factors change nothing.
        let before = view.seconds_per_pixel;
        view.zoom(f32::NAN, 0.0);
        view.zoom(0.0, 0.0);
        view.zoom(-2.0, 0.0);
        assert_eq!(view.seconds_per_pixel, before);
    }
}
