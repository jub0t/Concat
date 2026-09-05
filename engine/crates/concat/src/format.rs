// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Numbers into words and shapes: timecode, curves, sizes, and the drawn
//! waveform.

use crate::i18n::{t, tf};
use crate::ui::Bezier;

/// x of a cubic bezier with endpoints pinned at 0 and 1, at parameter `t`.
fn bezier_axis(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

fn bezier_axis_slope(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Solve a CSS cubic-bezier for y at a given x: Newton first, bisection as the
/// fallback where the curve is flat enough that Newton stalls. This is the
/// computation Slint's expression language cannot express — it has no loops —
/// so the timing function is evaluated here and read back through a global.
pub fn bezier_y_at_x(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    let mut t = x;

    for _ in 0..8 {
        let error = bezier_axis(x1, x2, t) - x;
        if error.abs() < 1e-5 {
            return bezier_axis(y1, y2, t);
        }
        let slope = bezier_axis_slope(x1, x2, t);
        if slope.abs() < 1e-6 {
            break;
        }
        t -= error / slope;
    }

    let (mut lo, mut hi) = (0.0_f32, 1.0_f32);
    t = x;
    for _ in 0..24 {
        let at = bezier_axis(x1, x2, t);
        if (at - x).abs() < 1e-5 {
            break;
        }
        if at > x {
            hi = t;
        } else {
            lo = t;
        }
        t = (lo + hi) / 2.0;
    }
    bezier_axis(y1, y2, t)
}

/// hh:mm:ss:ff, non-drop-frame, the way the ruler and the tray spell a
/// moment - see `Fmt.frames-timecode` in util.slint, which this mirrors so
/// the Details panel's duration reads like the readout beside it.
pub fn frames_timecode(seconds: f32, rate: f32) -> String {
    let rate = rate.round().max(1.0) as i64;
    let frames = (seconds.max(0.0) * rate as f32).floor() as i64;
    let whole = frames / rate;
    format!(
        "{:02}:{:02}:{:02}:{:02}",
        whole / 3600,
        (whole / 60) % 60,
        whole % 60,
        frames % rate
    )
}

/// "hh:mm:ss", "mm:ss" or "ss" -> seconds. Slint's string type has no split().
pub fn parse_timecode(text: &str) -> f32 {
    text.split(':')
        .rev()
        .enumerate()
        .map(|(index, part)| part.trim().parse::<f32>().unwrap_or(0.0) * 60_f32.powi(index as i32))
        .sum()
}

/// "hh:mm:ss:ff" -> frames. Short forms count from the right, so "12" is
/// twelve frames and "3:00" is three seconds — which is how anyone types into
/// a timecode field that is already showing them the shape.
pub fn parse_frames(text: &str, rate: f32) -> f32 {
    let fps = rate.round().max(1.0);
    let parts: Vec<f32> = text
        .split(':')
        .map(|part| part.trim().parse::<f32>().unwrap_or(0.0))
        .collect();
    let frames = parts.last().copied().unwrap_or(0.0);
    let seconds: f32 = parts
        .iter()
        .rev()
        .skip(1)
        .enumerate()
        .map(|(index, part)| part * 60_f32.powi(index as i32))
        .sum();
    (seconds * fps + frames).max(0.0)
}

/// "0.42, 0, 0.58, 1" -> Bezier, falling back to the current curve when the
/// text is not four numbers.
pub fn parse_bezier(text: &str, fallback: Bezier) -> Bezier {
    let parts: Vec<f32> = text
        .split(',')
        .filter_map(|part| part.trim().parse::<f32>().ok())
        .collect();
    match parts[..] {
        [x1, y1, x2, y2] => Bezier { x1, y1, x2, y2 },
        _ => fallback,
    }
}

/// The ruler's tick spacings, in seconds, finest to coarsest.
const TICKS: [f32; 16] = [
    1.0 / 30.0,
    0.1,
    0.25,
    0.5,
    1.0,
    2.0,
    5.0,
    10.0,
    15.0,
    30.0,
    60.0,
    120.0,
    300.0,
    600.0,
    1800.0,
    3600.0,
];

pub fn tick_interval(seconds_per_pixel: f32) -> f32 {
    TICKS
        .iter()
        .copied()
        .find(|interval| interval / seconds_per_pixel >= 90.0)
        .unwrap_or(3600.0)
}

// ─── the dialogs ────────────────────────────────────────────────────────────

pub fn bytes(count: f32) -> String {
    if count >= 1_000_000_000.0 {
        format!("{:.1} GB", count / 1_000_000_000.0)
    } else if count >= 1_000_000.0 {
        format!("{:.0} MB", count / 1_000_000.0)
    } else {
        format!("{:.0} KB", (count / 1_000.0).max(1.0))
    }
}

/// Seconds as a rough remaining time. Rough on purpose: a countdown to the
/// second on an estimate that is not accurate to the second is theatre.
pub fn eta(seconds: f32) -> String {
    if seconds <= 1.0 {
        t("almost done")
    } else if seconds < 60.0 {
        tf("{0}s left", &[&format!("{seconds:.0}")])
    } else {
        tf(
            "{0}m {1}s left",
            &[
                &format!("{:.0}", (seconds / 60.0).floor()),
                &format!("{:02.0}", seconds % 60.0),
            ],
        )
    }
}

pub fn hex_of(colour: slint::Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        colour.red(),
        colour.green(),
        colour.blue()
    )
}

/// The audio envelope, as SVG path commands in a 1x1 box.
///
/// Columns, not a curve. Each column is one rectangle, from the lowest to the
/// highest peak under it, so the waveform is a stepped silhouette with a flat
/// top on every column. Drawing the same peaks as a polyline gives a smooth,
/// rounded shape that reads as a graph of something rather than as audio.
///
/// Built from the engine's real peaks: each column takes the extremes of
/// the buckets that fall under it, so a trim shows the material it kept.
/// Normalised rather than drawn in pixels so a zoom costs nothing: the Path
/// that renders it stretches the box onto the clip's current width.
pub fn wave_path(
    peaks: &concat_media::Peaks,
    source_start: f32,
    duration: f32,
    gain: f32,
) -> String {
    /// Columns across the clip. Enough that the steps read as columns and
    /// not as a bar chart, few enough that the string stays a few kilobytes.
    const COLUMNS: usize = 128;
    /// Silence still draws a sliver: a hairline through the middle of a clip
    /// rather than a gap in it.
    const FLOOR: f32 = 0.012;

    if duration <= 0.0 || peaks.min.is_empty() || peaks.buckets_per_second <= 0.0 {
        return String::new();
    }
    let gain = gain.max(0.0);
    let count = peaks.min.len().min(peaks.max.len());
    let per_second = peaks.buckets_per_second;
    let mut path = String::with_capacity(COLUMNS * 56);

    for column in 0..COLUMNS {
        let left = column as f32 / COLUMNS as f32;
        let right = (column + 1) as f32 / COLUMNS as f32;
        let from = ((source_start + left * duration) * per_second)
            .floor()
            .max(0.0) as usize;
        let to = ((source_start + right * duration) * per_second)
            .ceil()
            .max(0.0) as usize;
        let (mut low, mut high) = (0.0f32, 0.0f32);
        for index in from..to.min(count).max(from) {
            if index < count {
                low = low.min(peaks.min[index]);
                high = high.max(peaks.max[index]);
            }
        }
        let amplitude = ((high.max(-low) * gain).clamp(0.0, 1.0) * 0.48).max(FLOOR);
        let (top, bottom) = (0.5 - amplitude, 0.5 + amplitude);
        path.push_str(&format!(
            "M {left:.4} {top:.4} L {right:.4} {top:.4} \
             L {right:.4} {bottom:.4} L {left:.4} {bottom:.4} Z "
        ));
    }
    path
}

/// A moment in the past, in the words a recents row wants: "just now",
/// "yesterday", "5 days ago".
pub fn when_phrase(opened_at_millis: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0);
    let seconds = now.saturating_sub(opened_at_millis) / 1000;
    let minutes = seconds / 60;
    let hours = minutes / 60;
    let days = hours / 24;
    if minutes < 2 {
        t("just now")
    } else if hours < 1 {
        tf("{0} minutes ago", &[&minutes])
    } else if days < 1 {
        t("today")
    } else if days == 1 {
        t("yesterday")
    } else if days < 30 {
        tf("{0} days ago", &[&days])
    } else {
        tf("{0} months ago", &[&(days / 30)])
    }
}

/// A Slint colour from a "#rrggbb" or "#rrggbbaa" string, or transparent
/// for anything else - which is how "no plate" is stored.
pub fn colour_of(hex: &str) -> slint::Color {
    let digits = hex.trim().trim_start_matches('#');
    let byte =
        |at: usize| u8::from_str_radix(digits.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0);
    match digits.len() {
        6 => slint::Color::from_rgb_u8(byte(0), byte(2), byte(4)),
        8 => slint::Color::from_argb_u8(byte(6), byte(0), byte(2), byte(4)),
        _ => slint::Color::from_argb_u8(0, 0, 0, 0),
    }
}

/// The inverse of [`colour_of`]: "#rrggbb", "#rrggbbaa" when translucent,
/// and an empty string for fully transparent.
pub fn hex_with_alpha(colour: slint::Color) -> String {
    match colour.alpha() {
        0 => String::new(),
        255 => hex_of(colour),
        alpha => format!("{}{alpha:02x}", hex_of(colour)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_round_trip() {
        let lime = colour_of("#cbf53f");
        assert_eq!(hex_of(lime), "#cbf53f");
        assert_eq!(hex_with_alpha(lime), "#cbf53f");
        assert_eq!(hex_with_alpha(colour_of("")), "");
        assert_eq!(hex_with_alpha(colour_of("#000000cc")), "#000000cc");
    }

    #[test]
    fn a_waveform_has_one_column_per_slot_and_follows_the_gain() {
        let peaks = concat_media::Peaks {
            min: vec![-0.5; 400],
            max: vec![0.5; 400],
            buckets_per_second: 200.0,
        };
        let loud = wave_path(&peaks, 0.0, 2.0, 1.0);
        let quiet = wave_path(&peaks, 0.0, 2.0, 0.25);
        assert_eq!(loud.matches('M').count(), 128);
        assert!(
            loud.contains("0.2600"),
            "half amplitude at unity gain: {loud}"
        );
        assert!(
            quiet.contains("0.4400"),
            "an eighth at a quarter gain: {quiet}"
        );
        assert!(wave_path(&peaks, 0.0, 0.0, 1.0).is_empty());
    }

    #[test]
    fn phrases_are_coarse() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as u64)
            .unwrap_or(0);
        assert_eq!(when_phrase(now), "just now");
        assert_eq!(when_phrase(now - 86_400_000 - 1000), "yesterday");
        assert_eq!(when_phrase(now - 3 * 86_400_000), "3 days ago");
    }
}
