// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The scopes: what the monitor's picture holds, counted on the GPU from
//! the working space - its light before anything is clipped - and drawn
//! here from the counts.
//!
//! On an SDR timeline a level is the display encoding, 0 to 100 %, as a
//! Rec. 709 waveform reads. On an HDR one it is the light in nits on PQ's
//! scale, so 100 nits, SDR white at 203 and a 1000-nit highlight each have
//! their place however far apart they are (`WgpuCompositor::render_texture_scoped`
//! counts; [`ScopeData::draw`] draws).

/// Which scope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeKind {
    /// Luma across the picture: a column of levels for each column of it.
    Waveform,
    /// Red, green and blue as three waveforms side by side.
    Parade,
    /// The colour of every pixel as a point on the Cb-Cr plane.
    Vectorscope,
    /// How many pixels hold each level, a curve a channel.
    Histogram,
}

/// The levels a waveform, a parade or a histogram tells apart, and the side
/// of the vectorscope's plane.
pub const LEVELS: u32 = 256;

/// The columns a waveform counts the picture in, and a parade each channel.
pub const COLUMNS: u32 = 256;

impl ScopeKind {
    /// Every kind, in the order the pane offers them.
    pub const ALL: [ScopeKind; 4] = [
        ScopeKind::Waveform,
        ScopeKind::Parade,
        ScopeKind::Vectorscope,
        ScopeKind::Histogram,
    ];

    /// How many counts the scope takes.
    pub const fn bins(self) -> usize {
        (match self {
            ScopeKind::Waveform => COLUMNS * LEVELS,
            ScopeKind::Parade => 3 * COLUMNS * LEVELS,
            ScopeKind::Vectorscope => LEVELS * LEVELS,
            ScopeKind::Histogram => 4 * LEVELS,
        }) as usize
    }

    /// The number the counting shader switches on.
    pub(crate) const fn index(self) -> u32 {
        match self {
            ScopeKind::Waveform => 0,
            ScopeKind::Parade => 1,
            ScopeKind::Vectorscope => 2,
            ScopeKind::Histogram => 3,
        }
    }
}

/// One frame's counts for a scope.
#[derive(Clone, Debug, PartialEq)]
pub struct ScopeData {
    /// Which scope the counts are for.
    pub kind: ScopeKind,
    /// The timeline is HDR: a level is nits on PQ's scale, not the
    /// display encoding.
    pub hdr: bool,
    /// The picture counted, in pixels.
    pub size: (u32, u32),
    /// The counts, laid out as the kind says: a waveform column by column,
    /// each `LEVELS` from black up; a parade the same for red, green and
    /// blue in turn; a vectorscope row by row from Cr's top; a histogram
    /// red, green, blue and luma.
    pub counts: Vec<u32>,
}

impl ScopeData {
    /// The scope as a picture: its width, its height, and RGBA, eight bits
    /// a channel, opaque, black where nothing fell.
    pub fn draw(&self) -> (u32, u32, Vec<u8>) {
        let pixels = (self.size.0 * self.size.1).max(1) as f32;
        match self.kind {
            ScopeKind::Waveform => {
                let mut image = Image::new(COLUMNS, LEVELS);
                self.traces(&mut image, 0, 0, [0.62, 1.0, 0.70], pixels);
                image.finish()
            }
            ScopeKind::Parade => {
                let mut image = Image::new(3 * COLUMNS, LEVELS);
                for (channel, tint) in [[1.0, 0.36, 0.36], [0.36, 1.0, 0.46], [0.42, 0.56, 1.0]]
                    .into_iter()
                    .enumerate()
                {
                    self.traces(
                        &mut image,
                        channel * (COLUMNS * LEVELS) as usize,
                        channel as u32 * COLUMNS,
                        tint,
                        pixels,
                    );
                }
                image.finish()
            }
            ScopeKind::Vectorscope => {
                let mut image = Image::new(LEVELS, LEVELS);
                // A point the square root of its share: a field of one
                // colour is a bright spot, a scatter of many still shows.
                let scale = (LEVELS * LEVELS) as f32 / 256.0 / pixels;
                for (at, count) in self.counts.iter().enumerate() {
                    let level = ((*count as f32 * scale).sqrt()).min(1.0);
                    let (x, y) = (at as u32 % LEVELS, at as u32 / LEVELS);
                    image.add(x, y, [level * 0.9, level, level * 0.9]);
                }
                for target in TARGETS {
                    let (x, y) = plane(target);
                    image.square(x, y, 3, [0.55, 0.42, 0.20]);
                }
                image.cross(LEVELS / 2, LEVELS / 2, [0.35, 0.35, 0.35]);
                image.finish()
            }
            ScopeKind::Histogram => {
                const HEIGHT: u32 = 128;
                let mut image = Image::new(LEVELS, HEIGHT);
                // Scaled to the tallest bar short of the two ends, where a
                // clipped or a black picture piles up and would flatten the
                // rest.
                let tallest = (0..4)
                    .flat_map(|channel| {
                        let start = channel * LEVELS as usize;
                        self.counts[start + 1..start + LEVELS as usize - 1]
                            .iter()
                            .copied()
                    })
                    .max()
                    .unwrap_or(0)
                    .max(1) as f32;
                for (channel, tint) in [[0.9, 0.25, 0.25], [0.25, 0.85, 0.35], [0.30, 0.45, 1.0]]
                    .into_iter()
                    .enumerate()
                {
                    for level in 0..LEVELS {
                        let count = self.counts[channel * LEVELS as usize + level as usize];
                        let bar = ((count as f32 / tallest).min(1.0) * HEIGHT as f32) as u32;
                        for y in HEIGHT - bar..HEIGHT {
                            image.add(level, y, tint.map(|c| c * 0.6));
                        }
                    }
                }
                let mut previous: Option<u32> = None;
                for level in 0..LEVELS {
                    let count = self.counts[3 * LEVELS as usize + level as usize];
                    let top = HEIGHT
                        - 1
                        - ((count as f32 / tallest).min(1.0) * (HEIGHT - 1) as f32) as u32;
                    let from = previous.unwrap_or(top);
                    for y in from.min(top)..=from.max(top) {
                        image.set(level, y, [0.9, 0.9, 0.9]);
                    }
                    previous = Some(top);
                }
                image.finish()
            }
        }
    }

    /// A waveform's columns from `start` in the counts, drawn from `left`
    /// in the image in `tint`: each level as bright as the square root of
    /// the share of its column it holds.
    fn traces(&self, image: &mut Image, start: usize, left: u32, tint: [f32; 3], pixels: f32) {
        let per_column = (pixels / COLUMNS as f32).max(1.0);
        let scale = LEVELS as f32 / 24.0 / per_column;
        for column in 0..COLUMNS {
            for level in 0..LEVELS {
                let count = self.counts[start + (column * LEVELS + level) as usize];
                if count == 0 {
                    continue;
                }
                let bright = ((count as f32 * scale).sqrt()).clamp(0.12, 1.0);
                image.add(left + column, LEVELS - 1 - level, tint.map(|c| c * bright));
            }
        }
    }
}

/// The colours of 75 % bars, which a vectorscope's targets mark: red,
/// magenta, blue, cyan, green, yellow, as display-encoded RGB.
const TARGETS: [[f32; 3]; 6] = [
    [0.75, 0.0, 0.0],
    [0.75, 0.0, 0.75],
    [0.0, 0.0, 0.75],
    [0.0, 0.75, 0.75],
    [0.0, 0.75, 0.0],
    [0.75, 0.75, 0.0],
];

/// Where an encoded colour falls on the vectorscope's plane, as the
/// counting shader places it.
fn plane(rgb: [f32; 3]) -> (u32, u32) {
    let y = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    let cb = (rgb[2] - y) / 1.8556;
    let cr = (rgb[0] - y) / 1.5748;
    (level_of(cb + 0.5), level_of(0.5 - cr))
}

/// The level a value `0..1` counts under, as the shader rounds it.
fn level_of(value: f32) -> u32 {
    ((value.clamp(0.0, 1.0) * (LEVELS - 1) as f32 + 0.5) as u32).min(LEVELS - 1)
}

/// SMPTE ST 2084's signal for `nits`: where a level sits on an HDR scope.
pub fn pq(nits: f64) -> f64 {
    let (m1, m2) = (0.159_301_757_812_5, 78.843_75);
    let (c1, c2, c3) = (0.835_937_5, 18.851_562_5, 18.687_5);
    let y = (nits / 10_000.0).clamp(0.0, 1.0).powf(m1);
    ((c1 + c2 * y) / (1.0 + c3 * y)).powf(m2)
}

/// The scale's marks: where each sits, `0..1` from the bottom of a
/// waveform or a parade or from the left of a histogram, and what it
/// reads - a share of the display level on an SDR timeline, nits on an
/// HDR one. None on a vectorscope, whose targets are drawn in it.
pub fn marks(kind: ScopeKind, hdr: bool) -> Vec<(f32, String)> {
    if kind == ScopeKind::Vectorscope {
        return Vec::new();
    }
    if hdr {
        [0.0, 1.0, 10.0, 100.0, 203.0, 1000.0, 4000.0]
            .into_iter()
            .map(|nits| (pq(nits) as f32, format!("{nits}")))
            .collect()
    } else {
        [0, 25, 50, 75, 100]
            .into_iter()
            .map(|share| (share as f32 / 100.0, format!("{share}")))
            .collect()
    }
}

/// An RGB picture being drawn, in floats, added to where traces cross.
struct Image {
    width: u32,
    height: u32,
    pixels: Vec<[f32; 3]>,
}

impl Image {
    fn new(width: u32, height: u32) -> Self {
        Image {
            width,
            height,
            pixels: vec![[0.0; 3]; (width * height) as usize],
        }
    }

    fn add(&mut self, x: u32, y: u32, rgb: [f32; 3]) {
        if x < self.width && y < self.height {
            let pixel = &mut self.pixels[(y * self.width + x) as usize];
            for (into, value) in pixel.iter_mut().zip(rgb) {
                *into += value;
            }
        }
    }

    fn set(&mut self, x: u32, y: u32, rgb: [f32; 3]) {
        if x < self.width && y < self.height {
            self.pixels[(y * self.width + x) as usize] = rgb;
        }
    }

    /// The outline of a square `2 * half + 1` across about `(x, y)`.
    fn square(&mut self, x: u32, y: u32, half: u32, rgb: [f32; 3]) {
        let (left, right) = (x.saturating_sub(half), x + half);
        let (top, bottom) = (y.saturating_sub(half), y + half);
        for across in left..=right {
            self.add(across, top, rgb);
            self.add(across, bottom, rgb);
        }
        for down in top..=bottom {
            self.add(left, down, rgb);
            self.add(right, down, rgb);
        }
    }

    /// A small cross at `(x, y)`: the middle of the plane, colour none.
    fn cross(&mut self, x: u32, y: u32, rgb: [f32; 3]) {
        for offset in 0..9 {
            self.add(x + offset - 4, y, rgb);
            self.add(x, y + offset - 4, rgb);
        }
    }

    fn finish(self) -> (u32, u32, Vec<u8>) {
        let bytes = self
            .pixels
            .iter()
            .flat_map(|[r, g, b]| {
                let byte = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
                [byte(*r), byte(*g), byte(*b), 255]
            })
            .collect();
        (self.width, self.height, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A waveform of a flat picture is one line across at its level; a
    /// parade is three; every kind draws at its own size, opaque.
    #[test]
    fn a_flat_picture_draws_a_line() {
        let mut counts = vec![0u32; ScopeKind::Waveform.bins()];
        for column in 0..COLUMNS {
            counts[(column * LEVELS + 128) as usize] = 100;
        }
        let waveform = ScopeData {
            kind: ScopeKind::Waveform,
            hdr: false,
            size: (256, 100),
            counts,
        };
        let (width, height, pixels) = waveform.draw();
        assert_eq!((width, height), (COLUMNS, LEVELS));
        let at = |x: u32, y: u32| pixels[((y * width + x) * 4 + 1) as usize];
        assert!(at(10, LEVELS - 1 - 128) > 200, "the line is bright");
        assert_eq!(at(10, 10), 0, "the rest is black");
        assert!(pixels.chunks_exact(4).all(|pixel| pixel[3] == 255));
        for kind in ScopeKind::ALL {
            let data = ScopeData {
                kind,
                hdr: true,
                size: (64, 36),
                counts: vec![3; kind.bins()],
            };
            let (width, height, pixels) = data.draw();
            assert_eq!(pixels.len(), (width * height * 4) as usize, "{kind:?}");
        }
    }

    /// The marks read the display level on SDR and nits on HDR, where SDR
    /// white sits at 58 % of PQ's scale and a 1000-nit highlight at 75 %.
    #[test]
    fn the_scale_reads_shares_or_nits() {
        let sdr = marks(ScopeKind::Waveform, false);
        assert_eq!(sdr.first(), Some(&(0.0, "0".to_owned())));
        assert_eq!(sdr.last(), Some(&(1.0, "100".to_owned())));
        let hdr = marks(ScopeKind::Parade, true);
        let at = |label: &str| {
            hdr.iter()
                .find(|(_, text)| text == label)
                .map(|(at, _)| *at)
        };
        assert!((at("203").expect("SDR white") - 0.5807).abs() < 0.001);
        assert!((at("1000").expect("a highlight") - 0.7518).abs() < 0.001);
        assert!(marks(ScopeKind::Vectorscope, true).is_empty());
        assert_eq!(plane([0.5, 0.5, 0.5]), (level_of(0.5), level_of(0.5)));
    }
}
