// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! One thread at a time on a software adapter.
//!
//! Direct3D 12 keeps one device an adapter in a process, so on Windows'
//! software renderer (WARP) every compositor - the previews', the export's
//! sibling of it, a card's opened only to learn what it is on - draws on
//! the one device underneath, each from a thread of its own. WARP did not
//! survive two threads in it at once: on CI's runner, which has no GPU,
//! the previews' first frame lost the device while the export's
//! compositor was building its pipelines on another thread, and every
//! device opened after that died on its first frame. The same build with
//! the debug layer on, which puts every call in a line, exported to the
//! end (#824).
//!
//! So on a software adapter every call into the device takes its turn:
//! opening a device, building a compositor, every frame drawn, submitted,
//! collected or read back, every wait, and letting one go. The CPU draws
//! those frames either way, so a turn costs little there. A GPU's driver
//! takes calls from many threads, as its API promises, and on one the turn
//! is never taken.
//!
//! The turn is reentrant: a call that already has it - a frame that lets
//! go of a texture, a compositor built while its device opens - goes on.
//!
//! `CONCAT_GPU_SERIAL=0` leaves the turn off. It is for CI's diagnostic
//! runs alone, which set it to see whether a failure on WARP comes and
//! goes with the turn; nobody else should.

use std::cell::RefCell;
use std::sync::OnceLock;

use parking_lot::{ReentrantMutex, ReentrantMutexGuard};

/// The one turn the process has.
static TURN: ReentrantMutex<()> = ReentrantMutex::new(());

/// Held while a call into a software adapter's device runs; holds nothing
/// on a GPU. Let go when dropped.
#[must_use = "the turn is let go the moment this is dropped"]
pub(crate) struct Turn(Option<ReentrantMutexGuard<'static, ()>>);

impl Turn {
    /// Whether this is a turn taken, not one a GPU passed over.
    #[cfg(test)]
    pub(crate) fn taken(&self) -> bool {
        self.0.is_some()
    }
}

/// The turn, for a call into a device on a software adapter when
/// `software`; nothing, at no cost, when not.
pub(crate) fn take(software: bool) -> Turn {
    Turn((software && serial()).then(|| TURN.lock()))
}

/// The turn for opening a device, where the adapter it lands on is not
/// known yet: taken when the software adapter is what is asked for, and
/// on Windows whatever is asked. There wgpu opens a device on every
/// adapter it finds, WARP among them, to read what each can do, so any
/// open in the process touches WARP's device.
pub(crate) fn opening(software: bool) -> Turn {
    take(software || cfg!(windows))
}

/// Says once, in the log, how drawing on a software adapter goes: in
/// turns, or not, as `CONCAT_GPU_SERIAL` asks.
pub(crate) fn say_once() {
    static SAID: std::sync::Once = std::sync::Once::new();
    SAID.call_once(|| {
        if serial() {
            log::info!(
                "gpu: on a software adapter every drawing takes its turn, one thread at a time"
            );
        } else {
            log::warn!(
                "gpu: on a software adapter drawing does not take turns, as CONCAT_GPU_SERIAL=0 \
                 asks; that is for CI's diagnostic runs alone"
            );
        }
    });
}

/// Whether the turn is on: always, but where `CONCAT_GPU_SERIAL=0`.
fn serial() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| serial_from(std::env::var_os("CONCAT_GPU_SERIAL")))
}

/// [`serial`] for the variable's value, `None` when it is not set.
fn serial_from(value: Option<std::ffi::OsString>) -> bool {
    value.is_none_or(|value| value.to_string_lossy().trim() != "0")
}

thread_local! {
    /// The turns [`UntilDropped::hold`] took on this thread, let go by the
    /// [`UntilDropped`] that took each, last first.
    static HELD: RefCell<Vec<ReentrantMutexGuard<'static, ()>>> = const { RefCell::new(Vec::new()) };
}

/// The turn, held from a struct's `Drop` through every field it lets go.
///
/// A `Drop` that takes the turn lets it go when it returns, and the
/// fields (the device, its pipelines, its buffers) are let go after that.
/// Made the struct's last field, this holds the turn its owner's `Drop` took
/// with [`UntilDropped::hold`] until it is dropped itself: the fields go
/// in the order they are declared, so last of all.
#[derive(Default)]
pub(crate) struct UntilDropped {
    held: bool,
}

impl UntilDropped {
    /// Takes the turn as [`take`] does, and keeps it until this is
    /// dropped. Called from the owner's `Drop`.
    pub(crate) fn hold(&mut self, software: bool) {
        if let Turn(Some(guard)) = take(software) {
            HELD.with(|held| held.borrow_mut().push(guard));
            self.held = true;
        }
    }
}

impl Drop for UntilDropped {
    fn drop(&mut self) {
        if self.held {
            // Nothing left to let go if the thread is being torn down.
            let _ = HELD.try_with(|held| held.borrow_mut().pop());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    /// Whether another thread can take the turn within `wait`.
    fn free_elsewhere(wait: Duration) -> bool {
        std::thread::scope(|scope| {
            scope
                .spawn(|| TURN.try_lock_for(wait).is_some())
                .join()
                .expect("the thread ran")
        })
    }

    /// The turn is taken on a software adapter, and taken again within
    /// itself on the same thread, while another thread waits; on a GPU it
    /// is never taken at all.
    #[test]
    fn the_turn_is_reentrant_and_passed_over_on_a_gpu() {
        if !serial() {
            eprintln!("CONCAT_GPU_SERIAL=0 here; skipped");
            return;
        }
        let gpu = take(false);
        assert!(!gpu.taken());
        assert!(
            free_elsewhere(Duration::from_secs(30)),
            "a GPU takes no turn"
        );
        drop(gpu);

        let outer = take(true);
        assert!(outer.taken());
        let inner = take(true);
        assert!(inner.taken(), "the same thread goes on");
        assert!(!free_elsewhere(Duration::from_millis(50)), "others wait");
        drop(inner);
        assert!(
            !free_elsewhere(Duration::from_millis(50)),
            "the outer still holds it"
        );
        drop(outer);
        assert!(free_elsewhere(Duration::from_secs(30)), "let go");
    }

    /// The turn a `Drop` takes is held until the last field goes.
    #[test]
    fn the_turn_is_held_through_every_field_a_drop_lets_go() {
        if !serial() {
            eprintln!("CONCAT_GPU_SERIAL=0 here; skipped");
            return;
        }
        struct Field(mpsc::Sender<bool>);
        impl Drop for Field {
            fn drop(&mut self) {
                let _ = self.0.send(TURN.is_owned_by_current_thread());
            }
        }
        struct Owner {
            _first: Field,
            _second: Field,
            released: UntilDropped,
        }
        impl Drop for Owner {
            fn drop(&mut self) {
                self.released.hold(true);
            }
        }
        let (tell, told) = mpsc::channel();
        drop(Owner {
            _first: Field(tell.clone()),
            _second: Field(tell),
            released: UntilDropped::default(),
        });
        assert_eq!(told.iter().collect::<Vec<_>>(), [true, true]);
        assert!(!TURN.is_owned_by_current_thread(), "let go after the last");
        assert!(free_elsewhere(Duration::from_secs(30)));

        // A GPU's owner holds nothing while it goes.
        struct Gpu {
            _field: Field,
            released: UntilDropped,
        }
        impl Drop for Gpu {
            fn drop(&mut self) {
                self.released.hold(false);
            }
        }
        let (tell, told) = mpsc::channel();
        drop(Gpu {
            _field: Field(tell),
            released: UntilDropped::default(),
        });
        assert_eq!(told.iter().collect::<Vec<_>>(), [false]);
    }

    /// A compositor on a software adapter draws in its turn, waiting while
    /// another thread has it; one on a GPU draws whoever has it, never
    /// waiting at all.
    #[test]
    fn a_software_adapter_draws_in_its_turn_and_a_gpu_never_waits() {
        use std::sync::Arc;

        use crate::compositor::Compositor;
        use crate::gpu::WgpuCompositor;
        use crate::plan::{FramePlan, PlannedLayer, detached_clip};

        if !serial() {
            eprintln!("CONCAT_GPU_SERIAL=0 here; skipped");
            return;
        }
        let (Some(gpu), Some(software)) = (WgpuCompositor::new(), WgpuCompositor::new()) else {
            eprintln!("no adapter here; skipped");
            return;
        };
        let mut picture = concat_core::frame::Frame::transparent(8, 8);
        picture.fill([220, 20, 20, 255]);
        let plan = FramePlan {
            layers: vec![PlannedLayer::picture(detached_clip(), Arc::new(picture))],
            ..FramePlan::empty(16, 16)
        };
        let draw_elsewhere = |compositor: WgpuCompositor| {
            let plan = plan.clone();
            let (done, drawn) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                let mut compositor = compositor;
                let frame = compositor.render(&plan);
                let _ = done.send(frame.pixel(8, 8));
            });
            (drawn, thread)
        };

        let held = take(true);
        let (drawn, thread) = draw_elsewhere(gpu.treated_as_software(false));
        let pixel = drawn
            .recv_timeout(Duration::from_secs(60))
            .expect("a GPU draws while another thread has the turn");
        assert!(pixel.is_some_and(|p| p[0] > 200), "{pixel:?}");
        thread.join().expect("drew");

        let (drawn, thread) = draw_elsewhere(software.treated_as_software(true));
        assert!(
            drawn.recv_timeout(Duration::from_millis(300)).is_err(),
            "a software adapter waits for its turn"
        );
        drop(held);
        let pixel = drawn
            .recv_timeout(Duration::from_secs(60))
            .expect("and draws once it has it");
        assert!(pixel.is_some_and(|p| p[0] > 200), "{pixel:?}");
        thread.join().expect("drew");
    }

    /// Only `CONCAT_GPU_SERIAL=0` turns it off.
    #[test]
    fn only_zero_turns_the_turn_off() {
        assert!(serial_from(None));
        assert!(serial_from(Some("1".into())));
        assert!(serial_from(Some("".into())));
        assert!(!serial_from(Some("0".into())));
        assert!(!serial_from(Some(" 0 ".into())));
    }
}
