// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The CPU compositor: the reference the GPU compositor's parity suite
//! holds it to, and the preview's last rung (see [`crate::ladder`]), the
//! one that draws when no device will. It blends 8-bit, gamma-encoded
//! pictures in light, as the GPU does.
//!
//! The picture of a layer is made the way [`crate::plan`] describes: the
//! crop, the flips and the fit, then the mask, the fades and the wipes at
//! the placement. Effects are the GPU's alone and are left out here, as are
//! treatments; [`leaves_out`] says when a frame had any. A deep picture is
//! drawn from its top eight bits.
//!
//! The frame is drawn in bands of rows, one a core, every layer in order
//! within a band: a pixel's blend reads only that pixel, so the bands
//! never meet.

use std::borrow::Cow;

use concat_core::frame::{BYTES_PER_PIXEL, Depth, Frame, Signal};
use concat_core::mask::{self, MaskUniform, StackUniform};
use concat_core::shader::TransitionPass;
use concat_core::timeline::Blend;

use crate::compositor::Compositor;
use crate::plan::{FramePlan, Geometry, PlannedLayer, Shading};

/// A straightforward CPU compositor.
#[derive(Clone, Copy, Default, Debug)]
pub struct CpuCompositor;

impl Compositor for CpuCompositor {
    /// The shape the pass names, drawn in plain arithmetic; see
    /// `transitions`. A pass naming no shape, or one the CPU does not
    /// draw, is declined.
    fn combine(
        &mut self,
        _width: u32,
        _height: u32,
        _time: f32,
        from: &Frame,
        to: &Frame,
        pass: &TransitionPass,
    ) -> Option<Frame> {
        let xfade = pass.xfade.as_deref()?;
        crate::transitions::combine(from, to, xfade, pass.progress)
    }

    fn render(&mut self, plan: &FramePlan) -> Frame {
        render_in_bands(plan, band_rows(plan.width, plan.height))
    }
}

/// [`CpuCompositor::render`], in bands of `rows` rows.
fn render_in_bands(plan: &FramePlan, rows: usize) -> Frame {
    // Geometry, masks, transitions and blending: what a frame is made of
    // besides its effects, which are the GPU's alone and are held to
    // their packages' probes instead.
    let mut ground = Frame::black(plan.width, plan.height);
    ground.fill(plan.canvas);
    let shallow: Vec<Option<Frame>> = plan
        .layers
        .iter()
        .map(|layer| {
            layer
                .source
                .as_deref()
                .filter(|source| source.depth() == Depth::Sixteen)
                .map(top_bytes)
        })
        .collect();
    let layers: Vec<Placed<'_>> = plan
        .layers
        .iter()
        .zip(&shallow)
        .filter_map(|(layer, shallow)| Placed::of(plan, layer, shallow.as_ref()))
        .collect();
    if layers.is_empty() {
        return ground;
    }
    let width = plan.width;
    let stride = width as usize * BYTES_PER_PIXEL;
    let pixels = ground.pixels_mut();
    std::thread::scope(|scope| {
        let mut bands = pixels.chunks_mut(rows * stride).enumerate();
        let first = bands.next();
        for (index, band) in bands {
            let layers = &layers;
            scope.spawn(move || draw_band(band, (index * rows) as u32, width, layers));
        }
        if let Some((_, band)) = first {
            draw_band(band, 0, width, &layers);
        }
    });
    ground
}

impl CpuCompositor {
    /// Whether the CPU leaves anything of `plan` out: an effect, a
    /// treatment, a deep picture it draws in eight bits, an HDR output it
    /// draws as SDR. The frame it draws is then not the frame, and a caller
    /// that promises the frame says so.
    pub fn leaves_out(plan: &FramePlan) -> bool {
        plan.output != Signal::Sdr
            || !plan.treatments.is_empty()
            || plan.layers.iter().any(|layer| {
                !layer.effects.is_empty()
                    || layer
                        .source
                        .as_ref()
                        .is_some_and(|source| source.depth() == Depth::Sixteen)
            })
    }
}

/// How many rows a band is: the frame split once a core, and drawn whole
/// on this thread where it is too small for a thread to pay.
fn band_rows(width: u32, height: u32) -> usize {
    static CORES: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let cores = *CORES.get_or_init(|| std::thread::available_parallelism().map_or(1, usize::from));
    let height = height.max(1) as usize;
    if (width as usize) * height < 1 << 16 {
        return height;
    }
    height.div_ceil(cores.min(height / 16).max(1))
}

/// A deep picture's top eight bits: what the CPU draws of it.
fn top_bytes(deep: &Frame) -> Frame {
    let pixels = deep.pixels().chunks_exact(2).map(|pair| pair[1]).collect();
    Frame::from_rgba(deep.width(), deep.height(), pixels)
        .unwrap_or_else(|| Frame::transparent(deep.width(), deep.height()))
}

/// `a` towards `b` by `amount` in their stored gamma: the old transition
/// shapes, which the GPU also runs in gamma through its legacy wrapper.
pub(crate) fn mix_frames(a: &Frame, b: &Frame, amount: f32) -> Frame {
    let amount = amount.clamp(0.0, 1.0);
    let mut out = a.clone();
    for (pixel, over) in out.pixels_mut().iter_mut().zip(b.pixels().iter()) {
        let base = f32::from(*pixel);
        *pixel = (base + (f32::from(*over) - base) * amount).round() as u8;
    }
    out
}

/// One layer made ready to draw: its picture, its placement, and what
/// weighs it. Built once a frame, and read by every band.
struct Placed<'a> {
    picture: Cow<'a, Frame>,
    geometry: Geometry,
    flip_h: bool,
    flip_v: bool,
    weigh: Weighing<'a>,
    aligned: bool,
}

impl<'a> Placed<'a> {
    /// `layer` placed in `plan`, its picture `shallow` where it is deep;
    /// `None` when it draws nothing.
    fn of(plan: &FramePlan, layer: &'a PlannedLayer, shallow: Option<&'a Frame>) -> Option<Self> {
        let opacity = layer.weight();
        if opacity <= 0.0 {
            return None;
        }
        let source = shallow.or(layer.source.as_deref())?;
        let geometry = layer.geometry(source, plan.width, plan.height);
        // The picture the placement samples: the source as it came, or made
        // first when the effects have to see it cropped, flipped and fitted.
        let (picture, geometry, flip_h, flip_v): (Cow<'_, Frame>, Geometry, bool, bool) =
            if layer.needs_preparing(&geometry) {
                let made = prepare(source, &geometry, layer.flip_h, layer.flip_v);
                (Cow::Owned(made), geometry.prepared(), false, false)
            } else {
                (Cow::Borrowed(source), geometry, layer.flip_h, layer.flip_v)
            };
        let shading = layer.shading();
        let weigh = Weighing {
            plain: layer.blend == Blend::Normal
                && shading.scale == 1.0
                && shading.offset == [0.0; 3],
            shading,
            mask: layer.mask.as_deref(),
            shape: layer.shape_uniform(&geometry),
            stack: layer.stack_uniform(&geometry),
            opacity,
            blend: layer.blend,
        };
        let aligned = geometry.is_aligned() && geometry.fits_source() && !flip_h && !flip_v;
        Some(Placed {
            picture,
            geometry,
            flip_h,
            flip_v,
            weigh,
            aligned,
        })
    }
}

/// Draws every layer, in order, into `band`: the rows of a `width` wide
/// frame from `top` down.
fn draw_band(band: &mut [u8], top: u32, width: u32, layers: &[Placed<'_>]) {
    let rows = (band.len() / (width as usize * BYTES_PER_PIXEL)) as u32;
    let mut band = Band {
        pixels: band,
        top,
        rows,
        width,
    };
    for layer in layers {
        if layer.aligned {
            blend_aligned(&mut band, &layer.picture, &layer.geometry, &layer.weigh);
        } else {
            blend_transformed(
                &mut band,
                &layer.picture,
                &layer.geometry,
                layer.flip_h,
                layer.flip_v,
                &layer.weigh,
            );
        }
    }
}

/// Rows `top..top + rows` of a frame `width` pixels wide.
struct Band<'a> {
    pixels: &'a mut [u8],
    top: u32,
    rows: u32,
    width: u32,
}

/// The picture at its fitted size: the crop rectangle of `source`, flipped
/// as asked, resampled bilinearly. What the effects run over when the
/// source is not already that.
fn prepare(source: &Frame, geometry: &Geometry, flip_h: bool, flip_v: bool) -> Frame {
    let (width, height) = geometry.fitted;
    let mut out = Frame::transparent(width, height);
    let stride = width as usize * BYTES_PER_PIXEL;
    let pixels = out.pixels_mut();
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            let (sx, sy) = geometry.source_of(u, v, flip_h, flip_v);
            let sample = sample_bilinear(source, sx, sy);
            let at = y as usize * stride + x as usize * BYTES_PER_PIXEL;
            for (channel, value) in sample.iter().enumerate() {
                pixels[at + channel] = value.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// What weighs a layer's pixel on its way to the ground: the fades and
/// wipes, the matte, the opacity, and the blend it meets the ground with.
struct Weighing<'a> {
    /// Normal blend and no fade: an opaque pixel is the picture's own.
    plain: bool,
    shading: Shading,
    mask: Option<&'a Frame>,
    shape: MaskUniform,
    /// The masks a mask pass draws on the GPU, evaluated per pixel here.
    stack: Option<StackUniform>,
    opacity: f32,
    blend: Blend,
}

impl Weighing<'_> {
    /// One pixel of the picture, `sample` its straight colour and alpha in
    /// `0..=255` at `u`, `v` across the picture, blended into `under`.
    #[inline]
    fn blend_into(&self, under: &mut [u8], sample: [f32; 4], u: f32, v: f32) {
        if !self.shading.keeps(u) {
            return;
        }
        let mut alpha = (sample[3] / 255.0) * self.opacity;
        if let Some(mask) = self.mask {
            let (mx, my) = (
                u * mask.width() as f32 - 0.5,
                v * mask.height() as f32 - 0.5,
            );
            alpha *= sample_bilinear(mask, mx, my)[3] / 255.0;
        }
        alpha *= mask::coverage(&self.shape, u, v);
        if let Some(stack) = &self.stack {
            alpha *= mask::stack_coverage(stack, u, v);
        }
        if alpha <= 0.0 {
            return;
        }
        if self.plain && alpha >= 1.0 {
            // Opaque over anything: the picture's level, which the trip
            // into light and back would land on anyway.
            for channel in 0..3 {
                under[channel] = sample[channel].round().clamp(0.0, 255.0) as u8;
            }
            under[3] = 255;
            return;
        }
        // In linear light, as the GPU blends: the layer's colour and the
        // ground decoded from their stored gamma, weighed, and encoded back.
        // The ground is weighed by its own coverage, which is all of it
        // unless the canvas is see-through (#456): the GPU blends over
        // premultiplied colour, so this mixes over the premultiplied
        // ground and divides the coverage that results back out.
        let coverage = f32::from(under[3]) / 255.0;
        let covered = alpha + coverage * (1.0 - alpha);
        for channel in 0..3 {
            let colour = self.shading.colour(channel, linear(sample[channel]));
            let ground = linear(f32::from(under[channel])) * coverage;
            under[channel] = encoded(mix(self.blend, colour, ground, alpha) / covered);
        }
        under[3] = (covered * 255.0).round().clamp(0.0, 255.0) as u8;
    }
}

/// The 2.4 gamma both ways, as tables: three powers a channel a pixel
/// were most of a frame's time.
struct Curves {
    /// Linear light at 1024 even steps of the level, and one past.
    light: [f32; 1025],
    /// The light at which each level rounds up to the next.
    edges: [f32; 255],
}

fn curves() -> &'static Curves {
    static CURVES: std::sync::OnceLock<Curves> = std::sync::OnceLock::new();
    CURVES.get_or_init(|| Curves {
        light: std::array::from_fn(|step| (step as f32 / 1024.0).powf(2.4)),
        edges: std::array::from_fn(|level| ((level as f32 + 0.5) / 255.0).powf(2.4)),
    })
}

/// A stored eight-bit level, `0..=255`, as linear light `0..1`: BT.1886's
/// 2.4 gamma, the GPU's upload. Read off the table between its steps.
fn linear(level: f32) -> f32 {
    let at = (level / 255.0).clamp(0.0, 1.0) * 1024.0;
    let step = (at as usize).min(1023);
    let table = &curves().light;
    table[step] + (table[step + 1] - table[step]) * (at - step as f32)
}

/// Linear light back to the eight-bit level nearest it, clipped: the GPU's
/// resolve.
fn encoded(light: f32) -> u8 {
    curves().edges.partition_point(|&edge| edge <= light) as u8
}

/// One channel of `blend`: `colour` is the layer's straight colour, `under`
/// the ground, both linear light `0..1`, `alpha` how much of the layer is
/// there.
/// Normal, Multiply, Screen and Add are the GPU's fixed-function blends
/// over premultiplied colour, spelled the same way so the two paths agree.
/// Lighten and Darken weigh the lighter (darker) of the two in by the
/// layer's alpha - a white layer at 30 % over mid grey lightens it 30 % of
/// the way to white, and darkens it not at all - which no fixed-function
/// blend expresses; the GPU samples a copy of the ground for these two
/// (audit 2026-09-23, #10).
#[inline]
fn mix(blend: Blend, colour: f32, under: f32, alpha: f32) -> f32 {
    let over = colour * alpha;
    match blend {
        Blend::Normal => over + under * (1.0 - alpha),
        Blend::Multiply => over * under + under * (1.0 - alpha),
        Blend::Screen => over * (1.0 - under) + under,
        Blend::Add => over + under,
        Blend::Lighten => colour.max(under) * alpha + under * (1.0 - alpha),
        Blend::Darken => colour.min(under) * alpha + under * (1.0 - alpha),
    }
}

/// `source` at texel coordinate `sx`, `sy` - its centre at `.0` - sampled
/// bilinearly with the edge texels clamped, the way the GPU's sampler
/// reads it. Straight colour and alpha in `0..=255`.
pub(crate) fn sample_bilinear(source: &Frame, sx: f32, sy: f32) -> [f32; 4] {
    let width = source.width() as usize;
    let height = source.height() as usize;
    let stride = width * BYTES_PER_PIXEL;
    let pixels = source.pixels();
    let x0 = (sx.floor().max(0.0) as usize).min(width - 1);
    let y0 = (sy.floor().max(0.0) as usize).min(height - 1);
    let x1 = (x0 + 1).min(width - 1);
    let y1 = (y0 + 1).min(height - 1);
    let fx = (sx - x0 as f32).clamp(0.0, 1.0);
    let fy = (sy - y0 as f32).clamp(0.0, 1.0);
    let mut sample = [0.0f32; 4];
    for (corner_x, corner_y, weight) in [
        (x0, y0, (1.0 - fx) * (1.0 - fy)),
        (x1, y0, fx * (1.0 - fy)),
        (x0, y1, (1.0 - fx) * fy),
        (x1, y1, fx * fy),
    ] {
        let at = corner_y * stride + corner_x * BYTES_PER_PIXEL;
        for channel in 0..4 {
            sample[channel] += f32::from(pixels[at + channel]) * weight;
        }
    }
    sample
}

/// Draws one layer through its placement: inverse-mapped, bilinearly sampled.
///
/// Each covered output pixel is carried backwards through the placement into
/// the picture, and from there through the flips and the crop into the
/// source, and sampled there. Inverse mapping is what makes the result
/// hole-free at any scale or angle; bilinear is the cheapest filter that
/// does not shimmer on motion. Only the transformed bounding box is
/// visited, so a small layer stays cheap on a large frame.
fn blend_transformed(
    band: &mut Band<'_>,
    picture: &Frame,
    geometry: &Geometry,
    flip_h: bool,
    flip_v: bool,
    weigh: &Weighing<'_>,
) {
    let (scale_x, scale_y) = geometry.scale;
    let (sin, cos) = geometry.rotation.sin_cos();
    let (fitted_w, fitted_h) = (geometry.fitted.0 as f32, geometry.fitted.1 as f32);
    let (centre_x, centre_y) = geometry.centre;

    // Bounding box of the transformed rectangle, clamped to the band.
    let half_w = fitted_w * scale_x / 2.0;
    let half_h = fitted_h * scale_y / 2.0;
    let reach_x = (half_w * cos.abs()) + (half_h * sin.abs());
    let reach_y = (half_w * sin.abs()) + (half_h * cos.abs());

    let x_from = ((centre_x - reach_x).floor().max(0.0)) as u32;
    let y_from = ((centre_y - reach_y).floor().max(band.top as f32)) as u32;
    let x_to = ((centre_x + reach_x).ceil().min(band.width as f32)) as u32;
    let y_to = ((centre_y + reach_y)
        .ceil()
        .min((band.top + band.rows) as f32)) as u32;
    if x_from >= x_to || y_from >= y_to {
        return;
    }

    let dst_stride = band.width as usize * BYTES_PER_PIXEL;
    for y in y_from..y_to {
        let row = (y - band.top) as usize * dst_stride;
        for x in x_from..x_to {
            // Sample at the pixel centre, mapped back into the picture:
            // untranslate, unrotate, unscale, then re-origin at the corner.
            let dx = (x as f32 + 0.5) - centre_x;
            let dy = (y as f32 + 0.5) - centre_y;
            let px = (dx * cos + dy * sin) / scale_x + fitted_w / 2.0;
            let py = (-dx * sin + dy * cos) / scale_y + fitted_h / 2.0;
            if px < 0.0 || py < 0.0 || px > fitted_w || py > fitted_h {
                continue;
            }
            let (u, v) = (px / fitted_w, py / fitted_h);
            let (sx, sy) = geometry.source_of(u, v, flip_h, flip_v);
            let sample = sample_bilinear(picture, sx, sy);
            let at = row + x as usize * BYTES_PER_PIXEL;
            weigh.blend_into(&mut band.pixels[at..at + BYTES_PER_PIXEL], sample, u, v);
        }
    }
}

/// Works out the visible span along one axis when a layer of `source` pixels is
/// placed at `offset` inside a destination of `destination` pixels.
///
/// Returns `(destination_start, source_start, count)`, or `None` when the layer
/// falls entirely outside.
fn overlap(offset: i32, source: u32, destination: u32) -> Option<(u32, u32, u32)> {
    let source = i64::from(source);
    let destination = i64::from(destination);
    let offset = i64::from(offset);

    let dst_start = offset.max(0);
    let src_start = (-offset).max(0);
    let count = (source - src_start).min(destination - dst_start);

    (count > 0).then_some((dst_start as u32, src_start as u32, count as u32))
}

/// Blends a picture placed by whole pixels, texel for pixel: the aligned
/// case, no resampling.
fn blend_aligned(band: &mut Band<'_>, picture: &Frame, geometry: &Geometry, weigh: &Weighing<'_>) {
    let (x, y) = geometry.corner();
    let Some((dst_x, src_x, columns)) = overlap(x, picture.width(), band.width) else {
        return;
    };
    // The band's rows alone: the frame's, less what is above and below it.
    let Some((dst_y, src_y, rows)) = overlap(y - band.top as i32, picture.height(), band.rows)
    else {
        return;
    };
    let src_stride = picture.width() as usize * BYTES_PER_PIXEL;
    let dst_stride = band.width as usize * BYTES_PER_PIXEL;
    let span = columns as usize * BYTES_PER_PIXEL;
    let (fitted_w, fitted_h) = (picture.width() as f32, picture.height() as f32);

    let src_pixels = picture.pixels();

    for row in 0..rows as usize {
        let src_offset = (src_y as usize + row) * src_stride + src_x as usize * BYTES_PER_PIXEL;
        let dst_offset = (dst_y as usize + row) * dst_stride + dst_x as usize * BYTES_PER_PIXEL;
        let v = (src_y as f32 + row as f32 + 0.5) / fitted_h;

        let src_row = &src_pixels[src_offset..src_offset + span];
        let dst_row = &mut band.pixels[dst_offset..dst_offset + span];

        for (column, (src_pixel, dst_pixel)) in src_row
            .chunks_exact(BYTES_PER_PIXEL)
            .zip(dst_row.chunks_exact_mut(BYTES_PER_PIXEL))
            .enumerate()
        {
            let u = (src_x as f32 + column as f32 + 0.5) / fitted_w;
            let sample = [
                f32::from(src_pixel[0]),
                f32::from(src_pixel[1]),
                f32::from(src_pixel[2]),
                f32::from(src_pixel[3]),
            ];
            weigh.blend_into(dst_pixel, sample, u, v);
        }
    }
}

#[cfg(test)]
mod tests {
    /// A frame with nothing on it is the canvas, and a picture that does
    /// not cover the frame sits on the canvas, not on black (#456).
    #[test]
    fn the_canvas_shows_where_no_picture_covers_the_frame() {
        use crate::Compositor;
        use crate::plan::{FramePlan, PlannedLayer, detached_clip};
        let gap = FramePlan {
            canvas: [0, 177, 64, 255],
            ..FramePlan::empty(8, 4)
        };
        let frame = super::CpuCompositor.render(&gap);
        assert!(
            frame
                .pixels()
                .chunks_exact(4)
                .all(|p| p == [0, 177, 64, 255])
        );

        let mut half = concat_core::Frame::black(4, 4);
        half.fill([255, 255, 255, 255]);
        let plan = FramePlan {
            layers: vec![PlannedLayer::picture(
                detached_clip(),
                std::sync::Arc::new(half),
            )],
            ..gap
        };
        let frame = super::CpuCompositor.render(&plan);
        // Fitted into an 8x4 frame, the 4x4 square leaves the sides bare.
        assert_eq!(frame.pixel(0, 2), Some([0, 177, 64, 255]));
        assert_eq!(frame.pixel(4, 2), Some([255, 255, 255, 255]));
    }

    /// On a see-through canvas the frame keeps its alpha: clear where
    /// nothing covers it, opaque under an opaque picture, and as covered
    /// as a half-faded picture is, in that picture's own colour (#456).
    #[test]
    fn a_see_through_canvas_keeps_the_alpha_of_what_covers_it() {
        use crate::Compositor;
        use crate::plan::{FramePlan, PlannedLayer, detached_clip};
        let mut square = concat_core::Frame::black(4, 4);
        square.fill([200, 40, 40, 255]);
        let square = std::sync::Arc::new(square);
        let plan = FramePlan {
            canvas: [0, 0, 0, 0],
            layers: vec![PlannedLayer::picture(detached_clip(), square.clone())],
            ..FramePlan::empty(8, 4)
        };
        let frame = super::CpuCompositor.render(&plan);
        assert_eq!(frame.pixel(0, 2).map(|p| p[3]), Some(0));
        assert_eq!(frame.pixel(4, 2), Some([200, 40, 40, 255]));

        let mut faded = PlannedLayer::picture(detached_clip(), square);
        faded.opacity = 0.5;
        let frame = super::CpuCompositor.render(&FramePlan {
            layers: vec![faded],
            ..plan
        });
        let pixel = frame.pixel(4, 2).expect("a pixel");
        assert!(pixel[3].abs_diff(128) <= 1, "{pixel:?}");
        assert!(
            pixel[0].abs_diff(200) <= 1 && pixel[1].abs_diff(40) <= 1,
            "{pixel:?}"
        );
    }

    #[test]
    fn lighten_and_darken_weigh_the_layer_in_by_its_alpha() {
        use super::{Blend, mix};
        // White at 30 % over mid grey: 30 % of the way to white, no darker.
        assert!((mix(Blend::Lighten, 255.0, 128.0, 0.3) - 166.1).abs() < 0.01);
        assert_eq!(mix(Blend::Darken, 255.0, 128.0, 0.3), 128.0);
        // Black at 30 % over mid grey: 30 % of the way to black, no lighter.
        assert!((mix(Blend::Darken, 0.0, 128.0, 0.3) - 89.6).abs() < 0.01);
        assert_eq!(mix(Blend::Lighten, 0.0, 128.0, 0.3), 128.0);
        // At full alpha the pair are the plain max and min.
        assert_eq!(mix(Blend::Lighten, 40.0, 128.0, 1.0), 128.0);
        assert_eq!(mix(Blend::Darken, 40.0, 128.0, 1.0), 40.0);
    }

    use std::sync::Arc;

    use concat_core::time::FrameRate;
    use concat_core::timeline::{Clip, ClipId, MediaRef, Timeline, Track, TrackKind, Transform};

    use super::*;
    use crate::plan::{Crop, PlannedTreatment, Transition};
    use concat_core::shader::ShaderPass;

    fn a_clip() -> ClipId {
        let mut timeline = Timeline::new(8, 8, FrameRate::THIRTY);
        let track = timeline.add_track(Track::new("V1", TrackKind::Video));
        timeline
            .add_clip(
                track,
                Clip::new(
                    MediaRef::new("a.mp4"),
                    concat_core::time::Rational::ZERO,
                    concat_core::time::Rational::ONE,
                ),
            )
            .expect("track exists")
    }

    fn layer(frame: Frame) -> PlannedLayer {
        PlannedLayer::picture(a_clip(), Arc::new(frame))
    }

    fn plan(width: u32, height: u32, layers: Vec<PlannedLayer>) -> FramePlan {
        FramePlan {
            layers,
            ..FramePlan::empty(width, height)
        }
    }

    fn render(width: u32, height: u32, layers: Vec<PlannedLayer>) -> Frame {
        CpuCompositor.render(&plan(width, height, layers))
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        let mut frame = Frame::transparent(width, height);
        frame.fill(rgba);
        frame
    }

    /// A layer moved by whole pixels; the transform is fractions of the
    /// output, so the test speaks in pixels and converts.
    fn moved(mut layer: PlannedLayer, x: f64, y: f64, width: u32, height: u32) -> PlannedLayer {
        layer.transform = Transform {
            offset_x: x / f64::from(width),
            offset_y: y / f64::from(height),
            ..Transform::default()
        };
        layer
    }

    /// Multiply by white and screen with black both leave the ground as it
    /// was; add brightens it - in light: 100 and 50 are 0.106 and 0.020 of
    /// white, and their sum is stored at 107 - darken cannot, lighten takes
    /// the brighter.
    #[test]
    fn the_blend_modes_do_what_their_names_say() {
        let over = |rgba: [u8; 4], blend: Blend| {
            let ground = solid(2, 2, [100, 100, 100, 255]);
            let mut top = layer(solid(2, 2, rgba));
            top.blend = blend;
            render(2, 2, vec![layer(ground), top]).pixels()[0]
        };
        assert_eq!(over([255, 255, 255, 255], Blend::Multiply), 100);
        assert_eq!(over([0, 0, 0, 255], Blend::Screen), 100);
        assert_eq!(over([255, 255, 255, 255], Blend::Screen), 255);
        assert_eq!(over([50, 50, 50, 255], Blend::Add), 107);
        assert_eq!(over([255, 255, 255, 255], Blend::Darken), 100);
        assert_eq!(over([30, 30, 30, 255], Blend::Lighten), 100);
        assert_eq!(over([200, 200, 200, 255], Blend::Lighten), 200);
        assert_eq!(over([200, 200, 200, 255], Blend::Normal), 200);
    }

    #[test]
    fn no_layers_gives_opaque_black() {
        let frame = render(2, 2, vec![]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn an_opaque_layer_replaces_the_background() {
        let frame = render(2, 2, vec![layer(solid(2, 2, [255, 0, 0, 255]))]);
        assert_eq!(frame.pixel(1, 1), Some([255, 0, 0, 255]));
    }

    /// Half a white layer over black is half the light of white, which a
    /// 2.4 gamma stores at 191, not at the level halfway up.
    #[test]
    fn half_opacity_lands_halfway_in_light() {
        let mut white = layer(solid(1, 1, [255, 255, 255, 255]));
        white.opacity = 0.5;
        let frame = render(1, 1, vec![white]);
        assert_eq!(frame.pixel(0, 0), Some([191, 191, 191, 255]));
    }

    #[test]
    fn source_alpha_and_layer_opacity_multiply() {
        let mut half = layer(solid(1, 1, [255, 255, 255, 128]));
        half.opacity = 0.5;
        let frame = render(1, 1, vec![half]);
        // 128/255 * 0.5 ~= 0.251 of white's light, stored at 143.
        assert_eq!(frame.pixel(0, 0), Some([143, 143, 143, 255]));
    }

    #[test]
    fn later_layers_draw_on_top() {
        let frame = render(
            1,
            1,
            vec![
                layer(solid(1, 1, [255, 0, 0, 255])),
                layer(solid(1, 1, [0, 0, 255, 255])),
            ],
        );
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn a_transparent_layer_changes_nothing() {
        let frame = render(2, 2, vec![layer(Frame::transparent(2, 2))]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn a_layer_without_a_picture_draws_nothing() {
        let mut missing = layer(solid(2, 2, [255, 0, 0, 255]));
        missing.source = None;
        let frame = render(2, 2, vec![missing]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn offset_layers_are_clipped_not_wrapped() {
        // Placed so only its bottom-right pixel lands on the output's top-left.
        let red = moved(layer(solid(2, 2, [255, 0, 0, 255])), -1.0, -1.0, 2, 2);
        let frame = render(2, 2, vec![red]);
        assert_eq!(frame.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(
            frame.pixel(1, 1),
            Some([0, 0, 0, 255]),
            "must not wrap around"
        );
    }

    #[test]
    fn a_layer_entirely_off_screen_is_skipped() {
        let red = moved(layer(solid(2, 2, [255, 0, 0, 255])), 50.0, 50.0, 2, 2);
        let frame = render(2, 2, vec![red]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    /// A picture larger than the output is fitted into it, not cropped:
    /// an 8 by 8 red square on a 2 by 2 output is the whole square, small.
    #[test]
    fn a_layer_larger_than_the_output_is_fitted() {
        let frame = render(2, 2, vec![layer(solid(8, 8, [255, 0, 0, 255]))]);
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.pixel(1, 1), Some([255, 0, 0, 255]));
        // And a wide picture on a square output keeps its shape: bars
        // above and below.
        let frame = render(4, 4, vec![layer(solid(8, 4, [255, 0, 0, 255]))]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
        assert_eq!(frame.pixel(0, 1), Some([255, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 2), Some([255, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 3), Some([0, 0, 0, 255]));
    }

    #[test]
    fn scaling_doubles_coverage() {
        // A 2x2 layer centred on a 4x4 output scaled x2 about its centre
        // covers the whole output.
        let mut red = layer(solid(2, 2, [255, 0, 0, 255]));
        red.transform = Transform {
            scale: 2.0,
            ..Transform::default()
        };
        let frame = render(4, 4, vec![red]);
        assert_eq!(frame.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 3), Some([255, 0, 0, 255]));
    }

    #[test]
    fn translation_moves_the_layer() {
        let red = moved(layer(solid(1, 1, [255, 0, 0, 255])), 1.0, 0.0, 3, 1);
        let frame = render(3, 1, vec![red]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
        assert_eq!(frame.pixel(2, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn a_half_turn_swaps_the_ends() {
        let mut strip = Frame::transparent(2, 1);
        strip.set_pixel(0, 0, [255, 0, 0, 255]);
        strip.set_pixel(1, 0, [0, 0, 255, 255]);
        let mut turned = layer(strip);
        turned.transform = Transform {
            rotation: 180.0,
            ..Transform::default()
        };
        let frame = render(2, 1, vec![turned]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(frame.pixel(1, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn a_transformed_layer_is_clipped_at_the_frame_edge() {
        let mut red = layer(solid(2, 2, [255, 0, 0, 255]));
        red.transform = Transform {
            scale: 100.0,
            ..Transform::default()
        };
        let frame = render(2, 2, vec![red]);
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.pixel(1, 1), Some([255, 0, 0, 255]));
    }

    /// A flip mirrors the picture, with and without a resample.
    #[test]
    fn flips_mirror_the_picture() {
        let mut strip = Frame::transparent(2, 2);
        strip.set_pixel(0, 0, [255, 0, 0, 255]);
        strip.set_pixel(1, 0, [0, 0, 255, 255]);
        strip.set_pixel(0, 1, [0, 255, 0, 255]);
        strip.set_pixel(1, 1, [255, 255, 255, 255]);
        let mut flipped = layer(strip.clone());
        flipped.flip_h = true;
        let frame = render(2, 2, vec![flipped]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(frame.pixel(1, 0), Some([255, 0, 0, 255]));
        let mut flipped = layer(strip);
        flipped.flip_v = true;
        let frame = render(2, 2, vec![flipped]);
        assert_eq!(frame.pixel(0, 0), Some([0, 255, 0, 255]));
        assert_eq!(frame.pixel(1, 1), Some([0, 0, 255, 255]));
    }

    /// A crop keeps the part it names and fits it to the output.
    #[test]
    fn a_crop_keeps_what_it_names() {
        let mut strip = Frame::transparent(4, 2);
        for x in 0..4 {
            for y in 0..2 {
                strip.set_pixel(
                    x,
                    y,
                    if x < 2 {
                        [255, 0, 0, 255]
                    } else {
                        [0, 0, 255, 255]
                    },
                );
            }
        }
        let mut cropped = layer(strip);
        cropped.crop = Crop::of([0.5, 0.0, 0.0, 0.0]);
        let frame = render(2, 2, vec![cropped]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(frame.pixel(1, 1), Some([0, 0, 255, 255]));
    }

    /// The matte's alpha weighs the picture, sampled across it whatever
    /// its size; a fade pulls the colour; a wipe hides the far side.
    #[test]
    fn the_mask_the_fade_and_the_wipe_weigh_the_picture() {
        let red = solid(4, 4, [255, 0, 0, 255]);
        let mut matte = Frame::transparent(2, 1);
        matte.set_pixel(0, 0, [0, 0, 0, 255]);
        let mut masked = layer(red.clone());
        masked.mask = Some(Arc::new(matte));
        let frame = render(4, 4, vec![masked]);
        assert_eq!(frame.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 3), Some([0, 0, 0, 255]));

        let mut faded = layer(red.clone());
        faded.transitions = vec![Transition::FadeTo {
            colour: [1.0; 3],
            amount: 0.5,
        }];
        let frame = render(4, 4, vec![faded]);
        // Half way to white in light: green and blue at half of white's.
        assert_eq!(frame.pixel(0, 0), Some([255, 191, 191, 255]));

        let mut wiped = layer(red);
        wiped.transitions = vec![Transition::Wipe {
            uncovered: 0.5,
            from_right: true,
        }];
        let frame = render(4, 4, vec![wiped]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 0), Some([255, 0, 0, 255]));
    }

    /// A treatment on track 1 runs over what track 0 drew and not over
    /// what track 2 draws on top of it, and its strength blends the result
    /// back. With no kernel for its package the ground is left as it was.
    #[test]
    fn a_treatment_without_a_kernel_leaves_the_stack_as_it_was() {
        let red = layer(solid(8, 8, [255, 0, 0, 255]));
        let mut blue = layer(solid(2, 2, [0, 0, 255, 255]));
        blue.track = 2;
        blue.transform = Transform {
            offset_x: -3.0 / 8.0,
            offset_y: -3.0 / 8.0,
            ..Transform::default()
        };
        let unknown = ShaderPass {
            package: "test.nothing".to_owned(),
            key: "test.nothing@1".to_owned(),
            source: Arc::from(""),
            params: vec![0; 16],
            values: Default::default(),
            intensity: 1.0,
            lut: None,
            reveal_map: None,
            stages: Vec::new(),
        };
        let mut frame_plan = plan(8, 8, vec![red, blue]);
        frame_plan.treatments = vec![PlannedTreatment {
            track: 1,
            effects: vec![unknown],
            strength: 1.0,
            start: 0.0,
        }];
        let out = CpuCompositor.render(&frame_plan);
        assert_eq!(out.pixel(7, 7), Some([255, 0, 0, 255]));
        assert_eq!(out.pixel(0, 0), Some([0, 0, 255, 255]));
    }

    /// Through the trait: a pass naming a shape the CPU draws is combined,
    /// one naming none is declined, and the shader source is never read.
    #[test]
    fn the_cpu_combines_by_the_named_shape_and_declines_without_one() {
        let from = solid(8, 8, [255, 0, 0, 255]);
        let to = solid(8, 8, [0, 0, 255, 255]);
        let pass = |xfade: Option<&str>| TransitionPass {
            key: "test.cut@1".to_owned(),
            source: Arc::from("not wgsl at all"),
            params: vec![0; 16],
            progress: 1.0,
            lut: None,
            xfade: xfade.map(str::to_owned),
        };
        let done = CpuCompositor
            .combine(8, 8, 0.0, &from, &to, &pass(Some("wipeleft")))
            .expect("a shape the CPU draws");
        assert_eq!(done.pixel(0, 0), Some([0, 0, 255, 255]));
        assert!(
            CpuCompositor
                .combine(8, 8, 0.0, &from, &to, &pass(None))
                .is_none()
        );
        assert!(
            CpuCompositor
                .combine(8, 8, 0.0, &from, &to, &pass(Some("hlslice")))
                .is_none()
        );
    }

    /// The #709 frame - a 1280x720 clip and six captions over it - drawn
    /// at the monitor's half size: the same picture in bands as whole, and
    /// how long a frame takes, printed (`--release --nocapture`).
    #[test]
    fn the_cpu_draws_the_709_frame_in_bands_as_it_does_whole() {
        let mut clip = Frame::transparent(1280, 720);
        for (index, pixel) in clip.pixels_mut().chunks_exact_mut(4).enumerate() {
            let (x, y) = (index % 1280, index / 1280);
            pixel.copy_from_slice(&[(x / 5) as u8, (y / 3) as u8, 160, 255]);
        }
        let mut layers = vec![layer(clip)];
        for index in 0..6 {
            let mut words = Frame::transparent(1280, 720);
            for y in 560..620 {
                for x in 200 + index * 20..1080 - index * 20 {
                    words.set_pixel(x, y, [255, 255, 255, 255]);
                }
            }
            let mut caption = layer(words);
            caption.opacity = if index % 2 == 0 { 1.0 } else { 0.5 };
            caption.transform.offset_y = -0.05 * f64::from(index);
            caption.transform.rotation = f64::from(index);
            layers.push(caption);
        }
        let plan = plan(640, 360, layers);
        let whole = render_in_bands(&plan, 360);
        let banded = render_in_bands(&plan, 7);
        assert!(whole == banded, "the bands draw the frame as one does");
        assert!(
            !CpuCompositor::leaves_out(&plan),
            "nothing in it is left out"
        );

        let begun = std::time::Instant::now();
        for _ in 0..20 {
            CpuCompositor.render(&plan);
        }
        eprintln!(
            "cpu: the 709 frame at 640x360 in {:.2} ms, bands of {} rows",
            begun.elapsed().as_secs_f64() * 1000.0 / 20.0,
            band_rows(640, 360)
        );
    }

    /// What the CPU leaves out of a frame is said: effects, treatments,
    /// deep pictures, an HDR output.
    #[test]
    fn the_cpu_says_what_it_leaves_out() {
        let plain = plan(4, 4, vec![layer(solid(4, 4, [9, 9, 9, 255]))]);
        assert!(!CpuCompositor::leaves_out(&plain));
        let mut treated = plain.clone();
        treated.layers[0].effects = vec![ShaderPass {
            package: "test.nothing".to_owned(),
            key: "test.nothing@1".to_owned(),
            source: Arc::from(""),
            params: Vec::new(),
            values: Default::default(),
            intensity: 1.0,
            lut: None,
            reveal_map: None,
            stages: Vec::new(),
        }];
        assert!(CpuCompositor::leaves_out(&treated));
        let deep = Frame::from_rgba64(2, 2, vec![0xff; 32], Signal::Hlg).expect("deep");
        let mut deep = plan(4, 4, vec![layer(deep)]);
        assert!(CpuCompositor::leaves_out(&deep));
        let frame = CpuCompositor.render(&deep);
        assert_eq!(
            frame.pixel(2, 2),
            Some([255, 255, 255, 255]),
            "drawn from its top bytes"
        );
        deep.layers.clear();
        deep.output = Signal::Pq;
        assert!(CpuCompositor::leaves_out(&deep));
    }

    #[test]
    fn overlap_spans_are_correct() {
        assert_eq!(overlap(0, 4, 4), Some((0, 0, 4)));
        assert_eq!(overlap(2, 4, 4), Some((2, 0, 2)));
        assert_eq!(overlap(-2, 4, 4), Some((0, 2, 2)));
        assert_eq!(overlap(4, 4, 4), None);
        assert_eq!(overlap(-4, 4, 4), None);
    }
}
