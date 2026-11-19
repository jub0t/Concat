// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! When a launch counts as started: the moment safe start's mark comes
//! off. See `safe_start` in lib.rs for what the mark is for.
//!
//! The mark measures one thing, a launch that never got its window
//! drawing: a driver that kills the process as the surface opens or on
//! the first frame (#600, #507, #754). It used to come off fifteen
//! seconds after the event loop started, on a timer the loop had to run.
//! A loop that froze before then - an export that hung as it began, and
//! Android ending the app as not responding - never ran it, and the next
//! launch read the hang as a failed start and drew the slow way for the
//! rest of that build.
//!
//! So it comes off on what it means: [`DRAWN_FOR`] after the window's
//! first frame, or at once when the person does anything in the window,
//! which they can only do to a window they can see. Fifteen seconds of a
//! running loop stays as a floor, for a window that never says it drew.
//! The frame and the person are told here by the backends' own hooks:
//! `DropHandler` on a desktop, the activity's event listener on Android.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

use concat_host::jobs::Tripwire;

/// How long the window runs after its first frame before the launch
/// counts as started. A driver that took the first frame and dies on what
/// it queued, or on the monitor's first picture, dies within it; nobody
/// opens a project and starts an export within it, and a person who
/// touches the window ends it early anyway.
pub(crate) const DRAWN_FOR: Duration = Duration::from_secs(3);

/// The floor: a launch whose loop has run this long counts as started,
/// whether or not a frame was ever reported.
pub(crate) const SAFE_AFTER: Duration = Duration::from_secs(15);

/// Safe start's mark, held from the moment it is armed until the launch
/// counts as started.
///
/// Its file is removed when the launch starts, and when the watch drops
/// on any orderly way out. Not on [`StartWatch::keep`], for a window that
/// could not run, and not when the watch drops while a panic unwinds
/// before the first frame: on Android a panic unwinds (the release build
/// is not `panic = "abort"`), and a launch that panicked before it ever
/// drew is a failed start as much as one a driver killed. A panic after
/// the first frame is a bug of the app's, not a reason to draw slowly.
pub(crate) struct StartWatch {
    mark: RefCell<Option<Tripwire>>,
    /// A frame has been reported, and the countdown to [`DRAWN_FOR`] is
    /// running.
    drawn: Cell<bool>,
}

impl StartWatch {
    pub(crate) fn new(mark: Tripwire) -> Self {
        Self {
            mark: RefCell::new(Some(mark)),
            drawn: Cell::new(false),
        }
    }

    /// The window drew a frame. True the first time, while the mark is
    /// still on: the countdown starts now.
    fn drew(&self) -> bool {
        !self.drawn.replace(true) && self.armed()
    }

    /// The launch has started: the mark comes off. True when this was
    /// the call that took it off.
    fn start(&self) -> bool {
        self.mark.borrow_mut().take().is_some()
    }

    /// The window could not run: the mark stays for the next launch.
    pub(crate) fn keep(&self) {
        if let Some(mark) = self.mark.borrow_mut().take() {
            std::mem::forget(mark);
        }
    }

    /// Whether the mark is still on.
    fn armed(&self) -> bool {
        self.mark.borrow().is_some()
    }
}

impl Drop for StartWatch {
    fn drop(&mut self) {
        if std::thread::panicking() && !self.drawn.get() {
            self.keep();
        }
    }
}

thread_local! {
    /// The launch's watch, for the backends' hooks. Weak: the launch owns
    /// it, and drops it as the loop ends.
    static WATCH: RefCell<Weak<StartWatch>> = const { RefCell::new(Weak::new()) };
}

/// Makes `watch` the one the hooks report to, and starts the floor's
/// timer. Called on the window's thread before the loop runs.
pub(crate) fn install(watch: &Rc<StartWatch>) {
    WATCH.with(|slot| *slot.borrow_mut() = Rc::downgrade(watch));
    let weak = Rc::downgrade(watch);
    slint::Timer::single_shot(SAFE_AFTER, move || {
        if let Some(watch) = weak.upgrade()
            && watch.start()
        {
            log::info!(
                "start: the window has run {}; a close from here on is not a failed start",
                concat_log::span(SAFE_AFTER)
            );
        }
    });
}

fn current() -> Option<Rc<StartWatch>> {
    WATCH.with(|slot| slot.borrow().upgrade())
}

/// The window drew, or is drawing, a frame: the launch counts as started
/// [`DRAWN_FOR`] later, if the loop is still running then. Cheap after the
/// first call; for the backends' hooks, on the window's thread.
pub fn window_drew() {
    let Some(watch) = current() else {
        return;
    };
    if !watch.drew() {
        return;
    }
    let weak = Rc::downgrade(&watch);
    slint::Timer::single_shot(DRAWN_FOR, move || {
        if let Some(watch) = weak.upgrade()
            && watch.start()
        {
            log::info!(
                "start: the window drew and ran {} more; a close from here on is not a failed start",
                concat_log::span(DRAWN_FOR)
            );
        }
    });
}

/// The person did something in the window - a press, a tap, a key: the
/// launch has started. For the backends' hooks, on the window's thread.
pub fn window_touched() {
    if let Some(watch) = current()
        && watch.start()
    {
        log::info!("start: the window was used; a close from here on is not a failed start");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of the test's own, removed when it ends.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "concat-start-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a scratch folder");
            Self(path)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn marked(dir: &std::path::Path) -> bool {
        dir.join("running").exists()
    }

    #[test]
    fn the_first_frame_starts_the_countdown_once_and_the_start_takes_the_mark_off() {
        let scratch = Scratch::new("drew");
        let watch = StartWatch::new(Tripwire::arm(&scratch.0));
        assert!(marked(&scratch.0));
        assert!(watch.drew(), "the first frame starts the countdown");
        assert!(!watch.drew(), "a second frame starts nothing more");
        assert!(
            marked(&scratch.0),
            "the mark stays until the countdown ends"
        );
        assert!(watch.start());
        assert!(!marked(&scratch.0));
        assert!(!Tripwire::tripped(&scratch.0));
        assert!(!watch.start(), "the mark comes off once");
    }

    #[test]
    fn the_person_touching_the_window_starts_it_before_any_countdown() {
        let scratch = Scratch::new("touched");
        let watch = StartWatch::new(Tripwire::arm(&scratch.0));
        assert!(watch.start());
        assert!(!marked(&scratch.0));
        assert!(!watch.drew(), "no countdown for a launch already started");
    }

    #[test]
    fn a_window_that_could_not_run_leaves_the_mark_for_the_next_launch() {
        let scratch = Scratch::new("keep");
        let watch = StartWatch::new(Tripwire::arm(&scratch.0));
        watch.keep();
        drop(watch);
        assert!(marked(&scratch.0));
        // The mark names this process, which reads as the launch that died.
        assert!(Tripwire::tripped(&scratch.0));
    }

    #[test]
    fn an_orderly_close_before_the_countdown_ends_takes_the_mark_off() {
        let scratch = Scratch::new("close");
        let watch = StartWatch::new(Tripwire::arm(&scratch.0));
        assert!(watch.drew());
        drop(watch);
        assert!(!marked(&scratch.0));
    }

    #[test]
    fn a_panic_before_the_first_frame_is_a_failed_start() {
        let scratch = Scratch::new("panic-early");
        let watch = StartWatch::new(Tripwire::arm(&scratch.0));
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _watch = watch;
            panic!("a start that panics before it draws");
        }));
        assert!(unwound.is_err());
        assert!(marked(&scratch.0), "the next launch starts safe");
    }

    #[test]
    fn a_panic_after_the_first_frame_is_not_a_failed_start() {
        let scratch = Scratch::new("panic-late");
        let watch = StartWatch::new(Tripwire::arm(&scratch.0));
        assert!(watch.drew());
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _watch = watch;
            panic!("a bug after the window drew");
        }));
        assert!(unwound.is_err());
        assert!(!marked(&scratch.0));
    }

    #[test]
    fn the_hooks_do_nothing_with_no_launch_watched() {
        // A smoke run, a phone's second activity, a test: no watch.
        window_drew();
        window_touched();
    }

    #[test]
    fn the_hooks_reach_the_installed_watch() {
        let scratch = Scratch::new("hooks");
        let watch = Rc::new(StartWatch::new(Tripwire::arm(&scratch.0)));
        WATCH.with(|slot| *slot.borrow_mut() = Rc::downgrade(&watch));
        window_touched();
        assert!(!marked(&scratch.0));
        assert!(!watch.drew(), "started already: no countdown");
        WATCH.with(|slot| *slot.borrow_mut() = Weak::new());
    }
}
