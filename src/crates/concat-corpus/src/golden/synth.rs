// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Pictures made up for the tests, and the measurements the default
//! bounds were set from.
//!
//! One scene, drawn from its description at any sub-pixel offset and
//! turned into eight bits in any of the ways two honest renderers differ:
//! rounded or truncated, dithered or not, its edges covered finely or
//! coarsely. It has what a golden has: a smooth gradient, flat fields,
//! antialiased round and straight edges, and fine detail. Against it are
//! the changes a golden exists to catch: the picture moved, a speck, a
//! field gone off its hue, the whole a little dimmer.
//!
//! `measure` prints what each of them measures. It is ignored, being a
//! report and not a check; run it when the bounds are in question:
//!
//! ```text
//! cargo test -p concat-corpus --lib golden::synth -- --ignored --nocapture
//! ```

use super::colour;
use super::flip;
use super::picture::Rgba8Image;

/// The size every case's frame is.
pub const WIDTH: u32 = 320;
/// See [`WIDTH`].
pub const HEIGHT: u32 = 180;

/// How a scene's real-valued levels become eight bits.
#[derive(Clone, Copy)]
pub enum Quantise {
    /// To the nearest level.
    Round,
    /// Down, as a conversion that forgets to round does.
    Truncate,
    /// To the nearest after a 4 × 4 ordered dither of half a level.
    Dither,
}

/// How a scene is drawn.
#[derive(Clone, Copy)]
pub struct Draw {
    /// The whole scene's offset, in pixels.
    pub offset: (f64, f64),
    /// Samples a side taken of each pixel: its edges' coverage comes in
    /// steps of one over this, squared.
    pub samples: u32,
    /// How the levels are made eight bits.
    pub quantise: Quantise,
    /// What every level is multiplied by: one for the scene as it is.
    pub gain: f64,
}

impl Default for Draw {
    fn default() -> Self {
        Self {
            offset: (0.0, 0.0),
            samples: 4,
            quantise: Quantise::Round,
            gain: 1.0,
        }
    }
}

/// The scene's colour at a point, in levels: a gradient with flat fields,
/// discs, a slanted bar, and stripes two pixels wide on it.
fn shade(x: f64, y: f64) -> [f64; 3] {
    let (w, h) = (f64::from(WIDTH), f64::from(HEIGHT));
    let mut colour = [
        60.0 + 120.0 * x / w,
        70.0 + 100.0 * y / h,
        150.0 - 60.0 * x / w,
    ];
    // Two flat fields, as a title's backing plate or a solid is.
    if (24.0..120.0).contains(&x) && (100.0..160.0).contains(&y) {
        colour = [40.0, 44.0, 52.0];
    }
    if (200.0..296.0).contains(&x) && (20.0..68.0).contains(&y) {
        colour = [150.0, 120.0, 90.0];
    }
    // Discs: round edges at every angle.
    for (cx, cy, radius, fill) in [
        (80.0, 50.0, 30.0, [235.0, 235.0, 235.0]),
        (170.0, 110.0, 22.0, [220.0, 60.0, 50.0]),
        (60.0, 130.0, 14.0, [240.0, 200.0, 40.0]),
    ] {
        if (x - cx).hypot(y - cy) < radius {
            colour = fill;
        }
    }
    // A slanted bar: a straight edge off the pixel grid.
    let along = (x - 130.0) * 0.94 + (y - 20.0) * 0.34;
    let across = -(x - 130.0) * 0.34 + (y - 20.0) * 0.94;
    if (0.0..150.0).contains(&along) && (0.0..7.0).contains(&across) {
        colour = [250.0, 250.0, 250.0];
    }
    // Stripes two pixels wide: detail as fine as text's stems.
    if (210.0..300.0).contains(&x) && (110.0..160.0).contains(&y) && (x / 2.0).floor() % 2.0 == 0.0
    {
        colour = [20.0, 20.0, 24.0];
    }
    colour
}

/// Draws the scene.
pub fn scene(draw: Draw) -> Rgba8Image {
    const BAYER: [[f64; 4]; 4] = [
        [0.0, 8.0, 2.0, 10.0],
        [12.0, 4.0, 14.0, 6.0],
        [3.0, 11.0, 1.0, 9.0],
        [15.0, 7.0, 13.0, 5.0],
    ];
    let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    let samples = draw.samples.max(1);
    for py in 0..HEIGHT {
        for px in 0..WIDTH {
            let mut sum = [0.0; 3];
            for sy in 0..samples {
                for sx in 0..samples {
                    let x = f64::from(px) + (f64::from(sx) + 0.5) / f64::from(samples);
                    let y = f64::from(py) + (f64::from(sy) + 0.5) / f64::from(samples);
                    let colour = shade(x - draw.offset.0, y - draw.offset.1);
                    for (total, level) in sum.iter_mut().zip(colour) {
                        *total += level;
                    }
                }
            }
            for total in sum {
                let level = total / f64::from(samples * samples) * draw.gain;
                let level = match draw.quantise {
                    Quantise::Round => level.round(),
                    Quantise::Truncate => level.floor(),
                    Quantise::Dither => {
                        let nudge = (BAYER[(py % 4) as usize][(px % 4) as usize] + 0.5) / 16.0;
                        (level + nudge - 0.5).round()
                    }
                };
                pixels.push(level.clamp(0.0, 255.0) as u8);
            }
            pixels.push(255);
        }
    }
    Rgba8Image::new(WIDTH, HEIGHT, pixels).unwrap()
}

/// A flat picture of one colour.
pub fn flat(width: u32, height: u32, rgb: [u8; 3]) -> Rgba8Image {
    let pixels = [rgb[0], rgb[1], rgb[2], 255].repeat((width * height) as usize);
    Rgba8Image::new(width, height, pixels).unwrap()
}

/// `picture` with `change` made to the colour of every pixel in the
/// rectangle at (`x`, `y`), `width` × `height`.
pub fn with_rect(
    picture: &Rgba8Image,
    (x, y, width, height): (u32, u32, u32, u32),
    change: impl Fn([u8; 3]) -> [u8; 3],
) -> Rgba8Image {
    let mut pixels = picture.pixels().to_vec();
    for py in y..(y + height).min(picture.height()) {
        for px in x..(x + width).min(picture.width()) {
            let at = ((py * picture.width() + px) * 4) as usize;
            let rgb = change([pixels[at], pixels[at + 1], pixels[at + 2]]);
            pixels[at..at + 3].copy_from_slice(&rgb);
        }
    }
    Rgba8Image::new(picture.width(), picture.height(), pixels).unwrap()
}

/// `picture` with every channel of about one pixel in `one_in` moved by up
/// to `levels` either way, the same way every time for the same `seed`.
pub fn with_noise(picture: &Rgba8Image, levels: i16, one_in: u32, seed: u64) -> Rgba8Image {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = move || {
        // xorshift64*: enough for noise, and the same on every machine.
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        (state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) as u32
    };
    let mut pixels = picture.pixels().to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        if next() % one_in != 0 {
            continue;
        }
        for level in &mut pixel[..3] {
            let nudge = (next() % (2 * levels as u32 + 1)) as i16 - levels;
            *level = (i16::from(*level) + nudge).clamp(0, 255) as u8;
        }
    }
    Rgba8Image::new(picture.width(), picture.height(), pixels).unwrap()
}

/// What two pictures of one size measure: FLIP's mean, its percentile,
/// its largest error, and the worst block's ΔE2000.
pub fn measure_pair(reference: &Rgba8Image, got: &Rgba8Image) -> [f64; 4] {
    let (a, b) = (reference.seen(), got.seen());
    let flip = flip::flip(&a, &b, reference.width(), reference.height());
    let largest = flip.map.iter().copied().fold(0.0_f32, f32::max);
    let block = colour::worst_block(&a, &b, reference.width(), reference.height());
    [
        flip.mean,
        flip.percentile,
        f64::from(largest),
        block.delta_e,
    ]
}

/// The largest ΔE2000 a move of one level in every channel, each either
/// way, makes anywhere in sRGB: what a flat field rounded the other way
/// can cost. Sampled every fourth level.
fn worst_single_level_step() -> (f64, [u8; 3], [u8; 3]) {
    let mut worst = (0.0, [0; 3], [0; 3]);
    for r in (0..=255_u8).step_by(4) {
        for g in (0..=255_u8).step_by(4) {
            for b in (0..=255_u8).step_by(4) {
                let from = colour::srgb_to_lab([r, g, b]);
                for step in 1..27_i32 {
                    let moved = [
                        (i32::from(r) + step % 3 - 1).clamp(0, 255) as u8,
                        (i32::from(g) + (step / 3) % 3 - 1).clamp(0, 255) as u8,
                        (i32::from(b) + (step / 9) % 3 - 1).clamp(0, 255) as u8,
                    ];
                    let delta_e = colour::delta_e_2000(from, colour::srgb_to_lab(moved));
                    if delta_e > worst.0 {
                        worst = (delta_e, [r, g, b], moved);
                    }
                }
            }
        }
    }
    worst
}

/// The scene's lighter flat field, the slanted bar across it included.
pub const FIELD: (u32, u32, u32, u32) = (200, 20, 96, 48);
/// The field's own colour.
pub const FIELD_COLOUR: [u8; 3] = [150, 120, 90];
/// One 8 x 8 block of the field, clear of the bar and on the grid the
/// colour gate's blocks are cut on.
pub const PATCH: (u32, u32, u32, u32) = (208, 24, 8, 8);

/// A change that gives the field's own colour, wherever it is, the colour
/// `to`, and leaves the bar's pixels and its edge's alone.
pub fn turn(to: [u8; 3]) -> impl Fn([u8; 3]) -> [u8; 3] {
    move |colour| if colour == FIELD_COLOUR { to } else { colour }
}

#[test]
#[ignore = "a report of what the bounds were set from, not a check"]
fn measure() {
    let reference = scene(Draw::default());
    let draw = |change: fn(&mut Draw)| {
        let mut draw = Draw::default();
        change(&mut draw);
        scene(draw)
    };
    let rows: Vec<(&str, Rgba8Image)> = vec![
        ("-- two honest renders of one scene --", reference.clone()),
        (
            "truncated, not rounded",
            draw(|d| d.quantise = Quantise::Truncate),
        ),
        (
            "dithered by half a level",
            draw(|d| d.quantise = Quantise::Dither),
        ),
        (
            "noise of 1 level, every pixel",
            with_noise(&reference, 1, 1, 7),
        ),
        (
            "noise of 1 level, one pixel in 10",
            with_noise(&reference, 1, 10, 7),
        ),
        (
            "noise of 2 levels, every pixel",
            with_noise(&reference, 2, 1, 7),
        ),
        (
            "every channel up 1 level",
            with_rect(&reference, (0, 0, WIDTH, HEIGHT), |c| {
                c.map(|l| l.saturating_add(1))
            }),
        ),
        (
            "every channel up 2 levels",
            with_rect(&reference, (0, 0, WIDTH, HEIGHT), |c| {
                c.map(|l| l.saturating_add(2))
            }),
        ),
        ("edges covered in 4ths, not 16ths", draw(|d| d.samples = 2)),
        ("edges covered in 64ths, not 16ths", draw(|d| d.samples = 8)),
        ("edges not antialiased at all", draw(|d| d.samples = 1)),
        (
            "moved 1/256 px",
            draw(|d| d.offset = (1.0 / 256.0, 1.0 / 256.0)),
        ),
        (
            "moved 1/64 px",
            draw(|d| d.offset = (1.0 / 64.0, 1.0 / 64.0)),
        ),
        (
            "moved 1/16 px",
            draw(|d| d.offset = (1.0 / 16.0, 1.0 / 16.0)),
        ),
        ("-- what a golden is there to catch --", reference.clone()),
        ("moved 1/4 px", draw(|d| d.offset = (0.25, 0.25))),
        ("moved 1/2 px", draw(|d| d.offset = (0.5, 0.5))),
        ("moved 1 px", draw(|d| d.offset = (1.0, 0.0))),
        ("moved 2 px", draw(|d| d.offset = (2.0, 0.0))),
        ("2% dimmer", draw(|d| d.gain = 0.98)),
        ("5% dimmer", draw(|d| d.gain = 0.95)),
        (
            "a 3 x 3 white speck",
            with_rect(&reference, (250, 90, 3, 3), |_| [255, 255, 255]),
        ),
        (
            "a 2 x 2 white speck",
            with_rect(&reference, (250, 90, 2, 2), |_| [255, 255, 255]),
        ),
        (
            "one white pixel",
            with_rect(&reference, (250, 90, 1, 1), |_| [255, 255, 255]),
        ),
        (
            "one white pixel on a dark field",
            with_rect(&reference, (100, 110, 1, 1), |_| [255, 255, 255]),
        ),
        (
            "a field's hue turned, lightness kept",
            with_rect(&reference, FIELD, turn([153, 118, 101])),
        ),
        (
            "a field's hue turned a little, lightness kept",
            with_rect(&reference, FIELD, turn([151, 119, 98])),
        ),
        (
            "a field 4 levels redder",
            with_rect(&reference, FIELD, turn([154, 120, 90])),
        ),
        (
            "an 8 x 8 patch 4 levels redder",
            with_rect(&reference, PATCH, turn([154, 120, 90])),
        ),
        (
            "-- the same, where FLIP's pooling loses them --",
            reference.clone(),
        ),
        (
            "patch 6 redder, under noise of 1",
            with_rect(&with_noise(&reference, 1, 1, 7), PATCH, |c| {
                [c[0] + 6, c[1], c[2]]
            }),
        ),
        (
            "patch 8 redder, under noise of 1",
            with_rect(&with_noise(&reference, 1, 1, 7), PATCH, |c| {
                [c[0] + 8, c[1], c[2]]
            }),
        ),
        (
            "patch 8 redder, under noise of 2",
            with_rect(&with_noise(&reference, 2, 1, 7), PATCH, |c| {
                [c[0] + 8, c[1], c[2]]
            }),
        ),
        (
            "patch 10 redder, under noise of 2",
            with_rect(&with_noise(&reference, 2, 1, 7), PATCH, |c| {
                [c[0] + 10, c[1], c[2]]
            }),
        ),
        (
            "patch hue turned a little, under noise of 2",
            with_rect(&with_noise(&reference, 2, 1, 7), PATCH, |c| {
                [c[0] + 1, c[1] - 1, c[2] + 8]
            }),
        ),
        (
            "patch hue turned, under noise of 2",
            with_rect(&with_noise(&reference, 2, 1, 7), PATCH, |c| {
                [c[0] + 3, c[1] - 2, c[2] + 11]
            }),
        ),
        (
            "field hue turned a little, edges in 4ths",
            with_rect(&draw(|d| d.samples = 2), FIELD, turn([151, 119, 98])),
        ),
        (
            "field 6 redder, edges in 4ths",
            with_rect(&draw(|d| d.samples = 2), FIELD, turn([156, 120, 90])),
        ),
        (
            "field 8 redder, edges in 4ths",
            with_rect(&draw(|d| d.samples = 2), FIELD, turn([158, 120, 90])),
        ),
        (
            "the disc gone",
            with_rect(&reference, (148, 88, 44, 44), |_| [120, 120, 120]),
        ),
    ];
    println!(
        "{:<46} {:>9} {:>9} {:>9} {:>8}",
        "", "FLIP mean", "FLIP p99", "FLIP max", "dE00"
    );
    for (name, got) in &rows {
        if name.starts_with("--") {
            println!("{name}");
            continue;
        }
        let [mean, percentile, largest, delta_e] = measure_pair(&reference, got);
        println!("{name:<46} {mean:>9.5} {percentile:>9.5} {largest:>9.5} {delta_e:>8.3}");
    }
    let (delta_e, from, to) = worst_single_level_step();
    println!(
        "the worst ΔE2000 of one level a channel anywhere in sRGB: {delta_e:.3}, {from:?} to {to:?}"
    );
    // Flat fields of every kind, each a level up in every channel, and
    // each a level out in the direction that costs most.
    println!("flat fields, one level a channel:");
    for floor in [0_u8, 16, 32, 64] {
        let mut up = (0.0_f64, 0.0_f64, [0_u8; 3]);
        let mut mixed = (0.0_f64, 0.0_f64, [0_u8; 3]);
        for r in (floor..=250).step_by(26) {
            for g in (floor..=250).step_by(26) {
                for b in (floor..=250).step_by(26) {
                    let field = flat(32, 32, [r, g, b]);
                    let [flip, _, _, delta_e] =
                        measure_pair(&field, &flat(32, 32, [r + 1, g + 1, b + 1]));
                    if flip > up.0 {
                        up = (flip, delta_e, [r, g, b]);
                    }
                    for (dr, dg, db) in
                        [(1, -1, 1), (-1, 1, -1), (1, 1, -1), (-1, 1, 1), (1, -1, -1)]
                    {
                        let to = [
                            (i16::from(r) + dr).clamp(0, 255) as u8,
                            (i16::from(g) + dg).clamp(0, 255) as u8,
                            (i16::from(b) + db).clamp(0, 255) as u8,
                        ];
                        let [flip, _, _, delta_e] = measure_pair(&field, &flat(32, 32, to));
                        if delta_e > mixed.1 {
                            mixed = (flip, delta_e, [r, g, b]);
                        }
                    }
                }
            }
        }
        println!(
            "  no channel under {floor:>2}: all up, the worst FLIP {:.4} (dE00 {:.3}) at {:?}; each its own way, the worst dE00 {:.3} (FLIP {:.4}) at {:?}",
            up.0, up.1, up.2, mixed.1, mixed.0, mixed.2
        );
    }
    let bytes = reference.encode_png().unwrap().len();
    println!("the scene as a reference PNG: {bytes} bytes");
}
