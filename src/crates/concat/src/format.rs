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

/// A waveform as SVG path commands in a 1x1 box: one column per slot, each
/// a bar standing on the floor, as tall as the loudest sample under it.
/// A level meter laid along the clip rather than the mirrored fish of a
/// waveform: the height of a bar is a level, read from one edge.
///
/// Drawn at unity gain, always. The clip's volume is applied where the
/// path is drawn, as a scale on its height, so a drag on the volume knob
/// moves the picture without asking for a new path: at two thousand bars
/// a rebuild, a parse and a tessellation per knob tick was the lag in the
/// lane. The part of a bar over the hot line is drawn there too, as the
/// same bars again in the hot colour, clipped to the band above the line.
///
/// Built from the engine's real peaks at the level that fits the column:
/// each column takes the extremes of the buckets under it, so a trim shows
/// the material it kept and a zoom shows the buckets it reveals. `columns`
/// is how many the drawing has room for - the clip's width in pixels, held
/// to a few thousand - and the path is normalised so the Path that renders
/// it stretches the box onto the clip's current width.
pub fn wave_path(
    peaks: &concat_media::Pyramid,
    source_start: f32,
    duration: f32,
    columns: usize,
    bar: f32,
) -> String {
    /// Fewest columns worth drawing, and the most: enough that a clip a
    /// screen wide reads a column a pixel, few enough that the string stays
    /// under a few hundred kilobytes.
    const COLUMNS: std::ops::RangeInclusive<usize> = 8..=4096;
    /// Silence still draws a sliver: a hairline through the middle of a
    /// clip rather than a gap in it.
    const FLOOR: f32 = 0.024;

    if duration.is_nan() || duration <= 0.0 || peaks.finest().is_empty() {
        return String::new();
    }
    let columns = columns.clamp(*COLUMNS.start(), *COLUMNS.end());
    let level = peaks.level_for(duration / columns as f32);
    let mut path = String::with_capacity(columns * 56);

    // Each bar is the peak of everything under its pitch, drawn on the
    // leading `bar` of it: the rest is the gap that makes it a bar and
    // not a run of columns. Bars stand about the centre line, the way
    // every editor draws sound, so a beat reads as a spike both ways and
    // silence as a hairline through the middle (#105).
    let bar = bar.clamp(0.1, 1.0);
    let pitch = 1.0 / columns as f32;
    for column in 0..columns {
        let left = column as f32 * pitch;
        let (low, high) = level.extremes(
            source_start + left * duration,
            source_start + (left + pitch) * duration,
        );
        let right = left + pitch * bar;
        let amplitude = high.max(-low).clamp(0.0, 1.0).max(FLOOR);
        let top = 0.5 - amplitude / 2.0;
        let bottom = 0.5 + amplitude / 2.0;
        path.push_str(&format!(
            "M {left:.4} {top:.4} L {right:.4} {top:.4} \
             L {right:.4} {bottom:.4} L {left:.4} {bottom:.4} Z "
        ));
    }
    path
}

/// A lane bar's pitch on screen, in pixels: one bar, and the gap after
/// it, every three. Fewer bars than pixels by that factor, and a rebuild,
/// a parse and a tessellation a third the size: at a zoom where seconds
/// of audio sit under a pixel, a bar a pixel was work that drew nothing
/// a bar every three does not.
pub const WAVE_PITCH: f32 = 3.0;

/// How much of a lane bar's pitch the bar takes; the rest is the gap.
/// Two pixels of three: a thin candle with a pixel of air after it. The
/// bin's cards, at a few dozen bars across, keep a fuller bar of their
/// own; see `media_bin`.
pub const WAVE_BAR: f32 = 2.0 / 3.0;

/// How many bars a span of `seconds` gets at `seconds_per_pixel`: one a
/// [`WAVE_PITCH`], rounded up to the next sixteen so a zoom rebuilds the
/// path at each step of that and not at every pixel, held to what
/// `wave_path` draws. The span is the window of a clip on screen (see
/// `Studio::wave`), so the cap is a screen's worth and a bar is never
/// stretched past its pitch.
pub fn wave_columns(seconds: f32, seconds_per_pixel: f32) -> usize {
    if seconds.is_nan() || seconds <= 0.0 || seconds_per_pixel.is_nan() || seconds_per_pixel <= 0.0
    {
        return 8;
    }
    let pixels = (seconds / seconds_per_pixel).ceil().max(1.0);
    let bars = (pixels / WAVE_PITCH).ceil() as usize;
    bars.div_ceil(16).max(1).saturating_mul(16).clamp(8, 4096)
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
    fn a_waveform_has_one_centred_bar_per_column() {
        let peaks = concat_media::Pyramid::of(concat_media::Peaks {
            min: vec![-0.5; 2000],
            max: vec![0.5; 2000],
            buckets_per_second: 1000.0,
        });
        let wave = wave_path(&peaks, 0.0, 2.0, 128, WAVE_BAR);
        assert_eq!(wave.matches('M').count(), 128);
        // A bar stands about the centre: half amplitude runs from a
        // quarter of the way down to three quarters.
        assert!(
            wave.contains(" 0.2500 L") && wave.contains(" 0.7500 Z"),
            "half amplitude, centred: {wave}"
        );
        assert!(wave_path(&peaks, 0.0, 0.0, 128, WAVE_BAR).is_empty());
        assert!(wave_path(&peaks, 0.0, f32::NAN, 128, WAVE_BAR).is_empty());
        // The column count is held to what is worth drawing, either way.
        assert_eq!(
            wave_path(&peaks, 0.0, 2.0, 0, WAVE_BAR)
                .matches('M')
                .count(),
            8
        );
        assert_eq!(
            wave_path(&peaks, 0.0, 2.0, 1_000_000, WAVE_BAR)
                .matches('M')
                .count(),
            4096
        );
        // A bar takes `bar` of its pitch; the rest is the gap.
        let eight = wave_path(&peaks, 0.0, 2.0, 8, 0.75);
        assert!(eight.contains("M 0.0000 0.2500 L 0.0938 0.2500"), "{eight}");
        assert!(eight.contains("M 0.1250 0.2500 L 0.2188 0.2500"), "{eight}");
        let thin = wave_path(&peaks, 0.0, 2.0, 8, 0.5);
        assert!(thin.contains("M 0.0000 0.2500 L 0.0625 0.2500"), "{thin}");
        // Held to a sliver at the least, and never over the pitch.
        let hair = wave_path(&peaks, 0.0, 2.0, 8, 0.0);
        assert!(hair.contains("M 0.0000 0.2500 L 0.0125 0.2500"), "{hair}");
        let solid = wave_path(&peaks, 0.0, 2.0, 8, 7.0);
        assert!(solid.contains("M 0.0000 0.2500 L 0.1250 0.2500"), "{solid}");
        // Silence is a hairline through the middle, never a gap.
        let silence = concat_media::Pyramid::of(concat_media::Peaks {
            min: vec![0.0; 100],
            max: vec![0.0; 100],
            buckets_per_second: 100.0,
        });
        let flat = wave_path(&silence, 0.0, 1.0, 10, WAVE_BAR);
        assert_eq!(flat.matches('M').count(), 10);
        assert!(
            flat.contains(" 0.4880 L") && flat.contains(" 0.5120 Z"),
            "{flat}"
        );
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
        // A full-scale spike is a column whose bar reaches the top: a
        // closed shape with a corner at the top.
        let spikes = |path: &str| {
            path.split('Z')
                .filter(|bar| bar.contains(" 0.0000 L"))
                .count()
        };
        let fine = wave_path(&peaks, 0.0, 1.0, 1000, WAVE_BAR);
        assert_eq!(spikes(&fine), 1, "one column carries the spike: {fine}");
        let coarse = wave_path(&peaks, 0.0, 1.0, 10, WAVE_BAR);
        assert_eq!(
            spikes(&coarse),
            1,
            "the spike survives the fold, in one column"
        );
        let trimmed = wave_path(&peaks, 0.6, 0.4, 10, WAVE_BAR);
        assert!(
            !trimmed.contains(" 0.0000 L"),
            "a trim past the spike does not show it"
        );
    }

    #[test]
    fn bars_follow_the_zoom_in_steps_of_sixteen() {
        assert_eq!(
            wave_columns(10.0, 0.05),
            80,
            "200 px is 67 bars, rounds up to 80"
        );
        assert_eq!(wave_columns(10.0, 0.01), 336, "1000 px is 334 bars");
        assert_eq!(
            wave_columns(600.0, 0.01),
            4096,
            "held to the most worth drawing"
        );
        assert_eq!(wave_columns(0.1, 0.05), 16, "never under a step");
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
