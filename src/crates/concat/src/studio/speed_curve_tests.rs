// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The Speed tab's curve graph, without a window: the scale speed is drawn
//! on, what putting down, dragging and taking off points does, the two
//! extreme sliders, Smooth and Straight, the drawn line, the presets' names,
//! and the edits a change on the graph lands as.

use concat_project::Command;
use concat_project::model::{Clip, ClipKind, SpeedPoint};
use concat_project::speed::is_smooth;

use crate::speed_ramp::{
    extreme, flat, height_of, move_chosen, path, preset_index, preset_names, preset_points,
    remove_point, set_extreme, set_point, set_smooth, speed_at_height,
};

const TIGHT: f64 = 1e-9;

fn close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < TIGHT,
        "{what}: got {actual}, expected {expected}"
    );
}

/// The points' places across the clip, against what is expected.
fn ats(points: &[SpeedPoint], expected: &[f64]) {
    assert_eq!(points.len(), expected.len(), "how many points");
    for (point, at) in points.iter().zip(expected) {
        close(point.at, *at, "place across the clip");
    }
}

#[test]
fn speed_is_drawn_on_a_scale_of_doublings_from_a_sixteenth_to_sixteen() {
    for (speed, height) in [
        (0.0625, 0.0),
        (0.125, 0.125),
        (0.25, 0.25),
        (0.5, 0.375),
        (1.0, 0.5),
        (2.0, 0.625),
        (4.0, 0.75),
        (8.0, 0.875),
        (16.0, 1.0),
    ] {
        close(height_of(speed), height, "height");
        close(speed_at_height(height), speed, "speed");
    }
    close(
        height_of(3.0),
        (3f64.log2() + 4.0) / 8.0,
        "between the lines",
    );
    // Off the scale, held to its ends.
    close(height_of(100.0), 1.0, "above");
    close(height_of(0.0), 0.0, "below");
    close(speed_at_height(1.5), 16.0, "above");
    close(speed_at_height(-1.0), 0.0625, "below");
    close(speed_at_height(f64::NAN), 1.0, "nonsense is normal speed");
}

#[test]
fn a_curve_starts_flat_and_smooth_at_the_clips_rate() {
    let start = flat(1.5);
    assert_eq!(
        start,
        vec![
            SpeedPoint {
                at: 0.0,
                speed: 1.5,
                slope: Some(0.0),
            },
            SpeedPoint {
                at: 1.0,
                speed: 1.5,
                slope: Some(0.0),
            },
        ]
    );
}

#[test]
fn a_point_put_down_lands_in_time_order_inside_the_ends_and_bends_like_the_rest() {
    let mut points = flat(1.0);
    set_point(&mut points, -1, 0.6, 0.75);
    set_point(&mut points, -1, 0.3, 0.25);
    ats(&points, &[0.0, 0.3, 0.6, 1.0]);
    close(points[1].speed, 0.25, "a quarter of the way down is 1/4x");
    close(points[2].speed, 4.0, "three quarters up is 4x");
    assert!(
        is_smooth(&points),
        "a new point on a smooth curve is smooth"
    );
    // On the very ends, it is kept a hair inside them.
    set_point(&mut points, -1, 0.0, 0.5);
    set_point(&mut points, -1, 1.0, 0.5);
    ats(&points, &[0.0, 0.002, 0.3, 0.6, 0.998, 1.0]);
    // On a straight curve, straight.
    let mut lines = flat(1.0);
    set_smooth(&mut lines, false);
    set_point(&mut lines, -1, 0.5, 0.5);
    assert!(lines.iter().all(|point| point.slope.is_none()));
    // Nonsense across lands in the middle.
    let mut points = flat(1.0);
    set_point(&mut points, -1, f64::NAN, 0.5);
    ats(&points, &[0.0, 0.5, 1.0]);
}

#[test]
fn a_dragged_point_stays_between_its_neighbours_and_the_ends_only_change_speed() {
    let mut points = flat(1.0);
    set_point(&mut points, -1, 0.4, 0.5);
    set_point(&mut points, -1, 0.6, 0.5);
    set_point(&mut points, 1, 0.9, 0.625);
    close(points[1].at, 0.598, "stops a hair before the next point");
    close(points[1].speed, 2.0, "and takes its height's speed");
    set_point(&mut points, 1, -1.0, 0.5);
    close(points[1].at, 0.002, "and a hair after the start");
    set_point(&mut points, 0, 0.5, 0.875);
    close(points[0].at, 0.0, "the start stays at the start");
    close(points[0].speed, 8.0, "and moves in speed");
    set_point(&mut points, 3, 0.2, 0.25);
    close(points[3].at, 1.0, "the end stays at the end");
    assert_eq!(points[0].slope, Some(0.0), "a drag keeps the point's bend");
    // A point that is not there is left alone.
    let before = points.clone();
    set_point(&mut points, 9, 0.5, 0.5);
    assert_eq!(points, before);
}

#[test]
fn the_ends_never_come_off() {
    let mut points = flat(1.0);
    set_point(&mut points, -1, 0.5, 0.5);
    for index in [0, 2, -1, 7] {
        remove_point(&mut points, index);
    }
    assert_eq!(points.len(), 3);
    remove_point(&mut points, 1);
    ats(&points, &[0.0, 1.0]);
}

/// Points at 0, 0.2, 0.4, 0.6 and 1, all at 1x.
fn five() -> Vec<SpeedPoint> {
    let mut points = flat(1.0);
    for at in [0.2, 0.4, 0.6] {
        set_point(&mut points, -1, at, 0.5);
    }
    points
}

#[test]
fn chosen_points_move_together_and_keep_their_spacing_against_the_points_that_stay() {
    let from = five();
    let mut points = from.clone();
    move_chosen(&mut points, &from, &[1, 2], -0.5, -0.25);
    // Held a hair after the start, the pair keeping its 0.2 apart.
    ats(&points, &[0.0, 0.002, 0.202, 0.6, 1.0]);
    close(points[1].speed, 0.25, "two doublings down");
    close(points[2].speed, 0.25, "both of them");
    close(points[3].speed, 1.0, "the unchosen stay");
    // Each move is measured from where the drag began.
    move_chosen(&mut points, &from, &[1, 2], 0.05, 0.0);
    ats(&points, &[0.0, 0.25, 0.45, 0.6, 1.0]);
    close(points[1].speed, 1.0, "back at the start's height");
}

#[test]
fn a_chosen_end_moves_only_in_speed_and_each_height_is_held_to_the_graph() {
    let mut from = five();
    from[3].speed = 8.0;
    let mut points = from.clone();
    move_chosen(&mut points, &from, &[0, 3, 4], 0.3, 0.25);
    close(points[0].at, 0.0, "the start stays");
    close(points[4].at, 1.0, "the end stays");
    close(points[3].at, 0.9, "the middle point moves across");
    close(points[0].speed, 4.0, "two doublings up");
    close(points[3].speed, 16.0, "held to the top");
}

#[test]
fn a_group_drag_that_fits_nowhere_does_not_move_across_and_nonsense_does_nothing() {
    // Three points within two hairs of each other, the middle one alone
    // chosen: no shift keeps it a hair from both.
    let from = vec![
        SpeedPoint {
            at: 0.0,
            speed: 1.0,
            slope: None,
        },
        SpeedPoint {
            at: 0.5,
            speed: 1.0,
            slope: None,
        },
        SpeedPoint {
            at: 0.501,
            speed: 1.0,
            slope: None,
        },
        SpeedPoint {
            at: 0.502,
            speed: 1.0,
            slope: None,
        },
        SpeedPoint {
            at: 1.0,
            speed: 1.0,
            slope: None,
        },
    ];
    let mut points = from.clone();
    move_chosen(&mut points, &from, &[2], 0.1, 0.125);
    close(points[2].at, 0.501, "no room across");
    close(points[2].speed, 2.0, "still moves in speed");
    let mut points = from.clone();
    move_chosen(&mut points, &from, &[2], f64::NAN, 0.0);
    assert_eq!(points, from);
    move_chosen(&mut points, &from[..3], &[1], 0.1, 0.0);
    assert_eq!(points, from, "a curve that is not the one grabbed");
}

#[test]
fn the_extremes_are_the_fastest_above_normal_and_the_slowest_below() {
    let hero = preset_points(1).unwrap();
    assert_eq!(extreme(&hero, true), Some(2.5));
    assert_eq!(extreme(&hero, false), Some(0.5));
    let flash_in = preset_points(4).unwrap();
    assert_eq!(extreme(&flash_in, false), None, "nothing slower than 1x");
    assert_eq!(extreme(&flat(1.0), true), None);
    assert_eq!(
        extreme(&flat(1.000_000_5), true),
        None,
        "within a hair of 1x"
    );
}

#[test]
fn a_slider_raises_every_speed_on_its_side_to_one_power() {
    let mut points = preset_points(4).unwrap(); // 8x, 4x, 1x, 1x
    set_extreme(&mut points, true, 2.0);
    // 8x to 2x is a third of the doublings, for every fast point.
    close(points[0].speed, 2.0, "the fastest");
    close(points[1].speed, 4f64.powf(1.0 / 3.0), "in proportion");
    close(points[2].speed, 1.0, "normal stays");
    // A slope follows its speed by the chain rule.
    let mut bent = vec![
        SpeedPoint {
            at: 0.0,
            speed: 8.0,
            slope: Some(0.0),
        },
        SpeedPoint {
            at: 0.5,
            speed: 4.0,
            slope: Some(2.0),
        },
        SpeedPoint {
            at: 1.0,
            speed: 0.5,
            slope: Some(-1.0),
        },
    ];
    set_extreme(&mut bent, true, 2.0);
    close(
        bent[1].slope.unwrap(),
        2.0 / 3.0 * 4f64.powf(-2.0 / 3.0),
        "scaled slope",
    );
    assert_eq!(bent[2].slope, Some(-1.0), "the slow side untouched");
    // The slow side, the same way.
    let mut hero = preset_points(1).unwrap();
    set_extreme(&mut hero, false, 0.25);
    close(hero[2].speed, 0.25, "slowest");
    close(hero[0].speed, 2.5, "fast side untouched");
}

#[test]
fn a_slider_cannot_bring_its_side_to_normal_or_past_the_range() {
    let mut points = preset_points(4).unwrap();
    set_extreme(&mut points, true, 1.0);
    close(points[0].speed, 1.1, "held a tenth above 1x");
    set_extreme(&mut points, true, 100.0);
    close(points[0].speed, 16.0, "held to 16x");
    let mut hero = preset_points(1).unwrap();
    set_extreme(&mut hero, false, 0.99);
    close(hero[2].speed, 0.9, "held a tenth below 1x");
    set_extreme(&mut hero, false, 0.0);
    close(hero[2].speed, 0.0625, "held to 1/16x");
    // Nothing on that side: nothing happens.
    let mut normal = flat(1.0);
    set_extreme(&mut normal, true, 4.0);
    assert_eq!(normal, flat(1.0));
}

#[test]
fn smooth_and_straight_switch_every_point() {
    let mut points = preset_points(0).unwrap();
    set_smooth(&mut points, false);
    assert!(points.iter().all(|point| point.slope.is_none()));
    assert_eq!(preset_index(&points), 0, "still the preset");
    set_smooth(&mut points, true);
    assert!(points.iter().all(|point| point.slope == Some(0.0)));
}

#[test]
fn the_line_is_drawn_through_the_points_and_sixteen_steps_along_each_bend() {
    let mut lines = vec![
        SpeedPoint {
            at: 0.0,
            speed: 1.0,
            slope: None,
        },
        SpeedPoint {
            at: 1.0,
            speed: 2.0,
            slope: None,
        },
    ];
    assert_eq!(path(&lines), "M 0.0000 0.5000 L 1.0000 0.3750");
    set_smooth(&mut lines, true);
    let drawn = path(&lines);
    assert_eq!(drawn.matches('L').count(), 16);
    assert!(drawn.starts_with("M 0.0000 0.5000 L 0.0625 "));
    assert!(drawn.ends_with("L 1.0000 0.3750"));
    // Half way along the ease, half way up in speed: 1.5x.
    let middle = format!("L 0.5000 {:.4}", 1.0 - height_of(1.5));
    assert!(drawn.contains(&middle), "{drawn}");
    // A point off the start is drawn from the start.
    let late = vec![SpeedPoint {
        at: 0.5,
        speed: 4.0,
        slope: None,
    }];
    assert_eq!(
        path(&late),
        "M 0.0000 0.2500 L 0.5000 0.2500 L 1.0000 0.2500"
    );
    assert_eq!(path(&[]), "");
}

#[test]
fn the_presets_are_listed_by_their_english_names() {
    assert_eq!(
        preset_names(),
        vec![
            "Montage",
            "Hero",
            "Bullet",
            "Jump cut",
            "Flash in",
            "Flash out"
        ]
    );
    for index in 0..6 {
        assert_eq!(preset_index(&preset_points(index).unwrap()), index);
    }
    assert!(preset_points(6).is_none());
    assert!(preset_points(-1).is_none());
    let mut shaped = preset_points(2).unwrap();
    let at = shaped[1].at;
    set_point(&mut shaped, 1, at, 0.6);
    assert_eq!(preset_index(&shaped), -1, "shaped by hand, custom");
}

fn curved(points: Option<Vec<SpeedPoint>>, speed: f64, duration: f64) -> Clip {
    let mut clip = Clip::blank("c1", "t1", ClipKind::Video, "clip", 0.0, duration);
    clip.speed = speed;
    clip.speed_curve = points;
    clip
}

#[test]
fn a_change_on_the_graph_lands_as_one_curve_edit() {
    let before = curved(None, 1.0, 10.0);
    let after = curved(preset_points(1), 1.76, 10.0 / 1.76);
    let commands = super::clip_commands("c1", &before, &after, true);
    assert_eq!(commands.len(), 1);
    assert_eq!(
        commands[0],
        Command::SetClipSpeedCurve {
            clip_id: "c1".to_owned(),
            curve: preset_points(1),
            ripple: true,
        }
    );
}

#[test]
fn switching_to_constant_lands_as_the_curves_mean_rate() {
    let before = curved(preset_points(1), 1.76, 10.0 / 1.76);
    let after = curved(None, 1.76, 10.0 / 1.76);
    let commands = super::clip_commands("c1", &before, &after, false);
    assert_eq!(
        commands,
        vec![Command::SetClipSpeed {
            clip_id: "c1".to_owned(),
            speed: 1.76,
            ripple: false,
        }]
    );
}
