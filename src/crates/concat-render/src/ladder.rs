// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The preview's compositor, which cannot go black.
//!
//! A [`Ladder`] draws on the first adapter of [`WgpuCompositor::rungs`]
//! that opens, and when its device is lost it lets the device go before
//! asking for another: Direct3D 12 opens one device an adapter in a
//! process, and while anything holds the removed one it opens none, so a
//! compositor that asked with the dead one in hand was refused on every
//! frame for the rest of the run, and the preview stayed black (#709).
//! The same rung is tried once more; a rung that will not open, or dies
//! a second time, is stepped past, an adapter that died twice is never
//! drawn on again, and below the last rung the CPU draws ([`CpuCompositor`]),
//! with effects left out. Each loss costs a few device opens at most and
//! never one a frame.

use concat_core::frame::Frame;
use concat_core::shader::TransitionPass;

use crate::compositor::Compositor;
use crate::gpu::{Rung, WgpuCompositor};
use crate::plan::FramePlan;
use crate::reference::CpuCompositor;

/// The preview's compositor: a device while one draws, the CPU after.
pub struct Ladder {
    gpu: Option<WgpuCompositor>,
    rungs: Vec<Rung>,
    /// The rung `gpu` is on, or the next to try; `rungs.len()` is the CPU.
    at: usize,
    /// The rung `at` has been opened again after a loss on it.
    reopened: bool,
    /// Adapters that died twice, by name and API.
    dead: Vec<(String, wgpu::Backend)>,
    /// Devices asked for, all told.
    opened: usize,
    cpu: CpuRung,
}

impl Ladder {
    /// Opens the first rung that opens, or draws on the CPU.
    pub fn new() -> Self {
        let mut ladder = Self::with_rungs(WgpuCompositor::rungs());
        ladder.climb();
        ladder
    }

    /// A ladder with no device, drawing on the CPU from the start.
    pub fn cpu_only() -> Self {
        Self::with_rungs(Vec::new())
    }

    fn with_rungs(rungs: Vec<Rung>) -> Self {
        Ladder {
            gpu: None,
            rungs,
            at: 0,
            reopened: false,
            dead: Vec::new(),
            opened: 0,
            cpu: CpuRung::default(),
        }
    }

    /// Whether frames are drawn on the CPU, slower and without effects.
    pub fn on_cpu(&self) -> bool {
        self.gpu.is_none()
    }

    /// Runs `draw` on the compositor in use, and again on the next if its
    /// device was lost under it, until one survives; the CPU always does.
    /// Hands back what `draw` made and whether the CPU left anything of it
    /// out (see [`CpuCompositor::leaves_out`]).
    pub fn draw<T>(&mut self, mut draw: impl FnMut(&mut dyn Compositor) -> T) -> (T, bool) {
        loop {
            let Some(gpu) = self.gpu.as_mut() else {
                self.cpu.left_out = false;
                let made = draw(&mut self.cpu);
                return (made, self.cpu.left_out);
            };
            let made = draw(gpu);
            if !gpu.lost() {
                return (made, false);
            }
            self.recover();
        }
    }

    /// After a loss: the dead device let go first - every texture, buffer
    /// and pipeline on it with it - then its rung once more, or the next.
    fn recover(&mut self) {
        let Some(gpu) = self.gpu.take() else { return };
        let info = gpu.adapter_info();
        drop(gpu);
        if self.reopened {
            self.dead.push((info.name.clone(), info.backend));
            self.at += 1;
            self.reopened = false;
        } else {
            self.reopened = true;
        }
        log::warn!(
            "gpu: previews let go of the lost device on {} ({:?}); {}",
            info.name,
            info.backend,
            if self.reopened {
                "opening it once more"
            } else {
                "it died twice, so the next adapter"
            }
        );
        self.climb();
    }

    /// Opens rungs from `at` down until one gives a device on an adapter
    /// that has not died twice; past the last, the CPU.
    fn climb(&mut self) {
        while let Some(&rung) = self.rungs.get(self.at) {
            self.opened += 1;
            if let Some(gpu) = WgpuCompositor::on_rung(rung) {
                let info = gpu.adapter_info();
                if !self.dead.contains(&(info.name.clone(), info.backend)) {
                    log::info!(
                        "gpu: previews draw on {} ({:?}, {:?})",
                        info.name,
                        info.device_type,
                        info.backend
                    );
                    self.gpu = Some(gpu);
                    return;
                }
            }
            self.at += 1;
            self.reopened = false;
        }
        log::warn!(
            "gpu: previews draw on the CPU from here, since no adapter opens a device that lasts; \
             effects are left out of them"
        );
    }
}

impl Default for Ladder {
    fn default() -> Self {
        Self::new()
    }
}

/// The CPU compositor, noting what it leaves out of the frames it draws.
#[derive(Default)]
struct CpuRung {
    left_out: bool,
}

impl Compositor for CpuRung {
    fn render(&mut self, plan: &FramePlan) -> Frame {
        self.left_out |= CpuCompositor::leaves_out(plan);
        CpuCompositor.render(plan)
    }

    fn combine(
        &mut self,
        width: u32,
        height: u32,
        time: f32,
        from: &Frame,
        to: &Frame,
        pass: &TransitionPass,
    ) -> Option<Frame> {
        let combined = CpuCompositor.combine(width, height, time, from, to, pass);
        // Declined, the caller draws a dissolve in its place.
        self.left_out |= combined.is_none();
        combined
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::plan::{PlannedLayer, detached_clip};

    fn red() -> FramePlan {
        let mut picture = Frame::transparent(32, 18);
        picture.fill([220, 20, 20, 255]);
        FramePlan {
            layers: vec![PlannedLayer::picture(detached_clip(), Arc::new(picture))],
            ..FramePlan::empty(32, 18)
        }
    }

    fn is_red(frame: &Frame) -> bool {
        frame.pixel(16, 9).is_some_and(|p| p[0] > 200 && p[1] < 40)
    }

    /// A lost device is let go and its rung opened once more; a second
    /// loss steps down, and losses past every rung end on the CPU, which
    /// draws the frame - never black, whatever dies - and asks for no
    /// device again, frame after frame.
    #[test]
    fn a_lost_device_is_reopened_once_then_stepped_down_to_the_cpu() {
        let plan = red();
        let mut ladder = Ladder::new();
        let mut losses = 0;
        while let Some(gpu) = ladder.gpu.as_ref() {
            let before = gpu.adapter_info();
            gpu.lose_device();
            losses += 1;
            let (frame, left_out) = ladder.draw(|compositor| compositor.render(&plan));
            assert!(is_red(&frame), "loss {losses}: drawn again, not black");
            assert!(!left_out);
            if losses == 1
                && let Some(again) = ladder.gpu.as_ref()
            {
                let now = again.adapter_info();
                assert_eq!(
                    (before.name, before.backend),
                    (now.name, now.backend),
                    "the same rung once more"
                );
            }
            assert!(losses <= 2 * ladder.rungs.len(), "the ladder ends");
        }
        assert!(ladder.on_cpu());
        eprintln!(
            "on the CPU after {losses} losses, {} devices asked for",
            ladder.opened
        );
        let asked = ladder.opened;
        for _ in 0..10 {
            let (frame, _) = ladder.draw(|compositor| compositor.render(&plan));
            assert!(is_red(&frame));
        }
        assert_eq!(ladder.opened, asked, "no device asked for per frame");
    }

    /// The CPU says when it left something out of a frame.
    #[test]
    fn the_cpu_rung_says_what_it_left_out() {
        let mut ladder = Ladder::cpu_only();
        let plain = red();
        assert!(!ladder.draw(|compositor| compositor.render(&plain)).1);
        let mut deep = plain.clone();
        deep.output = concat_core::frame::Signal::Pq;
        assert!(ladder.draw(|compositor| compositor.render(&deep)).1);
        assert!(
            !ladder.draw(|compositor| compositor.render(&plain)).1,
            "said per frame"
        );
    }
}
