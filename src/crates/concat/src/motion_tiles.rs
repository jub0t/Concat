// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The Animate panel's cards, played from the engine's own poses: each
//! entrance, exit and loop sampled a thirtieth of a second apart, so a card
//! moves exactly as a clip given that move will.

use concat_core::motion::{Cycle, Move, Pose};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::ui::{App, MotionPose, MotionTile, MotionTiles};

/// Frames in a card's move, then frames it rests before going round again.
const MOVING: usize = 20;
const RESTING: usize = 14;

fn pose(p: Pose) -> MotionPose {
    MotionPose {
        dx: p.dx as f32,
        dy: p.dy as f32,
        scale: p.scale as f32,
        rotation: p.rotation as f32,
        opacity: p.opacity as f32,
        sx: p.stretch_x as f32,
        sy: p.stretch_y as f32,
    }
}

fn tile(frames: Vec<Pose>, still: Pose) -> MotionTile {
    MotionTile {
        frames: ModelRc::new(VecModel::from(
            frames.into_iter().map(pose).collect::<Vec<_>>(),
        )),
        still: pose(still),
    }
}

/// An entrance's card: arriving, then at rest.
fn entering(kind: Move) -> MotionTile {
    let arriving = (0..MOVING).map(|i| kind.pose(i as f64 / MOVING as f64 - 1.0));
    let frames = arriving
        .chain(std::iter::repeat_n(Pose::REST, RESTING))
        .collect();
    tile(frames, kind.pose(-0.4))
}

/// An exit's card: at rest, then leaving, then a beat of empty frame.
fn leaving(kind: Move) -> MotionTile {
    let rest = std::iter::repeat_n(Pose::REST, RESTING / 2);
    let going = (0..=MOVING).map(|i| kind.pose(i as f64 / MOVING as f64));
    let gone = std::iter::repeat_n(kind.pose(1.0), RESTING / 2);
    tile(rest.chain(going).chain(gone).collect(), kind.pose(0.4))
}

/// A loop's card: two laps of a second each.
fn looping(kind: Cycle) -> MotionTile {
    let frames = (0..60).map(|i| kind.pose(i as f64 / 30.0, 1.0)).collect();
    tile(frames, kind.pose(0.3, 1.0))
}

/// Gives the panel its cards; once, at start.
pub(crate) fn install(app: &App) {
    let tiles = app.global::<MotionTiles>();
    let list = |cards: Vec<MotionTile>| ModelRc::new(VecModel::from(cards));
    tiles.set_enter(list(Move::ALL.into_iter().map(entering).collect()));
    tiles.set_exit(list(Move::ALL.into_iter().map(leaving).collect()));
    tiles.set_cycle(list(Cycle::ALL.into_iter().map(looping).collect()));
}
