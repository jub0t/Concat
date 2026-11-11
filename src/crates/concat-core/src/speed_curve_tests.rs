// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How a speed curve maps a clip's timeline onto its source: the speed at
//! each point, the source consumed up to it, the slope, the mean, and the
//! constant-rate pieces sound is played in. Every expectation is a sample
//! of the curve at a known fraction of the clip.

use crate::SpeedCurve;
use crate::time::Rational;
use crate::timeline::{Clip, MediaRef};

const TIGHT: f64 = 1e-9;

fn close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < TIGHT,
        "{what}: got {actual}, expected {expected}"
    );
}

/// The fractions of the clip every curve below is sampled at.
const XS: [f64; 9] = [0.0, 0.1, 0.2, 0.25, 0.4, 0.5, 0.7, 0.9, 1.0];

fn sample(curve: &SpeedCurve, speeds: [f64; 9], consumed: [f64; 9], slopes: [f64; 9]) {
    for (index, &x) in XS.iter().enumerate() {
        close(curve.speed_at(x), speeds[index], &format!("speed at {x}"));
        close(
            curve.consumed(x),
            consumed[index],
            &format!("consumed at {x}"),
        );
        close(curve.slope_at(x), slopes[index], &format!("slope at {x}"));
    }
}

/// Half speed up to 4x at 40% and down to 1x at the end.
fn rise_and_fall() -> [(f64, f64); 3] {
    [(0.0, 0.5), (0.4, 4.0), (1.0, 1.0)]
}

#[test]
fn straight_lines_interpolate_linearly_and_integrate_as_trapezoids() {
    let curve = SpeedCurve::new(&rise_and_fall()).unwrap();
    assert!(curve.slopes().is_none(), "no slopes, straight lines");
    sample(
        &curve,
        [0.5, 1.375, 2.25, 2.6875, 4.0, 3.5, 2.5, 1.5, 1.0],
        [
            0.0, 0.09375, 0.275, 0.3984375, 0.9, 1.275, 1.875, 2.275, 2.4,
        ],
        // A point's slope is the stretch that ends there.
        [8.75, 8.75, 8.75, 8.75, 8.75, -5.0, -5.0, -5.0, -5.0],
    );
    close(curve.mean(), 2.4, "mean");
}

#[test]
fn zero_slopes_ease_in_and_out_of_every_point_with_the_same_area() {
    let curve = SpeedCurve::with_slopes(&rise_and_fall(), Some(&[0.0, 0.0, 0.0])).unwrap();
    assert_eq!(curve.slopes(), Some(&[0.0, 0.0, 0.0][..]));
    sample(
        &curve,
        [
            0.5,
            1.046875,
            2.25,
            2.892578125,
            4.0,
            3.777_777_777_777_778,
            2.5,
            1.222_222_222_222_222,
            1.0,
        ],
        [
            0.0,
            0.069140625,
            0.23125,
            0.359_985_351_562_5,
            0.9,
            1.292_361_111_111_111,
            1.93125,
            2.292_361_111_111_111,
            2.4,
        ],
        [
            0.0,
            9.84375,
            13.125,
            12.3046875,
            0.0,
            -4.166_666_666_666_667,
            -7.5,
            -4.166_666_666_666_667,
            0.0,
        ],
    );
    // An ease with flat ends is symmetric about the straight line: the
    // same source is consumed by the end of each stretch.
    close(curve.mean(), 2.4, "mean");
}

#[test]
fn given_slopes_bend_the_stretch_and_a_peak_is_flattened() {
    // The middle point is a peak: its slope is held to zero, as the
    // second stretch falls. The outer slopes are within the monotone
    // limit and kept.
    let curve = SpeedCurve::with_slopes(
        &[(0.0, 1.0), (0.5, 3.0), (1.0, 2.0)],
        Some(&[10.0, 2.0, -1.0]),
    )
    .unwrap();
    let slopes = curve.slopes().unwrap();
    close(slopes[0], 10.0, "first slope");
    close(slopes[1], 0.0, "peak slope");
    close(slopes[2], -1.0, "last slope");
    sample(
        &curve,
        [1.0, 1.848, 2.424, 2.625, 2.952, 3.0, 2.696, 2.168, 2.0],
        [
            0.0,
            0.144_866_666_666_667,
            0.360_533_333_333_333,
            0.486_979_166_666_667,
            0.909_866_666_666_667,
            1.208_333_333_333_333,
            1.786_466_666_666_667,
            2.2718,
            2.479_166_666_666_667,
        ],
        [10.0, 7.04, 4.56, 3.5, 1.04, 0.0, -2.56, -2.24, -1.0],
    );
    close(curve.mean(), 2.479_166_666_666_667, "mean");
}

#[test]
fn slopes_too_steep_for_the_rise_are_scaled_to_the_monotone_limit() {
    // 40 against a rise of 6 per unit is far past three times the rise:
    // the pair is scaled until its length is three rises.
    let curve = SpeedCurve::with_slopes(
        &[(0.0, 1.0), (0.5, 4.0), (1.0, 1.0)],
        Some(&[40.0, 40.0, -40.0]),
    )
    .unwrap();
    let slopes = curve.slopes().unwrap();
    close(slopes[0], 9.0 * 2f64.sqrt(), "first slope, 3/sqrt(2) rises");
    close(slopes[1], 0.0, "the peak");
    close(slopes[2], -18.0, "last slope, three rises");
    sample(
        &curve,
        [
            1.0,
            2.126_587_011_927,
            2.972_410_388_418,
            3.295_495_128_835,
            3.891_646_752_982,
            4.0,
            3.808,
            2.464,
            1.0,
        ],
        [
            0.0,
            0.158_741_839_764,
            0.415_958_614_538,
            0.572_925_967_025,
            1.118_752_553_777,
            1.515_165_042_945,
            2.305_565_042_945,
            2.961_565_042_945,
            3.140_165_042_945,
        ],
        [
            12.727_922_061_358,
            9.832_935_059_635,
            7.112_649_352_637,
            5.818_019_484_661,
            2.196_181_822_82,
            0.0,
            -2.88,
            -11.52,
            -18.0,
        ],
    );
}

#[test]
fn the_curve_never_leaves_the_speeds_it_joins() {
    let curve = SpeedCurve::with_slopes(
        &[(0.0, 0.5), (0.3, 8.0), (0.6, 0.25), (1.0, 2.0)],
        Some(&[50.0, -50.0, 50.0, -50.0]),
    )
    .unwrap();
    for step in 0..=1000 {
        let x = f64::from(step) / 1000.0;
        let speed = curve.speed_at(x);
        assert!(
            (0.25 - TIGHT..=8.0 + TIGHT).contains(&speed),
            "{speed} at {x}"
        );
    }
}

#[test]
fn the_area_matches_a_fine_numeric_sum_for_both_joins() {
    let points = [(0.0, 0.3), (0.2, 5.0), (0.45, 1.0), (0.8, 3.0), (1.0, 0.7)];
    let straight = SpeedCurve::new(&points).unwrap();
    let smooth = SpeedCurve::with_slopes(&points, Some(&[0.0, 4.0, -1.0, 2.0, 0.0])).unwrap();
    for curve in [straight, smooth] {
        for end in [0.13, 0.45, 0.77, 1.0] {
            let steps = 40_000;
            let width = end / f64::from(steps);
            let sum: f64 = (0..steps)
                .map(|step| curve.speed_at((f64::from(step) + 0.5) * width) * width)
                .sum();
            assert!((curve.consumed(end) - sum).abs() < 1e-7, "area to {end}");
        }
    }
}

#[test]
fn points_off_the_ends_are_anchored_by_holding_the_end_speeds() {
    let curve = SpeedCurve::with_slopes(&[(0.25, 2.0), (0.75, 0.5)], Some(&[0.0, 0.0])).unwrap();
    assert_eq!(
        curve.points(),
        &[(0.0, 2.0), (0.25, 2.0), (0.75, 0.5), (1.0, 0.5)]
    );
    assert_eq!(curve.slopes(), Some(&[0.0, 0.0, 0.0, 0.0][..]));
    sample(
        &curve,
        [2.0, 2.0, 2.0, 2.0, 1.676, 1.25, 0.542, 0.5, 0.5],
        [
            0.0, 0.2, 0.4, 0.5, 0.7827875, 0.9296875, 1.0992875, 1.2, 1.25,
        ],
        [0.0, 0.0, 0.0, 0.0, -3.78, -4.5, -1.62, 0.0, 0.0],
    );
    close(curve.mean(), 1.25, "mean");
}

#[test]
fn one_point_is_a_flat_curve_at_its_speed() {
    let curve = SpeedCurve::new(&[(0.5, 3.0)]).unwrap();
    assert_eq!(curve.points(), &[(0.0, 3.0), (0.5, 3.0), (1.0, 3.0)]);
    close(curve.consumed(0.3), 0.9, "consumed");
    close(curve.mean(), 3.0, "mean");
}

#[test]
fn tidying_sorts_clamps_and_drops_what_is_not_a_number() {
    assert!(SpeedCurve::new(&[]).is_none());
    assert!(SpeedCurve::new(&[(f64::NAN, 1.0), (0.5, f64::INFINITY)]).is_none());
    let curve = SpeedCurve::new(&[(0.8, 50.0), (f64::NAN, 2.0), (0.3, 0.01)]).unwrap();
    assert_eq!(
        curve.points(),
        &[(0.0, 0.0625), (0.3, 0.0625), (0.8, 16.0), (1.0, 16.0)]
    );
    close(curve.mean(), 7.234_375, "mean");
    // Out of the unit interval: clamped onto its ends.
    let curve = SpeedCurve::new(&[(-3.0, 2.0), (4.0, 1.0)]).unwrap();
    assert_eq!(curve.points(), &[(0.0, 2.0), (1.0, 1.0)]);
}

#[test]
fn slopes_count_only_when_every_point_has_one() {
    let points = [(0.0, 1.0), (0.5, 2.0), (1.0, 1.0)];
    assert!(
        SpeedCurve::with_slopes(&points, Some(&[0.0, 0.0]))
            .unwrap()
            .slopes()
            .is_none()
    );
    assert!(
        SpeedCurve::with_slopes(&points, None)
            .unwrap()
            .slopes()
            .is_none()
    );
    // A slope that is not a number is a flat one.
    let curve = SpeedCurve::with_slopes(&points, Some(&[f64::NAN, 0.0, 0.0])).unwrap();
    close(curve.slopes().unwrap()[0], 0.0, "flat slope");
    close(curve.speed_at(0.25), 1.5, "the smoothstep's midpoint");
}

#[test]
fn a_step_holds_the_earlier_speed_on_itself_and_has_no_slope() {
    let curve = SpeedCurve::new(&[(0.0, 0.5), (0.5, 0.5), (0.5, 2.0), (1.0, 2.0)]).unwrap();
    close(curve.speed_at(0.25), 0.5, "before");
    close(curve.speed_at(0.5), 0.5, "on the step");
    close(curve.speed_at(0.500_001), 2.0, "just after");
    close(curve.consumed(0.5), 0.25, "consumed to the step");
    close(curve.consumed(0.75), 0.75, "consumed past it");
    close(curve.mean(), 1.25, "mean");
    for x in [0.25, 0.5, 0.75] {
        close(curve.slope_at(x), 0.0, "flat either side");
    }
}

#[test]
fn outside_the_clip_the_end_speeds_hold_and_the_slope_is_zero() {
    let curve = SpeedCurve::new(&rise_and_fall()).unwrap();
    close(curve.speed_at(-1.0), 0.5, "before");
    close(curve.speed_at(2.0), 1.0, "after");
    close(curve.slope_at(-0.1), 0.0, "slope before");
    close(curve.slope_at(1.1), 0.0, "slope after");
    close(curve.consumed(-1.0), 0.0, "nothing consumed before");
    close(curve.consumed(3.0), 2.4, "everything by the end");
}

#[test]
fn the_mean_is_floored_at_the_slowest_speed() {
    let curve = SpeedCurve::new(&[(0.0, 0.0625), (1.0, 0.0625)]).unwrap();
    close(curve.mean(), SpeedCurve::MIN_SPEED, "mean");
    assert_eq!(SpeedCurve::MIN_SPEED, 0.0625);
    assert_eq!(SpeedCurve::MAX_SPEED, 16.0);
}

#[test]
fn pieces_are_equal_stretches_at_their_mean_speed() {
    let curve = SpeedCurve::new(&rise_and_fall()).unwrap();
    let pieces = curve.pieces(5);
    let expected = [
        (0.0, 0.2, 0.0, 1.375),
        (0.2, 0.4, 0.275, 3.125),
        (0.4, 0.6, 0.9, 3.5),
        (0.6, 0.8, 1.6, 2.5),
        (0.8, 1.0, 2.1, 1.5),
    ];
    assert_eq!(pieces.len(), 5);
    for (piece, want) in pieces.iter().zip(expected) {
        close(piece.0, want.0, "from");
        close(piece.1, want.1, "to");
        close(piece.2, want.2, "consumed at the start");
        close(piece.3, want.3, "mean speed");
    }
    // Asked for none, one piece.
    let one = curve.pieces(0);
    assert_eq!(one.len(), 1);
    close(one[0].3, 2.4, "one piece at the mean");
}

fn clip_with(curve: Option<SpeedCurve>, start: i64, length: i64, source: i64) -> Clip {
    let mut clip = Clip::new(
        MediaRef::new("ramp.mp4"),
        Rational::from_int(start),
        Rational::from_int(length),
    );
    clip.source_start = Rational::from_int(source);
    clip.retime = curve;
    clip
}

#[test]
fn a_clip_under_a_curve_finds_its_source_by_the_area() {
    // Ten seconds from 3 s on the timeline, from 7 s into its file.
    let clip = clip_with(SpeedCurve::new(&rise_and_fall()), 3, 10, 7);
    for (index, &x) in XS.iter().enumerate().take(8) {
        let time = Rational::approximate(3.0 + 10.0 * x).unwrap();
        let expected = [0.0, 0.09375, 0.275, 0.3984375, 0.9, 1.275, 1.875, 2.275][index];
        let source = clip.source_time_at(time).unwrap().as_f64();
        assert!(
            (source - (7.0 + 10.0 * expected)).abs() < 1e-6,
            "at {x}: {source}"
        );
    }
    assert!(
        clip.source_time_at(Rational::from_int(13)).is_none(),
        "the end is past it"
    );
    assert!(clip.source_time_at(Rational::from_int(2)).is_none());
    assert!((clip.source_duration().as_f64() - 24.0).abs() < 1e-6);
    assert!(!clip.is_paced(), "every frame is sought by its own time");
}

#[test]
fn a_clip_without_a_curve_is_paced_at_its_rate() {
    let mut clip = clip_with(None, 0, 4, 1);
    clip.speed = Rational::from_int(2);
    assert!(clip.is_paced());
    assert_eq!(
        clip.source_time_at(Rational::from_int(3)),
        Some(Rational::from_int(7))
    );
}
