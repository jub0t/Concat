// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Blending layers into one frame.
//!
//! [`CpuCompositor`] is the reference implementation: obvious, dependency-free,
//! and slow enough that it will eventually only be used for tests and for
//! checking the GPU path's output. The `Compositor` trait is the seam that
//! `wgpu` will slot into.

use concat_core::frame::{BYTES_PER_PIXEL, Frame};
use concat_core::shader::ShaderPass;
use concat_core::timeline::Blend;

/// A layer's placement beyond its base position, in output pixels.
///
/// Applied about the layer's centre: scale first, then rotation, then the
/// translation. Pixel-space on purpose - the resolution-independent form
/// lives in `concat-core::timeline::Transform`, and whoever builds layers
/// converts exactly once.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    /// Multiplier over the layer's own size.
    pub scale: f32,
    /// Clockwise rotation about the layer's centre, in radians.
    pub rotation: f32,
    /// Horizontal offset of the layer's centre from its base position, pixels.
    pub translate_x: f32,
    /// Vertical offset of the layer's centre from its base position, pixels.
    pub translate_y: f32,
    /// A multiplier on the width beyond `scale`; 1 keeps the aspect.
    pub stretch_x: f32,
    /// The same for the height.
    pub stretch_y: f32,
}

impl Placement {
    /// Unscaled, unrotated, unmoved.
    pub const IDENTITY: Placement = Placement {
        scale: 1.0,
        rotation: 0.0,
        translate_x: 0.0,
        translate_y: 0.0,
        stretch_x: 1.0,
        stretch_y: 1.0,
    };

    /// True when applying this placement would change nothing.
    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

impl Default for Placement {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// One source image, positioned and weighted.
#[derive(Clone, Copy, Debug)]
pub struct Layer<'a> {
    /// The pixels to draw.
    pub frame: &'a Frame,
    /// Blend strength over what is beneath, in `0.0..=1.0`.
    pub opacity: f32,
    /// Horizontal offset of the layer's top-left corner. May be negative.
    pub x: i32,
    /// Vertical offset of the layer's top-left corner. May be negative.
    pub y: i32,
    /// Scale, rotation and translation about the layer's centre.
    pub placement: Placement,
    /// How the layer's colour meets the ground.
    pub blend: Blend,
    /// Shader passes to run over the layer's pixels before it is placed, in
    /// order. The GPU compositor runs them; the CPU reference cannot, and
    /// draws the layer untreated.
    pub passes: &'a [ShaderPass],
    /// Seconds into the timeline, for passes that move.
    pub time: f32,
}

impl<'a> Layer<'a> {
    /// A layer drawn at the origin, fully opaque.
    pub fn new(frame: &'a Frame) -> Self {
        Self {
            frame,
            opacity: 1.0,
            x: 0,
            y: 0,
            placement: Placement::IDENTITY,
            blend: Blend::Normal,
            passes: &[],
            time: 0.0,
        }
    }

    /// Sets the shader passes to run over the layer first.
    pub fn with_passes(mut self, passes: &'a [ShaderPass]) -> Self {
        self.passes = passes;
        self
    }

    /// Sets the timeline instant the passes see.
    pub fn at_time(mut self, time: f32) -> Self {
        self.time = time;
        self
    }

    /// Sets the blend mode.
    pub fn with_blend(mut self, blend: Blend) -> Self {
        self.blend = blend;
        self
    }

    /// Sets the blend strength.
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Sets the top-left corner.
    pub fn at(mut self, x: i32, y: i32) -> Self {
        self.x = x;
        self.y = y;
        self
    }

    /// Sets the scale/rotation/translation.
    pub fn with_placement(mut self, placement: Placement) -> Self {
        self.placement = placement;
        self
    }
}

/// Blends layers into a single output frame.
pub trait Compositor {
    /// Draws `layers` bottom-most first over an opaque black background.
    ///
    /// Layers may hang off any edge; anything outside the output is clipped.
    /// The result is always fully opaque - it is what goes to screen or to an
    /// encoder, and neither has anything to show through.
    fn composite(&mut self, width: u32, height: u32, layers: &[Layer<'_>]) -> Frame;
}

/// A straightforward CPU compositor.
#[derive(Clone, Copy, Default, Debug)]
pub struct CpuCompositor;

impl Compositor for CpuCompositor {
    fn composite(&mut self, width: u32, height: u32, layers: &[Layer<'_>]) -> Frame {
        let mut output = Frame::black(width, height);

        for layer in layers {
            let opacity = layer.opacity.clamp(0.0, 1.0);
            if opacity <= 0.0 {
                continue;
            }

            let source = layer.frame;

            if !layer.placement.is_identity() {
                blend_transformed(&mut output, layer, opacity);
                continue;
            }

            let Some((dst_x, src_x, columns)) = overlap(layer.x, source.width(), width) else {
                continue;
            };
            let Some((dst_y, src_y, rows)) = overlap(layer.y, source.height(), height) else {
                continue;
            };

            blend_region(
                &mut output,
                source,
                (dst_x, dst_y),
                (src_x, src_y),
                (columns, rows),
                opacity,
                layer.blend,
            );
        }

        output
    }
}

/// One channel of `blend`: `over` is the layer's colour already times its
/// alpha, `under` the ground, both in `0..=255`. The formulas are the GPU's
/// fixed-function ones over premultiplied colour, so the two paths agree.
#[inline]
fn mix(blend: Blend, over: f32, under: f32, alpha: f32) -> f32 {
    match blend {
        Blend::Normal => over + under * (1.0 - alpha),
        Blend::Multiply => over * under / 255.0 + under * (1.0 - alpha),
        Blend::Screen => over * (1.0 - under / 255.0) + under,
        Blend::Add => over + under,
        Blend::Lighten => over.max(under),
        Blend::Darken => over.min(under),
    }
}

/// Draws one layer through its placement: inverse-mapped, bilinearly sampled.
///
/// Each covered output pixel is carried backwards through the placement into
/// source coordinates and sampled there. Inverse mapping is what makes the
/// result hole-free at any scale or angle; bilinear is the cheapest filter
/// that does not shimmer on motion. Only the transformed bounding box is
/// visited, so a small layer stays cheap on a large frame.
fn blend_transformed(output: &mut Frame, layer: &Layer<'_>, opacity: f32) {
    let source = layer.frame;
    let placement = layer.placement;
    // One factor per axis: the scale, and the stretch that axis carries.
    let scale_x = (placement.scale * placement.stretch_x).max(1e-6);
    let scale_y = (placement.scale * placement.stretch_y).max(1e-6);
    let (sin, cos) = placement.rotation.sin_cos();

    let src_w = source.width() as f32;
    let src_h = source.height() as f32;
    // The layer's centre in output space, after translation.
    let centre_x = layer.x as f32 + src_w / 2.0 + placement.translate_x;
    let centre_y = layer.y as f32 + src_h / 2.0 + placement.translate_y;

    // Bounding box of the transformed rectangle, clamped to the output.
    let half_w = src_w * scale_x / 2.0;
    let half_h = src_h * scale_y / 2.0;
    let reach_x = (half_w * cos.abs()) + (half_h * sin.abs());
    let reach_y = (half_w * sin.abs()) + (half_h * cos.abs());

    let x_from = ((centre_x - reach_x).floor().max(0.0)) as u32;
    let y_from = ((centre_y - reach_y).floor().max(0.0)) as u32;
    let x_to = ((centre_x + reach_x).ceil().min(output.width() as f32)) as u32;
    let y_to = ((centre_y + reach_y).ceil().min(output.height() as f32)) as u32;
    if x_from >= x_to || y_from >= y_to {
        return;
    }

    let dst_stride = output.width() as usize * BYTES_PER_PIXEL;
    let src_stride = source.width() as usize * BYTES_PER_PIXEL;
    let src_pixels = source.pixels();
    let dst_pixels = output.pixels_mut();

    for y in y_from..y_to {
        for x in x_from..x_to {
            // Sample at the pixel centre, mapped back into source space:
            // untranslate, unrotate, unscale, then re-origin at the corner.
            let dx = (x as f32 + 0.5) - centre_x;
            let dy = (y as f32 + 0.5) - centre_y;
            let sx = (dx * cos + dy * sin) / scale_x + src_w / 2.0 - 0.5;
            let sy = (-dx * sin + dy * cos) / scale_y + src_h / 2.0 - 0.5;

            if sx < -0.5 || sy < -0.5 || sx > src_w - 0.5 || sy > src_h - 0.5 {
                continue;
            }

            let x0 = sx.floor().max(0.0) as usize;
            let y0 = sy.floor().max(0.0) as usize;
            let x1 = (x0 + 1).min(source.width() as usize - 1);
            let y1 = (y0 + 1).min(source.height() as usize - 1);
            let fx = (sx - x0 as f32).clamp(0.0, 1.0);
            let fy = (sy - y0 as f32).clamp(0.0, 1.0);

            let mut sample = [0.0f32; 4];
            for (corner_x, corner_y, weight) in [
                (x0, y0, (1.0 - fx) * (1.0 - fy)),
                (x1, y0, fx * (1.0 - fy)),
                (x0, y1, (1.0 - fx) * fy),
                (x1, y1, fx * fy),
            ] {
                let at = corner_y * src_stride + corner_x * BYTES_PER_PIXEL;
                for channel in 0..4 {
                    sample[channel] += f32::from(src_pixels[at + channel]) * weight;
                }
            }

            let alpha = (sample[3] / 255.0) * opacity;
            if alpha <= 0.0 {
                continue;
            }

            let at = y as usize * dst_stride + x as usize * BYTES_PER_PIXEL;
            for channel in 0..3 {
                let over = sample[channel] * alpha;
                let under = f32::from(dst_pixels[at + channel]);
                dst_pixels[at + channel] = mix(layer.blend, over, under, alpha)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            dst_pixels[at + 3] = 255;
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

/// Source-over alpha blend of an aligned rectangle.
fn blend_region(
    output: &mut Frame,
    source: &Frame,
    (dst_x, dst_y): (u32, u32),
    (src_x, src_y): (u32, u32),
    (columns, rows): (u32, u32),
    opacity: f32,
    blend: Blend,
) {
    let src_stride = source.width() as usize * BYTES_PER_PIXEL;
    let dst_stride = output.width() as usize * BYTES_PER_PIXEL;
    let span = columns as usize * BYTES_PER_PIXEL;

    let src_pixels = source.pixels();
    let dst_pixels = output.pixels_mut();

    for row in 0..rows as usize {
        let src_offset = (src_y as usize + row) * src_stride + src_x as usize * BYTES_PER_PIXEL;
        let dst_offset = (dst_y as usize + row) * dst_stride + dst_x as usize * BYTES_PER_PIXEL;

        let src_row = &src_pixels[src_offset..src_offset + span];
        let dst_row = &mut dst_pixels[dst_offset..dst_offset + span];

        for (src_pixel, dst_pixel) in src_row
            .chunks_exact(BYTES_PER_PIXEL)
            .zip(dst_row.chunks_exact_mut(BYTES_PER_PIXEL))
        {
            let alpha = (f32::from(src_pixel[3]) / 255.0) * opacity;
            if alpha <= 0.0 {
                continue;
            }
            for channel in 0..3 {
                let over = f32::from(src_pixel[channel]) * alpha;
                let under = f32::from(dst_pixel[channel]);
                dst_pixel[channel] = mix(blend, over, under, alpha).round().clamp(0.0, 255.0) as u8;
            }
            dst_pixel[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    /// Multiply by white and screen with black both leave the ground as it
    /// was; add brightens it, darken cannot, lighten takes the brighter.
    #[test]
    fn the_blend_modes_do_what_their_names_say() {
        let over = |rgba: [u8; 4], blend: Blend| {
            let ground_frame = {
                let mut frame = Frame::black(2, 2);
                for pixel in frame.pixels_mut().chunks_exact_mut(4) {
                    pixel.copy_from_slice(&[100, 100, 100, 255]);
                }
                frame
            };
            let mut layer_frame = Frame::black(2, 2);
            for pixel in layer_frame.pixels_mut().chunks_exact_mut(4) {
                pixel.copy_from_slice(&rgba);
            }
            let out = CpuCompositor.composite(
                2,
                2,
                &[
                    Layer::new(&ground_frame),
                    Layer::new(&layer_frame).with_blend(blend),
                ],
            );
            out.pixels()[0]
        };
        assert_eq!(over([255, 255, 255, 255], Blend::Multiply), 100);
        assert_eq!(over([0, 0, 0, 255], Blend::Screen), 100);
        assert_eq!(over([255, 255, 255, 255], Blend::Screen), 255);
        assert_eq!(over([50, 50, 50, 255], Blend::Add), 150);
        assert_eq!(over([255, 255, 255, 255], Blend::Darken), 100);
        assert_eq!(over([30, 30, 30, 255], Blend::Lighten), 100);
        assert_eq!(over([200, 200, 200, 255], Blend::Lighten), 200);
        assert_eq!(over([200, 200, 200, 255], Blend::Normal), 200);
    }

    use super::*;

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        let mut frame = Frame::transparent(width, height);
        frame.fill(rgba);
        frame
    }

    #[test]
    fn no_layers_gives_opaque_black() {
        let frame = CpuCompositor.composite(2, 2, &[]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn an_opaque_layer_replaces_the_background() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        let frame = CpuCompositor.composite(2, 2, &[Layer::new(&red)]);
        assert_eq!(frame.pixel(1, 1), Some([255, 0, 0, 255]));
    }

    #[test]
    fn half_opacity_lands_halfway() {
        let white = solid(1, 1, [255, 255, 255, 255]);
        let frame = CpuCompositor.composite(1, 1, &[Layer::new(&white).with_opacity(0.5)]);
        assert_eq!(frame.pixel(0, 0), Some([128, 128, 128, 255]));
    }

    #[test]
    fn source_alpha_and_layer_opacity_multiply() {
        let half_alpha = solid(1, 1, [255, 255, 255, 128]);
        let frame = CpuCompositor.composite(1, 1, &[Layer::new(&half_alpha).with_opacity(0.5)]);
        // 128/255 * 0.5 ~= 0.251
        assert_eq!(frame.pixel(0, 0), Some([64, 64, 64, 255]));
    }

    #[test]
    fn later_layers_draw_on_top() {
        let red = solid(1, 1, [255, 0, 0, 255]);
        let blue = solid(1, 1, [0, 0, 255, 255]);
        let frame = CpuCompositor.composite(1, 1, &[Layer::new(&red), Layer::new(&blue)]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
    }

    #[test]
    fn a_transparent_layer_changes_nothing() {
        let clear = Frame::transparent(2, 2);
        let frame = CpuCompositor.composite(2, 2, &[Layer::new(&clear)]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn offset_layers_are_clipped_not_wrapped() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        // Placed so only its bottom-right pixel lands on the output's top-left.
        let frame = CpuCompositor.composite(2, 2, &[Layer::new(&red).at(-1, -1)]);
        assert_eq!(frame.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(
            frame.pixel(1, 1),
            Some([0, 0, 0, 255]),
            "must not wrap around"
        );
    }

    #[test]
    fn a_layer_entirely_off_screen_is_skipped() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        let frame = CpuCompositor.composite(2, 2, &[Layer::new(&red).at(50, 50)]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    }

    #[test]
    fn a_layer_larger_than_the_output_is_cropped() {
        let red = solid(8, 8, [255, 0, 0, 255]);
        let frame = CpuCompositor.composite(2, 2, &[Layer::new(&red)]);
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.pixel(1, 1), Some([255, 0, 0, 255]));
    }

    #[test]
    fn scaling_doubles_coverage() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        let placement = Placement {
            scale: 2.0,
            ..Placement::IDENTITY
        };
        let frame =
            CpuCompositor.composite(4, 4, &[Layer::new(&red).at(1, 1).with_placement(placement)]);
        // A 2x2 layer based at (1,1) scaled x2 about its centre covers the
        // whole 4x4 output.
        assert_eq!(frame.pixel(0, 0), Some([255, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 3), Some([255, 0, 0, 255]));
    }

    #[test]
    fn translation_moves_the_layer() {
        let red = solid(1, 1, [255, 0, 0, 255]);
        let placement = Placement {
            translate_x: 2.0,
            ..Placement::IDENTITY
        };
        let frame = CpuCompositor.composite(3, 1, &[Layer::new(&red).with_placement(placement)]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
        assert_eq!(frame.pixel(2, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn a_half_turn_swaps_the_ends() {
        let mut strip = Frame::transparent(2, 1);
        strip.set_pixel(0, 0, [255, 0, 0, 255]);
        strip.set_pixel(1, 0, [0, 0, 255, 255]);
        let placement = Placement {
            rotation: std::f32::consts::PI,
            ..Placement::IDENTITY
        };
        let frame = CpuCompositor.composite(2, 1, &[Layer::new(&strip).with_placement(placement)]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(frame.pixel(1, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn a_transformed_layer_is_clipped_at_the_frame_edge() {
        let red = solid(2, 2, [255, 0, 0, 255]);
        let placement = Placement {
            scale: 100.0,
            ..Placement::IDENTITY
        };
        let frame = CpuCompositor.composite(2, 2, &[Layer::new(&red).with_placement(placement)]);
        assert_eq!(frame.width(), 2);
        assert_eq!(frame.pixel(1, 1), Some([255, 0, 0, 255]));
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
