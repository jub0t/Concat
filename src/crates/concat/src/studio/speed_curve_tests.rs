// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The Speed tab's curve graph, without a window: the scale speed is drawn
//! on, what putting down, dragging and taking off points does, the two
//! extreme sliders, Smooth and Straight, the drawn line, the presets' names,
//! and the edits a change on the graph lands as.

use concat_doc::commands::{Command, speed_from_decimal, speed_graph};
use concat_doc::model::SpeedPoint;
use concat_doc::model::{Item, ItemId, MediaKind};

use super::chains::{curve_preview, speed_commands, time_map_mut};
use crate::doc_view::{self, bench::Bench};
use crate::speed_ramp::{
    apply_easing, curve_path, drop_point, extreme_of, height_for, is_eased, level_curve,
    move_selected, place_point, preset_by_index, preset_index, preset_labels, speed_at_height,
    stretch_extreme,
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
        close(height_for(speed), height, "height");
        close(speed_at_height(height), speed, "speed");
    }
    close(
        height_for(3.0),
        (3f64.log2() + 4.0) / 8.0,
        "between the lines",
    );
    // Off the scale, held to its ends.
    close(height_for(100.0), 1.0, "above");
    close(height_for(0.0), 0.0, "below");
    close(speed_at_height(1.5), 16.0, "above");
    close(speed_at_height(-1.0), 0.0625, "below");
    close(speed_at_height(f64::NAN), 1.0, "nonsense is normal speed");
}

#[test]
fn a_curve_starts_flat_and_smooth_at_the_clips_rate() {
    let start = level_curve(1.5);
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
    let mut points = level_curve(1.0);
    place_point(&mut points, -1, 0.6, 0.75);
    place_point(&mut points, -1, 0.3, 0.25);
    ats(&points, &[0.0, 0.3, 0.6, 1.0]);
    close(points[1].speed, 0.25, "a quarter of the way down is 1/4x");
    close(points[2].speed, 4.0, "three quarters up is 4x");
    assert!(is_eased(&points), "a new point on a smooth curve is smooth");
    // On the very ends, it is kept a hair inside them.
    place_point(&mut points, -1, 0.0, 0.5);
    place_point(&mut points, -1, 1.0, 0.5);
    ats(&points, &[0.0, 0.002, 0.3, 0.6, 0.998, 1.0]);
    // On a straight curve, straight.
    let mut lines = level_curve(1.0);
    apply_easing(&mut lines, false);
    place_point(&mut lines, -1, 0.5, 0.5);
    assert!(lines.iter().all(|point| point.slope.is_none()));
    // Nonsense across lands in the middle.
    let mut points = level_curve(1.0);
    place_point(&mut points, -1, f64::NAN, 0.5);
    ats(&points, &[0.0, 0.5, 1.0]);
}

#[test]
fn a_dragged_point_stays_between_its_neighbours_and_the_ends_only_change_speed() {
    let mut points = level_curve(1.0);
    place_point(&mut points, -1, 0.4, 0.5);
    place_point(&mut points, -1, 0.6, 0.5);
    place_point(&mut points, 1, 0.9, 0.625);
    close(points[1].at, 0.598, "stops a hair before the next point");
    close(points[1].speed, 2.0, "and takes its height's speed");
    place_point(&mut points, 1, -1.0, 0.5);
    close(points[1].at, 0.002, "and a hair after the start");
    place_point(&mut points, 0, 0.5, 0.875);
    close(points[0].at, 0.0, "the start stays at the start");
    close(points[0].speed, 8.0, "and moves in speed");
    place_point(&mut points, 3, 0.2, 0.25);
    close(points[3].at, 1.0, "the end stays at the end");
    assert_eq!(points[0].slope, Some(0.0), "a drag keeps the point's bend");
    // A point that is not there is left alone.
    let before = points.clone();
    place_point(&mut points, 9, 0.5, 0.5);
    assert_eq!(points, before);
}

#[test]
fn the_ends_never_come_off() {
    let mut points = level_curve(1.0);
    place_point(&mut points, -1, 0.5, 0.5);
    for index in [0, 2, -1, 7] {
        drop_point(&mut points, index);
    }
    assert_eq!(points.len(), 3);
    drop_point(&mut points, 1);
    ats(&points, &[0.0, 1.0]);
}

/// Points at 0, 0.2, 0.4, 0.6 and 1, all at 1x.
fn five() -> Vec<SpeedPoint> {
    let mut points = level_curve(1.0);
    for at in [0.2, 0.4, 0.6] {
        place_point(&mut points, -1, at, 0.5);
    }
    points
}

#[test]
fn chosen_points_move_together_and_keep_their_spacing_against_the_points_that_stay() {
    let from = five();
    let mut points = from.clone();
    move_selected(&mut points, &from, &[1, 2], -0.5, -0.25);
    // Held a hair after the start, the pair keeping its 0.2 apart.
    ats(&points, &[0.0, 0.002, 0.202, 0.6, 1.0]);
    close(points[1].speed, 0.25, "two doublings down");
    close(points[2].speed, 0.25, "both of them");
    close(points[3].speed, 1.0, "the unchosen stay");
    // Each move is measured from where the drag began.
    move_selected(&mut points, &from, &[1, 2], 0.05, 0.0);
    ats(&points, &[0.0, 0.25, 0.45, 0.6, 1.0]);
    close(points[1].speed, 1.0, "back at the start's height");
}

#[test]
fn a_chosen_end_moves_only_in_speed_and_each_height_is_held_to_the_graph() {
    let mut from = five();
    from[3].speed = 8.0;
    let mut points = from.clone();
    move_selected(&mut points, &from, &[0, 3, 4], 0.3, 0.25);
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
    move_selected(&mut points, &from, &[2], 0.1, 0.125);
    close(points[2].at, 0.501, "no room across");
    close(points[2].speed, 2.0, "still moves in speed");
    let mut points = from.clone();
    move_selected(&mut points, &from, &[2], f64::NAN, 0.0);
    assert_eq!(points, from);
    move_selected(&mut points, &from[..3], &[1], 0.1, 0.0);
    assert_eq!(points, from, "a curve that is not the one grabbed");
}

#[test]
fn the_extremes_are_the_fastest_above_normal_and_the_slowest_below() {
    let hero = preset_by_index(1).unwrap();
    assert_eq!(extreme_of(&hero, true), Some(2.5));
    assert_eq!(extreme_of(&hero, false), Some(0.5));
    let flash_in = preset_by_index(4).unwrap();
    assert_eq!(extreme_of(&flash_in, false), None, "nothing slower than 1x");
    assert_eq!(extreme_of(&level_curve(1.0), true), None);
    assert_eq!(
        extreme_of(&level_curve(1.000_000_5), true),
        None,
        "within a hair of 1x"
    );
}

#[test]
fn a_slider_raises_every_speed_on_its_side_to_one_power() {
    let mut points = preset_by_index(4).unwrap(); // 8x, 4x, 1x, 1x
    stretch_extreme(&mut points, true, 2.0);
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
    stretch_extreme(&mut bent, true, 2.0);
    close(
        bent[1].slope.unwrap(),
        2.0 / 3.0 * 4f64.powf(-2.0 / 3.0),
        "scaled slope",
    );
    assert_eq!(bent[2].slope, Some(-1.0), "the slow side untouched");
    // The slow side, the same way.
    let mut hero = preset_by_index(1).unwrap();
    stretch_extreme(&mut hero, false, 0.25);
    close(hero[2].speed, 0.25, "slowest");
    close(hero[0].speed, 2.5, "fast side untouched");
}

#[test]
fn a_slider_cannot_bring_its_side_to_normal_or_past_the_range() {
    let mut points = preset_by_index(4).unwrap();
    stretch_extreme(&mut points, true, 1.0);
    close(points[0].speed, 1.1, "held a tenth above 1x");
    stretch_extreme(&mut points, true, 100.0);
    close(points[0].speed, 16.0, "held to 16x");
    let mut hero = preset_by_index(1).unwrap();
    stretch_extreme(&mut hero, false, 0.99);
    close(hero[2].speed, 0.9, "held a tenth below 1x");
    stretch_extreme(&mut hero, false, 0.0);
    close(hero[2].speed, 0.0625, "held to 1/16x");
    // Nothing on that side: nothing happens.
    let mut normal = level_curve(1.0);
    stretch_extreme(&mut normal, true, 4.0);
    assert_eq!(normal, level_curve(1.0));
}

#[test]
fn smooth_and_straight_switch_every_point() {
    let mut points = preset_by_index(0).unwrap();
    apply_easing(&mut points, false);
    assert!(points.iter().all(|point| point.slope.is_none()));
    assert_eq!(preset_index(&points), 0, "still the preset");
    apply_easing(&mut points, true);
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
    assert_eq!(curve_path(&lines), "M 0.0000 0.5000 L 1.0000 0.3750");
    apply_easing(&mut lines, true);
    let drawn = curve_path(&lines);
    assert_eq!(drawn.matches('L').count(), 16);
    assert!(drawn.starts_with("M 0.0000 0.5000 L 0.0625 "));
    assert!(drawn.ends_with("L 1.0000 0.3750"));
    // Half way along the ease, half way up in speed: 1.5x.
    let middle = format!("L 0.5000 {:.4}", 1.0 - height_for(1.5));
    assert!(drawn.contains(&middle), "{drawn}");
    // A point off the start is drawn from the start.
    let late = vec![SpeedPoint {
        at: 0.5,
        speed: 4.0,
        slope: None,
    }];
    assert_eq!(
        curve_path(&late),
        "M 0.0000 0.2500 L 0.5000 0.2500 L 1.0000 0.2500"
    );
    assert_eq!(curve_path(&[]), "");
}

#[test]
fn the_presets_are_listed_by_their_english_names() {
    assert_eq!(
        preset_labels(),
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
        assert_eq!(preset_index(&preset_by_index(index).unwrap()), index);
    }
    assert!(preset_by_index(6).is_none());
    assert!(preset_by_index(-1).is_none());
    let mut shaped = preset_by_index(2).unwrap();
    let at = shaped[1].at;
    place_point(&mut shaped, 1, at, 0.6);
    assert_eq!(preset_index(&shaped), -1, "shaped by hand, custom");
}

/// A ten-second video clip on a bench, and its id.
fn placed() -> (Bench, ItemId) {
    let mut bench = Bench::new();
    let media = bench.add_media("clip.mp4", MediaKind::Video, 10.0);
    let lane = bench.main_lane();
    let id = bench.add_clip(media, lane, 0.0);
    (bench, id)
}

fn item(bench: &Bench, id: ItemId) -> Item {
    bench
        .doc()
        .items
        .get(id)
        .expect("the clip")
        .as_ref()
        .clone()
}

/// The clip as the Speed tab previews `curve` on the echo - None for a
/// constant pace: the time map and the length the document would give it.
fn previewed(bench: &Bench, id: ItemId, curve: Option<Vec<SpeedPoint>>) -> Item {
    let (map, len) = curve_preview(&bench.document, id, curve).expect("the document takes it");
    let mut after = item(bench, id);
    *time_map_mut(&mut after).expect("a map") = map;
    after.len = len;
    after
}

#[test]
fn a_change_on_the_graph_lands_as_one_curve_edit() {
    let (mut bench, id) = placed();
    let before = item(&bench, id);
    let curve = preset_by_index(1).expect("a preset");
    let after = previewed(&bench, id, Some(curve.clone()));
    // While it is dragged, the graph shows the points as they are set.
    assert_eq!(speed_graph(&after), Some(curve.clone()));
    let command = speed_commands(id, &before, &after, true);
    assert_eq!(
        command,
        Some(Command::SetClipSpeedCurve {
            item: id,
            curve: Some(curve.clone()),
            ripple: true,
        })
    );
    // The command is taken: the clip plays the curve, at the length the
    // echo showed, and the graph gives back the points as drawn.
    bench.apply(&command.expect("an edit"));
    let landed = item(&bench, id);
    assert_eq!(doc_view::time_map(&landed), doc_view::time_map(&after));
    assert_eq!(landed.len, after.len);
    assert_eq!(speed_graph(&landed), Some(curve));
}

#[test]
fn each_move_of_a_drag_is_previewed_from_the_clip_as_committed() {
    let (mut bench, id) = placed();
    bench.apply(&Command::SetClipSpeedCurve {
        item: id,
        curve: preset_by_index(0),
        ripple: false,
    });
    let hero = preset_by_index(1).expect("a preset");
    let once = previewed(&bench, id, Some(hero.clone()));
    let again = previewed(&bench, id, Some(hero));
    assert_eq!(once, again);
}

#[test]
fn curve_mode_on_a_constant_clip_lands_as_a_curve_though_the_pace_is_the_same() {
    for rate in [1.0, 2.0] {
        let (mut bench, id) = placed();
        if rate != 1.0 {
            bench.apply(&Command::SetClipSpeed {
                item: id,
                speed: speed_from_decimal(rate).expect("a pace"),
                ripple: false,
            });
        }
        let before = item(&bench, id);
        assert_eq!(speed_graph(&before), None, "a constant pace has no curve");
        // Curve, level at the clip's own pace: nothing changes on screen.
        let level = level_curve(rate);
        let after = previewed(&bench, id, Some(level.clone()));
        assert_eq!(after.len, before.len, "at {rate}x");
        let command = speed_commands(id, &before, &after, false);
        assert!(
            matches!(
                command,
                Some(Command::SetClipSpeedCurve { curve: Some(_), .. })
            ),
            "at {rate}x: {command:?}"
        );
        bench.apply(&command.expect("an edit"));
        let landed = item(&bench, id);
        assert_eq!(speed_graph(&landed), Some(level), "at {rate}x");
        assert_eq!(landed.len, before.len, "at {rate}x");
    }
}

#[test]
fn switching_to_constant_lands_as_the_curves_mean_rate() {
    let (mut bench, id) = placed();
    bench.apply(&Command::SetClipSpeedCurve {
        item: id,
        curve: preset_by_index(1),
        ripple: false,
    });
    let before = item(&bench, id);
    let (_, span) = doc_view::covered(&before).expect("a source");
    let mean = span.get() as f64 / before.len.get() as f64;
    let after = previewed(&bench, id, None);
    assert_eq!(after.len, before.len, "the length is kept");
    let command = speed_commands(id, &before, &after, false);
    assert_eq!(
        command,
        Some(Command::SetClipSpeed {
            item: id,
            speed: speed_from_decimal(mean).expect("a pace"),
            ripple: false,
        })
    );
    bench.apply(&command.expect("an edit"));
    assert_eq!(speed_graph(&item(&bench, id)), None);
    // Nothing changed is nothing to land.
    assert_eq!(speed_commands(id, &before, &before, false), None);
}

#[test]
fn a_curve_set_on_a_clip_is_given_back_as_drawn() {
    let point = |at, speed, slope| SpeedPoint { at, speed, slope };
    for drawn in [
        vec![point(0.0, 1.0, None), point(1.0, 2.0, None)],
        vec![
            point(0.0, 0.5, Some(0.0)),
            point(0.3, 3.0, Some(1.5)),
            point(1.0, 1.0, Some(0.0)),
        ],
    ] {
        let (mut bench, id) = placed();
        bench.apply(&Command::SetClipSpeedCurve {
            item: id,
            curve: Some(drawn.clone()),
            ripple: false,
        });
        assert_eq!(speed_graph(&item(&bench, id)), Some(drawn));
    }
    // A constant pace has no curve to give.
    let (bench, id) = placed();
    assert_eq!(speed_graph(&item(&bench, id)), None);
}
