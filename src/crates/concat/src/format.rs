// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Numbers into words and shapes: timecode, curves, sizes, and the drawn
//! waveform.

use crate::i18n::{t, tf};
use crate::ui::Bezier;

/// Solve a CSS cubic-bezier for y at a given x. This is the computation
/// Slint's expression language cannot express — it has no loops — so the
/// timing function is evaluated here and read back through `Curves.ease`.
///
/// The solve itself lives in the engine, where a keyed property's easing is
/// applied on every frame. Two implementations of one curve is how a preview
/// comes to disagree with an export invisibly, so there is only the one.
pub fn bezier_y_at_x(x1: f32, y1: f32, x2: f32, y2: f32, x: f32) -> f32 {
    concat_core::animate::bezier_y_at_x(
        f64::from(x1),
        f64::from(y1),
        f64::from(x2),
        f64::from(y2),
        f64::from(x),
    ) as f32
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

/// Where a bar goes hot: the peak level, as a fraction of full scale,
/// above which the part of the bar that is over it is drawn in the hot
/// colour. -6 dBFS. A mix that keeps its peaks under this has headroom; one
/// that crosses it often is on its way to clipping, which is the thing a
/// glance at the lane should catch.
pub const WAVE_HOT: f32 = 0.5012;

/// A drawn waveform: two sets of SVG path commands in a 1x1 box, drawn on
/// top of each other. See [`wave_path`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wave {
    /// Every column's bar, from the floor up to its peak or to
    /// [`WAVE_HOT`], whichever is lower.
    pub body: String,
    /// The part of each bar over [`WAVE_HOT`], for the columns that reach
    /// it; empty when none does.
    pub hot: String,
}

/// A waveform as SVG path commands in a 1x1 box: one column per slot, each
/// a bar standing on the floor, as tall as the loudest sample under it.
/// A level meter laid along the clip rather than the mirrored fish of a
/// waveform: the height of a bar is a level, read from one edge, and the
/// part of it above [`WAVE_HOT`] is a second path so it can be drawn in a
/// colour that says so.
///
/// Built from the engine's real peaks at the level that fits the column:
/// each column takes the extremes of the buckets under it, so a trim shows
/// the material it kept and a zoom shows the buckets it reveals. `columns`
/// is how many the drawing has room for - the clip's width in pixels, held
/// to a few thousand - and the paths are normalised so the Path that
/// renders them stretches the box onto the clip's current width.
pub fn wave_path(
    peaks: &concat_media::Pyramid,
    source_start: f32,
    duration: f32,
    gain: f32,
    columns: usize,
) -> Wave {
    /// Fewest columns worth drawing, and the most: enough that a clip a
    /// screen wide reads a column a pixel, few enough that the string stays
    /// under a few hundred kilobytes.
    const COLUMNS: std::ops::RangeInclusive<usize> = 8..=2048;
    /// Silence still draws a sliver: a hairline along the floor of a clip
    /// rather than a gap in it.
    const FLOOR: f32 = 0.024;

    if duration.is_nan() || duration <= 0.0 || peaks.finest().is_empty() {
        return Wave::default();
    }
    let columns = columns.clamp(*COLUMNS.start(), *COLUMNS.end());
    let gain = gain.max(0.0);
    let level = peaks.level_for(duration / columns as f32);
    let mut body = String::with_capacity(columns * 56);
    let mut hot = String::new();

    for column in 0..columns {
        let left = column as f32 / columns as f32;
        let right = (column + 1) as f32 / columns as f32;
        let (low, high) = level.extremes(
            source_start + left * duration,
            source_start + right * duration,
        );
        let amplitude = (high.max(-low) * gain).clamp(0.0, 1.0).max(FLOOR);
        let top = 1.0 - amplitude.min(WAVE_HOT);
        body.push_str(&format!(
            "M {left:.4} {top:.4} L {right:.4} {top:.4} \
             L {right:.4} 1.0000 L {left:.4} 1.0000 Z "
        ));
        if amplitude > WAVE_HOT {
            let (peak, limit) = (1.0 - amplitude, 1.0 - WAVE_HOT);
            hot.push_str(&format!(
                "M {left:.4} {peak:.4} L {right:.4} {peak:.4} \
                 L {right:.4} {limit:.4} L {left:.4} {limit:.4} Z "
            ));
        }
    }
    Wave { body, hot }
}

/// How many columns a clip `seconds` long gets at `seconds_per_pixel`: one
/// a pixel, rounded up to the next sixty-four so a zoom rebuilds the path
/// at each step of that and not at every pixel, held to what `wave_path`
/// draws.
pub fn wave_columns(seconds: f32, seconds_per_pixel: f32) -> usize {
    if seconds.is_nan() || seconds <= 0.0 || seconds_per_pixel.is_nan() || seconds_per_pixel <= 0.0
    {
        return 8;
    }
    let pixels = (seconds / seconds_per_pixel).ceil().max(1.0) as usize;
    pixels.div_ceil(64).max(1).saturating_mul(64).clamp(8, 2048)
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

/// What a person types into a colour field: `#rgb`, `#rgba`, `#rrggbb` or
/// `#rrggbbaa`, the hash optional, case ignored. `None` for anything else,
/// so the field can keep what it had rather than go black.
pub fn parse_colour(text: &str) -> Option<slint::Color> {
    let digits = text.trim().trim_start_matches('#');
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let nibble = |at: usize| u8::from_str_radix(digits.get(at..at + 1)?, 16).ok();
    let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
    match digits.len() {
        3 | 4 => {
            let wide = |at: usize| nibble(at).map(|n| n * 17);
            let alpha = if digits.len() == 4 { wide(3)? } else { 255 };
            Some(slint::Color::from_argb_u8(
                alpha,
                wide(0)?,
                wide(1)?,
                wide(2)?,
            ))
        }
        6 | 8 => {
            let alpha = if digits.len() == 8 { byte(6)? } else { 255 };
            Some(slint::Color::from_argb_u8(
                alpha,
                byte(0)?,
                byte(2)?,
                byte(4)?,
            ))
        }
        _ => None,
    }
}

/// "#rrggbb" when opaque, else "#rrggbbaa" - at zero too. For a colour
/// whose alpha is a dial of its own, like a stroke's: an opacity turned
/// down to nothing must not take the colour with it, or turning it back
/// up brings back black.
pub fn hex_rgba(colour: slint::Color) -> String {
    match colour.alpha() {
        255 => hex_of(colour),
        alpha => format!("{}{alpha:02x}", hex_of(colour)),
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
    fn a_typed_colour_is_read_in_every_length_and_refused_otherwise() {
        let lime = slint::Color::from_rgb_u8(0xcb, 0xf5, 0x3f);
        assert_eq!(parse_colour("#cbf53f"), Some(lime));
        assert_eq!(parse_colour("CBF53F"), Some(lime));
        assert_eq!(parse_colour("  #cbf53f  "), Some(lime));
        assert_eq!(
            parse_colour("#fff"),
            Some(slint::Color::from_rgb_u8(255, 255, 255))
        );
        assert_eq!(
            parse_colour("#f008"),
            Some(slint::Color::from_argb_u8(0x88, 255, 0, 0))
        );
        assert_eq!(
            parse_colour("#00000000"),
            Some(slint::Color::from_argb_u8(0, 0, 0, 0))
        );
        for junk in [
            "", "#", "#12", "#12345", "#1234567", "#ggg", "red", "#cbf53f9",
        ] {
            assert_eq!(parse_colour(junk), None, "{junk:?}");
        }
        // A stroke at zero opacity keeps its colour spelled.
        assert_eq!(
            hex_rgba(slint::Color::from_argb_u8(0, 0xcb, 0xf5, 0x3f)),
            "#cbf53f00"
        );
        assert_eq!(hex_rgba(lime), "#cbf53f");
    }

    #[test]
    fn a_waveform_has_one_column_per_slot_and_follows_the_gain() {
        let peaks = concat_media::Pyramid::of(concat_media::Peaks {
            min: vec![-0.5; 2000],
            max: vec![0.5; 2000],
            buckets_per_second: 1000.0,
        });
        let loud = wave_path(&peaks, 0.0, 2.0, 1.0, 128);
        let quiet = wave_path(&peaks, 0.0, 2.0, 0.25, 128);
        assert_eq!(loud.body.matches('M').count(), 128);
        // A bar stands on the floor: its top is one less its height.
        assert!(
            loud.body.contains(" 0.5000 L") && loud.body.contains(" 1.0000 Z"),
            "half amplitude at unity gain, floored: {}",
            loud.body
        );
        assert!(loud.hot.is_empty(), "half scale is under the hot line");
        assert!(
            quiet.body.contains(" 0.8750 L"),
            "an eighth at a quarter gain: {}",
            quiet.body
        );
        assert_eq!(wave_path(&peaks, 0.0, 0.0, 1.0, 128), Wave::default());
        assert_eq!(wave_path(&peaks, 0.0, f32::NAN, 1.0, 128), Wave::default());
        // The column count is held to what is worth drawing, either way.
        assert_eq!(
            wave_path(&peaks, 0.0, 2.0, 1.0, 0)
                .body
                .matches('M')
                .count(),
            8
        );
        assert_eq!(
            wave_path(&peaks, 0.0, 2.0, 1.0, 1_000_000)
                .body
                .matches('M')
                .count(),
            2048
        );
    }

    /// A bar over the hot line is two bars: the body stops at the line,
    /// and the hot path carries what is over it, and nothing else. A bar
    /// under it has no hot part; a gain can push it over.
    #[test]
    fn only_the_part_of_a_bar_over_the_hot_line_goes_hot() {
        let peaks = concat_media::Pyramid::of(concat_media::Peaks {
            min: vec![-0.8; 1000],
            max: vec![0.8; 1000],
            buckets_per_second: 1000.0,
        });
        let wave = wave_path(&peaks, 0.0, 1.0, 1.0, 10);
        let limit = format!("{:.4}", 1.0 - WAVE_HOT);
        assert!(
            wave.body.contains(&format!(" {limit} L")),
            "the body stops at the line: {}",
            wave.body
        );
        assert!(
            !wave.body.contains(" 0.2000 "),
            "and never reaches the peak"
        );
        assert_eq!(wave.hot.matches('M').count(), 10, "every column is over");
        assert!(
            wave.hot.contains(" 0.2000 L") && wave.hot.contains(&format!(" {limit} Z")),
            "the hot part runs from the peak down to the line: {}",
            wave.hot
        );

        let under = wave_path(&peaks, 0.0, 1.0, 0.5, 10);
        assert!(under.hot.is_empty(), "at half gain nothing is over");
        assert!(under.body.contains(" 0.6000 L"), "{}", under.body);
        let pushed = wave_path(&peaks, 0.0, 1.0, 2.0, 10);
        assert_eq!(
            pushed.hot.matches('M').count(),
            10,
            "a gain can push it over"
        );
        assert!(pushed.hot.contains(" 0.0000 L"), "clamped at full scale");
    }

    /// A zoomed-in clip reads the fine buckets: a single loud millisecond
    /// shows in one column at a column a millisecond, and is folded into
    /// its neighbours' column, still at full height, when a column is a
    /// tenth of a second.
    #[test]
    fn zooming_in_reveals_the_fine_buckets_and_never_loses_a_peak() {
        let mut min = vec![0.0; 1000];
        let mut max = vec![0.0; 1000];
        min[500] = -1.0;
        max[500] = 1.0;
        let peaks = concat_media::Pyramid::of(concat_media::Peaks {
            min,
            max,
            buckets_per_second: 1000.0,
        });
        // A full-scale spike is a column whose hot part reaches the top.
        let fine = wave_path(&peaks, 0.0, 1.0, 1.0, 1000);
        assert_eq!(
            fine.hot.matches('M').count(),
            1,
            "one column carries the spike: {}",
            fine.hot
        );
        assert!(fine.hot.contains(" 0.0000 L"), "at full height");
        let coarse = wave_path(&peaks, 0.0, 1.0, 1.0, 10);
        assert_eq!(
            coarse.hot.matches('M').count(),
            1,
            "the spike survives the fold, in one column"
        );
        assert!(coarse.hot.contains(" 0.0000 L"), "at full height");
        let trimmed = wave_path(&peaks, 0.6, 0.4, 1.0, 10);
        assert!(
            trimmed.hot.is_empty(),
            "a trim past the spike does not show it"
        );
    }

    #[test]
    fn columns_follow_the_zoom_in_steps_of_sixty_four() {
        assert_eq!(wave_columns(10.0, 0.05), 256, "200 px rounds up to 256");
        assert_eq!(wave_columns(10.0, 0.01), 1024);
        assert_eq!(
            wave_columns(600.0, 0.01),
            2048,
            "held to the most worth drawing"
        );
        assert_eq!(wave_columns(0.1, 0.05), 64, "never under a step");
        assert_eq!(wave_columns(0.0, 0.05), 8);
        assert_eq!(wave_columns(f32::NAN, 0.05), 8);
        assert_eq!(wave_columns(10.0, 0.0), 8);
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
