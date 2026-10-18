// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The VU meters' feed: playback's peaks, thirty times a second, onto the
//! Meters global, with the ballistics a meter needs - up at once, down at
//! a steady rate, the loudest of the last moment held as a line, and the
//! clip light latched until it is clicked.
//!
//! The timer runs only while something plays and the meters are on
//! screen, and for the moment after it takes the bars to fall; it stops
//! itself once they are down. Put away - the tray's meter button, a dock
//! with no timeline in it, the phone shell, which has none - they cost
//! nothing: no timer, and no property changing thirty times a second.

use slint::ComponentHandle;

use crate::studio::Studio;
use crate::ui::{Meters, PaneKind};

/// The meters' floor in dB: their 0. Their 1 is 0 dB.
const FLOOR_DB: f32 = -60.0;
/// How fast a bar falls, in dB a second.
const FALL_DB_PER_SECOND: f32 = 24.0;
/// How long the held peak stays before it starts to fall.
const HOLD_SECONDS: f32 = 1.5;
const TICK: std::time::Duration = std::time::Duration::from_millis(33);

/// A linear sample level as the meters' `0..1` decibel scale.
pub fn scale(level: f32) -> f32 {
    if level <= 0.0 {
        return 0.0;
    }
    let db = 20.0 * level.log10();
    ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
}

/// One side's bar and its held peak, on the meters' scale.
#[derive(Default, Clone, Copy)]
struct Side {
    shown: f32,
    held: f32,
    held_for: f32,
}

impl Side {
    /// One tick: `level` is the loudest sample since the last.
    fn tick(&mut self, level: f32, seconds: f32) {
        let fall = FALL_DB_PER_SECOND * seconds / -FLOOR_DB;
        let now = scale(level);
        self.shown = now.max(self.shown - fall).max(0.0);
        if now >= self.held {
            self.held = now;
            self.held_for = 0.0;
        } else {
            self.held_for += seconds;
            if self.held_for > HOLD_SECONDS {
                self.held = (self.held - fall).max(self.shown);
            }
        }
    }

    fn quiet(&self) -> bool {
        self.shown <= 0.0 && self.held <= 0.0
    }
}

/// The meters' state, kept by the studio.
#[derive(Default)]
pub struct MeterFeed {
    left: Side,
    right: Side,
    pub clipped: bool,
    /// The window is a phone's, whose shell has no meters to feed.
    pub phone: bool,
    timer: slint::Timer,
}

impl Studio {
    /// Whether anything can see the meters: the tray's switch is on, and
    /// a timeline - whose right edge they sit on - is in the dock.
    pub fn meters_shown(&self) -> bool {
        !self.meters.phone && self.prefs.meters_shown() && self.dock.contains(PaneKind::Timeline)
    }

    fn rolling(&self) -> bool {
        self.playing || self.source.as_ref().is_some_and(|source| source.playing)
    }

    /// Something started playing, or the meters came into view while it
    /// plays: they follow it. Nothing happens when nobody can see them.
    pub fn wake_meters(&self) {
        if self.meters.timer.running() || !self.rolling() || !self.meters_shown() {
            return;
        }
        // The peaks playback kept while nobody was reading them belong to
        // some earlier moment; the first tick is to show this one.
        let _ = self.host.playback.take_levels();
        self.meters
            .timer
            .start(slint::TimerMode::Repeated, TICK, || {
                crate::host::Shell::with(|shell, app| {
                    let mut studio = shell.studio.borrow_mut();
                    let rolling = studio.rolling();
                    let shown = studio.meters_shown();
                    let (left, right) = studio.host.playback.take_levels();
                    let feed = &mut studio.meters;
                    if !shown {
                        // Put away mid-play: the bars drop to the floor now,
                        // so they are not found standing where they were when
                        // the meters are shown again, and the timer stops.
                        feed.left = Side::default();
                        feed.right = Side::default();
                        feed.publish(&app);
                        feed.timer.stop();
                        return;
                    }
                    feed.left.tick(left, TICK.as_secs_f32());
                    feed.right.tick(right, TICK.as_secs_f32());
                    feed.clipped |= left >= 1.0 || right >= 1.0;
                    feed.publish(&app);
                    if !rolling && feed.left.quiet() && feed.right.quiet() {
                        feed.timer.stop();
                    }
                });
            });
    }
}

impl MeterFeed {
    /// The clip light was clicked.
    pub fn clear_clip(&mut self, app: &crate::ui::App) {
        self.clipped = false;
        self.publish(app);
    }

    fn publish(&self, app: &crate::ui::App) {
        let meters = app.global::<Meters>();
        meters.set_left(self.left.shown);
        meters.set_right(self.right.shown);
        let hold = |side: &Side| if side.held > 0.0 { side.held } else { -1.0 };
        meters.set_hold_left(hold(&self.left));
        meters.set_hold_right(hold(&self.right));
        meters.set_clipped(self.clipped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_is_the_top_and_the_floor_is_the_foot() {
        assert_eq!(scale(1.0), 1.0);
        assert_eq!(scale(0.0), 0.0);
        assert!((scale(0.5) - (1.0 - 6.0206 / 60.0)).abs() < 1e-3, "-6 dB");
        assert_eq!(scale(0.0001), 0.0, "below -60 dB sits on the floor");
        assert_eq!(scale(2.0), 1.0, "a clipped mix stops at the top");
    }

    #[test]
    fn a_bar_rises_at_once_and_falls_steadily_while_its_peak_holds() {
        let mut side = Side::default();
        side.tick(1.0, 0.033);
        assert_eq!(side.shown, 1.0);
        side.tick(0.0, 0.5);
        // Twenty-four dB a second for half a second is twelve dB down.
        assert!((side.shown - (1.0 - 12.0 / 60.0)).abs() < 1e-4);
        assert_eq!(side.held, 1.0, "the peak holds");
        side.tick(0.0, 1.1);
        assert!(side.held < 1.0, "past the hold it falls too");
    }
}
