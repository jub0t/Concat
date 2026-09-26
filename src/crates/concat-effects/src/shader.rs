// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! A package's shader: what it declares, checked and laid out at load.
//!
//! A WGSL package writes two things and nothing else: a `Params` struct
//! whose fields are its knobs, and `fn effect(uv: vec2<f32>) -> vec4<f32>`,
//! the colour it wants at a point of the layer. Everything around that -
//! the bindings, the frame's size and time, the vertex stage, the mixing of
//! the result back over the untouched layer by intensity - is the host's,
//! and is stitched on here so every package shares one contract and no
//! package can bind things differently.
//!
//! The contract comes in editions, chosen by the manifest's `format` and
//! its `[wgsl]` space ([`Contract`]). From format 2 a shader samples the
//! compositor's working space as it is - linear light, unclipped, 1.0 the
//! white of an SDR picture - and is given the scene-linear library:
//! exposure in stops, a log space for what the eye judges, contrast and
//! saturation that never clip. A format 2 look that asks for the display
//! space is handed the same picture in the display encoding, still
//! unclipped, with the grading helpers it was drawn with made safe past
//! white, and its result is taken back into light before it is mixed. A
//! format 1 shader, written before the working space was linear, is
//! handed the gamma-encoded `0..1` picture it expects and the library it
//! was written against, and its result is taken back into light, so it
//! looks as it always did.
//!
//! The stitched module is parsed and validated when the package loads, the
//! same way a chain template is, so a broken shader is a load error and not
//! a black frame. Its `Params` struct is read back through naga for the
//! offset of every field, which is how a clip's settings become the bytes
//! of a uniform buffer without a package having to say anything about
//! layout.
//!
//! From format 2 a package may draw in several passes: each
//! `[[wgsl.pass]]` names a picture, `fn <target>(uv)` draws it, and the
//! passes after it - `effect`, the last, among them - read it through
//! `<target>_at(uv)`. The host binds each picture beside the layer, gives
//! every pass an entry point of its own (`fs_<target>`, then `fs_main`),
//! and refuses at load a pass that reads a picture not yet drawn. The
//! pictures hold what the passes return, untouched: only the last pass's
//! colour is taken back into light and mixed by intensity.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use concat_core::{Lut, RevealMap, ShaderPass, Stage, TransitionPass};

use crate::expr::{Expr, Value};
use crate::manifest::{MAX_CURVE_POINTS, MAX_SHRINK, Manifest, Param, ParamType, Pass, Space};

/// What an effect's shader binds: the layer at group 0, the host's frame
/// block and the package's own parameters at group 1, its table at group 2
/// and a title's reveal map at group 3.
const EFFECT_HEAD: &str = r#"// ── the host's half of the contract; see concat-effects/src/shader.rs ──
struct Frame {
    /// The layer's size in pixels.
    size: vec2<f32>,
    /// Seconds into the timeline.
    time: f32,
    /// How much of the effect to keep over the untouched layer.
    intensity: f32,
    /// Seconds since this layer's own clip began - zero at its first
    /// frame, however far into the timeline that is. A one-shot look
    /// times itself to this instead of `time`, so it plays the same
    /// whether the clip starts at zero or at the twenty-minute mark; a
    /// looping one can still read `time` for a phase nothing needs to
    /// reset. Layers with no single clip of their own - a treatment's
    /// stack, a synthesized ground - carry zero here always, which reads
    /// as "just started" forever; a look that only loops is unaffected.
    clip_time: f32,
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(1) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(1) var<uniform> params: Params;
@group(2) @binding(0) var lut_texture: texture_3d<f32>;
@group(2) @binding(1) var lut_sampler: sampler;
@group(3) @binding(0) var reveal_texture: texture_2d<f32>;
@group(3) @binding(1) var reveal_sampler: sampler;

/// A title's per-word reveal order at `uv`, 0..1 - the word painted there,
/// 0 for the first and 1 for the last, and 0 wherever no word was. A
/// pass over anything but a title reads the identity map here, which is
/// 0 everywhere: `reveal_order(uv) <= progress` is then always true, so a
/// package built on it is a no-op off a title with no special casing.
fn reveal_order(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(reveal_texture, reveal_sampler, uv, 0.0).r;
}
"#;

/// An effect's `sample` under the format 1 contract.
const EFFECT_SAMPLE_LEGACY: &str = r#"
/// The layer's colour at `uv`, straight alpha, in the gamma-encoded Rec. 709
/// `0..1` format 1 packages were written for (see `legacy_in`).
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return legacy_in(textureSample(source, source_sampler, uv));
}
"#;

/// An effect's `sample` under the scene-linear contract.
const EFFECT_SAMPLE_LINEAR: &str = r#"
/// The layer's colour at `uv`, straight alpha, in the working space as it
/// is: linear light on Rec. 709 primaries, extended - negative in a channel
/// outside Rec. 709's gamut, above 1.0 in a highlight - with 1.0 the white
/// of an SDR picture (203 nits).
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return into_space(textureSample(source, source_sampler, uv));
}

/// A colour of the layer as the texture holds it, in the space `sample`
/// hands it over in: here, as it is.
fn into_space(c: vec4<f32>) -> vec4<f32> {
    return c;
}
"#;

/// An effect's `sample` under the scene-linear contract in the display
/// space.
const EFFECT_SAMPLE_DISPLAY: &str = r#"
/// The layer's colour at `uv`, straight alpha, in the display encoding
/// (see `to_display`): the picture a look drawn by eye on an SDR display
/// reads, not clipped at white.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return into_space(textureSample(source, source_sampler, uv));
}

/// A colour of the layer as the texture holds it, in the space `sample`
/// hands it over in: here, the display encoding.
fn into_space(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(to_display(c.rgb), c.a);
}
"#;

/// An effect's `sample` under the scene-linear contract in log.
const EFFECT_SAMPLE_LOG: &str = r#"
/// The layer's colour at `uv`, straight alpha, in log (see `to_log`): the
/// space a table made for ACEScct reads, and where every level of light,
/// far past white, has a place between 0 and 1.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return into_space(textureSample(source, source_sampler, uv));
}

/// A colour of the layer as the texture holds it, in the space `sample`
/// hands it over in: here, log.
fn into_space(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(to_log(c.rgb), c.a);
}
"#;

/// What every format 2 effect can read besides `sample`, in whichever
/// space it asked for (`into_space`).
const EFFECT_SAMPLE_PREMULTIPLIED: &str = r#"
/// The layer at `uv` as `sample` reads it, but with each of the four
/// pixels it lies among premultiplied by its alpha before they are mixed,
/// and handed back premultiplied: what a blur or an average sums, so that
/// a transparent pixel's colour never bleeds into the pixels beside it.
/// Where the four are as opaque as each other - all of an opaque picture -
/// it is the one fetch. Read without derivatives, so the reading may turn
/// on what it reads.
fn sample_premultiplied(uv: vec2<f32>) -> vec4<f32> {
    let c = into_space(textureSampleLevel(source, source_sampler, uv, 0.0));
    if (abs(c.a - round(c.a)) < 1e-4) {
        return vec4<f32>(c.rgb * c.a, c.a);
    }
    let p = uv * frame.size - vec2<f32>(0.5);
    let f = fract(p);
    let first = vec2<i32>(floor(p));
    let last = vec2<i32>(frame.size) - vec2<i32>(1);
    var mixed = vec4<f32>(0.0);
    for (var y = 0; y < 2; y++) {
        for (var x = 0; x < 2; x++) {
            let at = clamp(first + vec2<i32>(x, y), vec2<i32>(0), last);
            let t = into_space(textureLoad(source, at, 0));
            let w = select(1.0 - f.x, f.x, x == 1) * select(1.0 - f.y, f.y, y == 1);
            mixed += vec4<f32>(t.rgb * t.a, t.a) * w;
        }
    }
    return mixed;
}
"#;

/// What a transition's shader binds: the outgoing picture and the incoming
/// one at group 0, the frame block - its spare slot the progress - and the
/// parameters at group 1, the table at group 2, and no reveal map.
const TRANSITION_HEAD: &str = r#"// ── the host's half of a transition; see concat-effects/src/shader.rs ──
struct Frame {
    /// The layer's size in pixels.
    size: vec2<f32>,
    /// Seconds into the timeline.
    time: f32,
    /// How far through the cut, 0 the outgoing picture, 1 the incoming one.
    progress: f32,
}

@group(0) @binding(0) var from_texture: texture_2d<f32>;
@group(0) @binding(1) var from_sampler: sampler;
@group(0) @binding(2) var to_texture: texture_2d<f32>;
@group(0) @binding(3) var to_sampler: sampler;
@group(1) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(1) var<uniform> params: Params;
@group(2) @binding(0) var lut_texture: texture_3d<f32>;
@group(2) @binding(1) var lut_sampler: sampler;

/// What the shared helpers read: the outgoing picture, so `soften` and its
/// like work on the from-side with no wiring by the author.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return from_at(uv);
}
"#;

/// A transition's two pictures under the format 1 contract.
const TRANSITION_SAMPLE_LEGACY: &str = r#"
/// The outgoing picture's colour at `uv`, straight alpha, gamma-encoded
/// Rec. 709 as format 1 packages were written for (see `legacy_in`).
fn from_at(uv: vec2<f32>) -> vec4<f32> {
    return legacy_in(textureSample(from_texture, from_sampler, uv));
}

/// The incoming picture's colour at `uv`, likewise.
fn to_at(uv: vec2<f32>) -> vec4<f32> {
    return legacy_in(textureSample(to_texture, to_sampler, uv));
}
"#;

/// A transition's two pictures under the scene-linear contract.
const TRANSITION_SAMPLE_LINEAR: &str = r#"
/// The outgoing picture's colour at `uv`, straight alpha, in the working
/// space as it is (see an effect's `sample`).
fn from_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(from_texture, from_sampler, uv);
}

/// The incoming picture's colour at `uv`, likewise.
fn to_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(to_texture, to_sampler, uv);
}
"#;

/// What every shader is given whatever it is and whichever contract it
/// keeps: they read only the frame's size and the table, which both heads
/// declare. (`hash` is each edition's own: see the libraries.)
const BASICS: &str = r#"
/// One pixel, as a fraction of the layer.
fn texel() -> vec2<f32> {
    return vec2<f32>(1.0, 1.0) / frame.size;
}

/// Luminance, by Rec. 709's weights: the primaries the working space is on.
fn luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// The package's look-up table applied to a colour in `0..1` - the identity
/// when the package ships none, so the call is always safe. Sampled at the
/// texel centres, so the table's ends land on black and white exactly.
fn lut(rgb: vec3<f32>) -> vec3<f32> {
    let n = f32(textureDimensions(lut_texture).x);
    let uvw = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * (n - 1.0) / n + vec3<f32>(0.5 / n);
    return textureSampleLevel(lut_texture, lut_sampler, uvw, 0.0).rgb;
}
"#;

/// The format 1 library: the way into and out of the gamma-encoded picture
/// those packages were written for, and the display-referred grading
/// helpers they are built from.
const LEGACY_LIBRARY: &str = r#"
// ── the grading library (format 1) ──

/// The compositor's working space - linear light on Rec. 709 primaries,
/// extended past `0..1`, with 1.0 the white of an SDR picture - brought into
/// the gamma-encoded `0..1` a package written before it was made for:
/// clipped to the range, then BT.1886's 2.4 gamma. The host samples through
/// this, so such a package sees the picture it always saw.
fn legacy_in(colour: vec4<f32>) -> vec4<f32> {
    let clipped = clamp(colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(pow(clipped, vec3<f32>(1.0 / 2.4)), colour.a);
}

/// A legacy package's result taken back into the working space: the 2.4
/// gamma undone.
fn legacy_out(colour: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(pow(max(colour.rgb, vec3<f32>(0.0)), vec3<f32>(2.4)), colour.a);
}

/// A hash in 0..1 from a point and a seed, for grain and dither.
fn hash(p: vec2<f32>, seed: f32) -> f32 {
    let q = vec3<f32>(p, seed);
    return fract(sin(dot(q, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}
//
// Every look is a few of these in different amounts, so they live here,
// once, rather than in each package. Each has an FFmpeg twin a manifest's
// chain can reach for: `saturation` is eq=saturation, `contrast` is
// eq=contrast, `fade`, `matte`, `s_curve` and `film_curve` are curves,
// `split_tone` and `tint_midtones` are colorbalance, `white_balance` is
// colortemperature, `vignette`
// is vignette, `mono` is colorchannelmixer, `hsl_band` is selectivecolor,
// `halation` is a split, gblur and screen blend.

/// Everything held to the displayable range.
fn clamp01(rgb: vec3<f32>) -> vec3<f32> {
    return clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
}

/// Saturation about luminance: 1 as shot, 0 grey, above 1 richer.
fn saturation(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3<f32>(luma(rgb)), rgb, amount);
}

/// Vibrance: the muted colours saturated more than the vivid ones, so a
/// face does not go orange before a sky goes blue. 0 as shot.
fn vibrance(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    let mx = max(max(rgb.r, rgb.g), rgb.b);
    let mn = min(min(rgb.r, rgb.g), rgb.b);
    return saturation(rgb, 1.0 + amount * (1.0 - (mx - mn)));
}

/// Contrast about middle grey: 1 as shot.
fn contrast(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    return (rgb - vec3<f32>(0.5)) * amount + vec3<f32>(0.5);
}

/// An S-curve: shadows down, highlights up, the midtones held. 0 as shot,
/// 1 the whole curve.
fn s_curve(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    let c = clamp01(rgb);
    return mix(c, c * c * (vec3<f32>(3.0) - 2.0 * c), amount);
}

/// A fade: the blacks lifted to `lift` and the rest compressed to fit,
/// which is what an old print and every faded look does.
fn fade(rgb: vec3<f32>, lift: f32) -> vec3<f32> {
    return rgb * (1.0 - lift) + vec3<f32>(lift);
}

/// Lift, gamma, gain: the three-way grade. Lift moves the shadows, gain
/// scales the highlights, gamma bends the midtones; (0, 1, 1) in every
/// channel is as shot.
fn lift_gamma_gain(rgb: vec3<f32>, lift: vec3<f32>, gamma: vec3<f32>, gain: vec3<f32>) -> vec3<f32> {
    let lifted = rgb * (vec3<f32>(1.0) - lift) + lift;
    let gained = clamp01(lifted * gain);
    return pow(gained, vec3<f32>(1.0) / max(gamma, vec3<f32>(0.01)));
}

/// How much of a pixel is shadow, highlight or midtone, by luminance:
/// the weights a tint on one end of the picture and not the other needs.
fn shadows(rgb: vec3<f32>) -> f32 {
    return 1.0 - smoothstep(0.0, 0.6, luma(rgb));
}
fn highlights(rgb: vec3<f32>) -> f32 {
    return smoothstep(0.4, 1.0, luma(rgb));
}
fn midtones(rgb: vec3<f32>) -> f32 {
    return 1.0 - min(abs(luma(rgb) - 0.5) * 2.0, 1.0);
}

/// A split tone: one tint into the shadows and another into the
/// highlights, each a signed offset per channel, so zero is as shot.
fn split_tone(rgb: vec3<f32>, shadow: vec3<f32>, highlight: vec3<f32>, amount: f32) -> vec3<f32> {
    return rgb + (shadow * shadows(rgb) + highlight * highlights(rgb)) * amount;
}

/// A tint over the midtones alone, the same signed offset.
fn tint_midtones(rgb: vec3<f32>, tint: vec3<f32>, amount: f32) -> vec3<f32> {
    return rgb + tint * midtones(rgb) * amount;
}

/// The colour of black-body light at `k` kelvin.
fn kelvin(k: f32) -> vec3<f32> {
    let t = clamp(k, 1000.0, 40000.0) / 100.0;
    var r: f32;
    var g: f32;
    var b: f32;
    if (t <= 66.0) {
        r = 1.0;
        g = clamp((99.4708 * log(t) - 161.1196) / 255.0, 0.0, 1.0);
        if (t <= 19.0) {
            b = 0.0;
        } else {
            b = clamp((138.5177 * log(t - 10.0) - 305.0448) / 255.0, 0.0, 1.0);
        }
    } else {
        r = clamp(329.6987 * pow(t - 60.0, -0.1332) / 255.0, 0.0, 1.0);
        g = clamp(288.1222 * pow(t - 60.0, -0.0755) / 255.0, 0.0, 1.0);
        b = 1.0;
    }
    return vec3<f32>(r, g, b);
}

/// White balance: the picture as if lit at `k` kelvin while the camera
/// was set for daylight. 6500 is as shot.
fn white_balance(rgb: vec3<f32>, k: f32) -> vec3<f32> {
    let tint = kelvin(k) / kelvin(6500.0);
    return rgb * (tint / max(luma(tint), 0.001));
}

/// A vignette: the corners darkened by `amount` from a clear middle.
fn vignette(rgb: vec3<f32>, uv: vec2<f32>, amount: f32) -> vec3<f32> {
    let d = distance(uv, vec2<f32>(0.5)) * 1.4142;
    return rgb * (1.0 - smoothstep(0.35, 1.1, d) * amount);
}

/// Black and white through a coloured filter: the channel weights, made
/// to sum to one. A red filter darkens skies and lightens skin.
fn mono(rgb: vec3<f32>, weights: vec3<f32>) -> vec3<f32> {
    let w = weights / max(weights.r + weights.g + weights.b, 0.001);
    return vec3<f32>(dot(rgb, w));
}

/// A matte: the blacks lifted to `black` and the whites pulled down to
/// `white`, the range between them kept in proportion. The print look
/// every faded, milky and instant-camera grade is built on; (0, 1) is as
/// shot. FFmpeg: curves with those two end points.
fn matte(rgb: vec3<f32>, black: f32, white: f32) -> vec3<f32> {
    return rgb * (white - black) + vec3<f32>(black);
}

/// A film curve: a toe that rolls the shadows into black by `toe` and a
/// shoulder that rolls the highlights into white by `shoulder`, both
/// `0..1`, the midtones left on the line. Unlike a contrast, it never
/// clips: it compresses the ends the way a negative does.
fn film_curve(rgb: vec3<f32>, toe: f32, shoulder: f32) -> vec3<f32> {
    let c = clamp01(rgb);
    let t = mix(c, c * c, vec3<f32>(toe) * (vec3<f32>(1.0) - c));
    return mix(t, vec3<f32>(1.0) - (vec3<f32>(1.0) - t) * (vec3<f32>(1.0) - t), vec3<f32>(shoulder) * t);
}

/// Every hue turned by `degrees`, brightness held: a rotation in the
/// YIQ plane, the same for every pixel.
fn hue_rotate(rgb: vec3<f32>, degrees: f32) -> vec3<f32> {
    let a = radians(degrees);
    let y = luma(rgb);
    let i = dot(rgb, vec3<f32>(0.596, -0.274, -0.322));
    let q = dot(rgb, vec3<f32>(0.211, -0.523, 0.312));
    let i2 = i * cos(a) - q * sin(a);
    let q2 = i * sin(a) + q * cos(a);
    return vec3<f32>(
        y + 0.956 * i2 + 0.621 * q2,
        y - 0.272 * i2 - 0.647 * q2,
        y - 1.106 * i2 + 1.703 * q2,
    );
}

/// One band of hues adjusted and the rest untouched: the band `width`
/// degrees around `centre` has its hue turned by `turn` degrees, its
/// saturation scaled by `sat` and its brightness by `lum`, weighted by
/// `hue_mask` so the edges of the band blend. What a grading panel's HSL
/// sliders do, and what keeps a sky change off a face. FFmpeg:
/// selectivecolor on the nearest of its six ranges.
fn hsl_band(rgb: vec3<f32>, centre: f32, width: f32, turn: f32, sat: f32, lum: f32) -> vec3<f32> {
    let w = hue_mask(rgb, centre, width);
    var out = hue_rotate(rgb, turn);
    out = saturation(out, sat);
    out = out * lum;
    return mix(rgb, out, w);
}

/// Halation: the brights above `threshold` gathered from `radius` pixels
/// around, tinted, and screened back over the picture by `amount`. The
/// glow around a lamp on film, and the bloom every soft look leans on.
/// FFmpeg: a split, a gblur and a screen blend.
fn halation(uv: vec2<f32>, rgb: vec3<f32>, threshold: f32, radius: f32, tint: vec3<f32>, amount: f32) -> vec3<f32> {
    let t = texel() * radius * 0.5;
    var sum = vec3<f32>(0.0);
    for (var y: i32 = -2; y <= 2; y++) {
        for (var x: i32 = -2; x <= 2; x++) {
            let s = sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
            let bright = smoothstep(threshold, 1.0, luma(s));
            sum += s * bright;
        }
    }
    let glow = clamp01(sum / 25.0 * tint * amount);
    return vec3<f32>(1.0) - (vec3<f32>(1.0) - rgb) * (vec3<f32>(1.0) - glow);
}

// ── targeting and texture: the taps a look takes around a pixel, and the
// bands of colour it singles out. FFmpeg twins: `hue_mask` is
// selectivecolor, `soften` is gblur, `grain_at` is noise, `edge_at` is
// edgedetect.

/// Hue in degrees, 0..360; 0 for a grey.
fn hue_of(rgb: vec3<f32>) -> f32 {
    let mx = max(max(rgb.r, rgb.g), rgb.b);
    let mn = min(min(rgb.r, rgb.g), rgb.b);
    let d = mx - mn;
    if (d < 0.0001) {
        return 0.0;
    }
    var h: f32;
    if (mx == rgb.r) {
        h = (rgb.g - rgb.b) / d;
    } else if (mx == rgb.g) {
        h = 2.0 + (rgb.b - rgb.r) / d;
    } else {
        h = 4.0 + (rgb.r - rgb.g) / d;
    }
    return fract(h / 6.0) * 360.0;
}

/// Chroma, 0..1: how far from grey.
fn chroma_of(rgb: vec3<f32>) -> f32 {
    return max(max(rgb.r, rgb.g), rgb.b) - min(min(rgb.r, rgb.g), rgb.b);
}

/// How much a pixel belongs to the hues within `width` degrees of
/// `centre`, weighted by chroma so a grey belongs to no band.
fn hue_mask(rgb: vec3<f32>, centre: f32, width: f32) -> f32 {
    let d = abs(fract((hue_of(rgb) - centre) / 360.0 + 0.5) * 360.0 - 180.0);
    return (1.0 - smoothstep(width * 0.5, width, d)) * smoothstep(0.0, 0.25, chroma_of(rgb));
}

/// The weight of skin: the orange band, a warm tan to a pale cheek.
fn skin_mask(rgb: vec3<f32>) -> f32 {
    return hue_mask(rgb, 25.0, 40.0);
}

/// The layer averaged over a square of taps `radius` pixels across: a
/// bloom, a soft denoise, the blur an unsharp mask subtracts.
fn soften(uv: vec2<f32>, radius: f32) -> vec3<f32> {
    let t = texel() * radius * 0.5;
    var sum = vec3<f32>(0.0);
    for (var y: i32 = -2; y <= 2; y++) {
        for (var x: i32 = -2; x <= 2; x++) {
            sum += sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
        }
    }
    return sum / 25.0;
}

/// Grain: noise that changes every frame, centred on zero, `amount` as a
/// fraction of the range. Seeded by the frame's time so the monitor and
/// the export show the same grain on the same frame.
fn grain_at(uv: vec2<f32>, amount: f32) -> vec3<f32> {
    let n = hash(uv * frame.size, fract(frame.time * 7.31)) - 0.5;
    return vec3<f32>(n * amount);
}

/// The strength of an edge at `uv`: Sobel on luminance, 0..1.
fn edge_at(uv: vec2<f32>) -> f32 {
    let t = texel();
    let tl = luma(sample(uv + vec2<f32>(-t.x, -t.y)).rgb);
    let tc = luma(sample(uv + vec2<f32>(0.0, -t.y)).rgb);
    let tr = luma(sample(uv + vec2<f32>(t.x, -t.y)).rgb);
    let ml = luma(sample(uv + vec2<f32>(-t.x, 0.0)).rgb);
    let mr = luma(sample(uv + vec2<f32>(t.x, 0.0)).rgb);
    let bl = luma(sample(uv + vec2<f32>(-t.x, t.y)).rgb);
    let bc = luma(sample(uv + vec2<f32>(0.0, t.y)).rgb);
    let br = luma(sample(uv + vec2<f32>(t.x, t.y)).rgb);
    let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    let gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
    return clamp(sqrt(gx * gx + gy * gy), 0.0, 1.0);
}
"#;

/// The display-space library (format 2, `space = "display"`): the grading
/// helpers the looks are built from, safe past white.
const DISPLAY_LIBRARY: &str = r#"
// ── the display-space library (format 2, `space = "display"`) ──
//
// A look drawn by eye on an SDR picture is maths on the display encoding,
// and this is the format 1 grading library for it, with one difference:
// nothing clips. Between black and white every helper computes what its
// format 1 twin did, so a look ported here reads as it did on SDR; past
// white a curve carries a highlight on rather than bending it back, and a
// blend treats it as the brightest thing there.

/// `over` screened onto `base`: both inverted, multiplied, inverted back -
/// light added the way two projectors add it. A level of `base` beyond
/// white stays beyond it by as much.
fn screen(base: vec3<f32>, over: vec3<f32>) -> vec3<f32> {
    let b = min(base, vec3<f32>(1.0));
    return vec3<f32>(1.0) - (vec3<f32>(1.0) - b) * (vec3<f32>(1.0) - over) + (base - b);
}

/// Saturation about luminance: 1 as shot, 0 grey, above 1 richer.
fn saturation(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3<f32>(luma(rgb)), rgb, amount);
}

/// Vibrance: the muted colours saturated more than the vivid ones, so a
/// face does not go orange before a sky goes blue. 0 as shot.
fn vibrance(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    let mx = max(max(rgb.r, rgb.g), rgb.b);
    let mn = min(min(rgb.r, rgb.g), rgb.b);
    return saturation(rgb, 1.0 + amount * max(1.0 - (mx - mn), 0.0));
}

/// Contrast about middle grey (0.5 here, 0.19 of the light): 1 as shot.
fn contrast(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    return (rgb - vec3<f32>(0.5)) * amount + vec3<f32>(0.5);
}

/// An S-curve: shadows down, highlights up, the midtones held. 0 as shot,
/// 1 the whole curve. The curve spans black to white; beyond them a level
/// is left where it is, so a highlight is not bent back down.
fn s_curve(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(c, c * c * (vec3<f32>(3.0) - 2.0 * c), amount) + (rgb - c);
}

/// A fade: the blacks lifted to `lift` and the rest compressed to fit,
/// which is what an old print and every faded look does.
fn fade(rgb: vec3<f32>, lift: f32) -> vec3<f32> {
    return rgb * (1.0 - lift) + vec3<f32>(lift);
}

/// Lift, gamma, gain: the three-way grade. Lift moves the shadows, gain
/// scales the highlights, gamma bends the midtones; (0, 1, 1) in every
/// channel is as shot.
fn lift_gamma_gain(rgb: vec3<f32>, lift: vec3<f32>, gamma: vec3<f32>, gain: vec3<f32>) -> vec3<f32> {
    let lifted = rgb * (vec3<f32>(1.0) - lift) + lift;
    let gained = max(lifted * gain, vec3<f32>(0.0));
    return pow(gained, vec3<f32>(1.0) / max(gamma, vec3<f32>(0.01)));
}

/// How much of a pixel is shadow, highlight or midtone, by luminance:
/// the weights a tint on one end of the picture and not the other needs.
fn shadows(rgb: vec3<f32>) -> f32 {
    return 1.0 - smoothstep(0.0, 0.6, luma(rgb));
}
fn highlights(rgb: vec3<f32>) -> f32 {
    return smoothstep(0.4, 1.0, luma(rgb));
}
fn midtones(rgb: vec3<f32>) -> f32 {
    return 1.0 - min(abs(luma(rgb) - 0.5) * 2.0, 1.0);
}

/// A split tone: one tint into the shadows and another into the
/// highlights, each a signed offset per channel, so zero is as shot.
fn split_tone(rgb: vec3<f32>, shadow: vec3<f32>, highlight: vec3<f32>, amount: f32) -> vec3<f32> {
    return rgb + (shadow * shadows(rgb) + highlight * highlights(rgb)) * amount;
}

/// A tint over the midtones alone, the same signed offset.
fn tint_midtones(rgb: vec3<f32>, tint: vec3<f32>, amount: f32) -> vec3<f32> {
    return rgb + tint * midtones(rgb) * amount;
}

/// The colour of black-body light at `k` kelvin.
fn kelvin(k: f32) -> vec3<f32> {
    let t = clamp(k, 1000.0, 40000.0) / 100.0;
    var r: f32;
    var g: f32;
    var b: f32;
    if (t <= 66.0) {
        r = 1.0;
        g = clamp((99.4708 * log(t) - 161.1196) / 255.0, 0.0, 1.0);
        if (t <= 19.0) {
            b = 0.0;
        } else {
            b = clamp((138.5177 * log(t - 10.0) - 305.0448) / 255.0, 0.0, 1.0);
        }
    } else {
        r = clamp(329.6987 * pow(t - 60.0, -0.1332) / 255.0, 0.0, 1.0);
        g = clamp(288.1222 * pow(t - 60.0, -0.0755) / 255.0, 0.0, 1.0);
        b = 1.0;
    }
    return vec3<f32>(r, g, b);
}

/// White balance: the picture as if lit at `k` kelvin while the camera
/// was set for daylight. 6500 is as shot.
fn white_balance(rgb: vec3<f32>, k: f32) -> vec3<f32> {
    let tint = kelvin(k) / kelvin(6500.0);
    return rgb * (tint / max(luma(tint), 0.001));
}

/// A vignette: the corners darkened by `amount` from a clear middle.
fn vignette(rgb: vec3<f32>, uv: vec2<f32>, amount: f32) -> vec3<f32> {
    let d = distance(uv, vec2<f32>(0.5)) * 1.4142;
    return rgb * (1.0 - smoothstep(0.35, 1.1, d) * amount);
}

/// Black and white through a coloured filter: the channel weights, made
/// to sum to one. A red filter darkens skies and lightens skin.
fn mono(rgb: vec3<f32>, weights: vec3<f32>) -> vec3<f32> {
    let w = weights / max(weights.r + weights.g + weights.b, 0.001);
    return vec3<f32>(dot(rgb, w));
}

/// A matte: the blacks lifted to `black` and the whites pulled down to
/// `white`, the range between them kept in proportion. The print look
/// every faded, milky and instant-camera grade is built on; (0, 1) is as
/// shot.
fn matte(rgb: vec3<f32>, black: f32, white: f32) -> vec3<f32> {
    return rgb * (white - black) + vec3<f32>(black);
}

/// A film curve: a toe that rolls the shadows into black by `toe` and a
/// shoulder that rolls the highlights into white by `shoulder`, both
/// `0..1`, the midtones left on the line. Unlike a contrast, it never
/// clips: it compresses the ends the way a negative does. The curve spans
/// black to white; a highlight beyond white is carried on above it.
fn film_curve(rgb: vec3<f32>, toe: f32, shoulder: f32) -> vec3<f32> {
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let t = mix(c, c * c, vec3<f32>(toe) * (vec3<f32>(1.0) - c));
    let curved = mix(t, vec3<f32>(1.0) - (vec3<f32>(1.0) - t) * (vec3<f32>(1.0) - t), vec3<f32>(shoulder) * t);
    return curved + (rgb - c);
}

/// Every hue turned by `degrees`, brightness held: a rotation in the
/// YIQ plane, the same for every pixel.
fn hue_rotate(rgb: vec3<f32>, degrees: f32) -> vec3<f32> {
    let a = radians(degrees);
    let y = luma(rgb);
    let i = dot(rgb, vec3<f32>(0.596, -0.274, -0.322));
    let q = dot(rgb, vec3<f32>(0.211, -0.523, 0.312));
    let i2 = i * cos(a) - q * sin(a);
    let q2 = i * sin(a) + q * cos(a);
    return vec3<f32>(
        y + 0.956 * i2 + 0.621 * q2,
        y - 0.272 * i2 - 0.647 * q2,
        y - 1.106 * i2 + 1.703 * q2,
    );
}

/// One band of hues adjusted and the rest untouched: the band `width`
/// degrees around `centre` has its hue turned by `turn` degrees, its
/// saturation scaled by `sat` and its brightness by `lum`, weighted by
/// `hue_mask` so the edges of the band blend. What a grading panel's HSL
/// sliders do, and what keeps a sky change off a face.
fn hsl_band(rgb: vec3<f32>, centre: f32, width: f32, turn: f32, sat: f32, lum: f32) -> vec3<f32> {
    let w = hue_mask(rgb, centre, width);
    var out = hue_rotate(rgb, turn);
    out = saturation(out, sat);
    out = out * lum;
    return mix(rgb, out, w);
}

/// Halation: the brights above `threshold` gathered from `radius` pixels
/// around, tinted, and screened back over the picture by `amount`. The
/// glow around a lamp on film, and the bloom every soft look leans on.
fn halation(uv: vec2<f32>, rgb: vec3<f32>, threshold: f32, radius: f32, tint: vec3<f32>, amount: f32) -> vec3<f32> {
    let t = texel() * radius * 0.5;
    var sum = vec3<f32>(0.0);
    for (var y: i32 = -2; y <= 2; y++) {
        for (var x: i32 = -2; x <= 2; x++) {
            let s = sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
            let bright = smoothstep(threshold, 1.0, luma(s));
            sum += s * bright;
        }
    }
    let glow = clamp(sum / 25.0 * tint * amount, vec3<f32>(0.0), vec3<f32>(1.0));
    return screen(rgb, glow);
}

// ── targeting and texture: the taps a look takes around a pixel, and the
// bands of colour it singles out.

/// Hue in degrees, 0..360; 0 for a grey.
fn hue_of(rgb: vec3<f32>) -> f32 {
    let mx = max(max(rgb.r, rgb.g), rgb.b);
    let mn = min(min(rgb.r, rgb.g), rgb.b);
    let d = mx - mn;
    if (d < 0.0001) {
        return 0.0;
    }
    var h: f32;
    if (mx == rgb.r) {
        h = (rgb.g - rgb.b) / d;
    } else if (mx == rgb.g) {
        h = 2.0 + (rgb.b - rgb.r) / d;
    } else {
        h = 4.0 + (rgb.r - rgb.g) / d;
    }
    return fract(h / 6.0) * 360.0;
}

/// Chroma, 0..1: how far from grey.
fn chroma_of(rgb: vec3<f32>) -> f32 {
    return max(max(rgb.r, rgb.g), rgb.b) - min(min(rgb.r, rgb.g), rgb.b);
}

/// How much a pixel belongs to the hues within `width` degrees of
/// `centre`, weighted by chroma so a grey belongs to no band.
fn hue_mask(rgb: vec3<f32>, centre: f32, width: f32) -> f32 {
    let d = abs(fract((hue_of(rgb) - centre) / 360.0 + 0.5) * 360.0 - 180.0);
    return (1.0 - smoothstep(width * 0.5, width, d)) * smoothstep(0.0, 0.25, chroma_of(rgb));
}

/// The weight of skin: the orange band, a warm tan to a pale cheek.
fn skin_mask(rgb: vec3<f32>) -> f32 {
    return hue_mask(rgb, 25.0, 40.0);
}

/// The layer averaged over a square of taps `radius` pixels across: a
/// bloom, a soft denoise, the blur an unsharp mask subtracts.
fn soften(uv: vec2<f32>, radius: f32) -> vec3<f32> {
    let t = texel() * radius * 0.5;
    var sum = vec3<f32>(0.0);
    for (var y: i32 = -2; y <= 2; y++) {
        for (var x: i32 = -2; x <= 2; x++) {
            sum += sample(uv + vec2<f32>(f32(x), f32(y)) * t).rgb;
        }
    }
    return sum / 25.0;
}

/// Grain: noise that changes every frame, centred on zero, `amount` as a
/// fraction of the range. Seeded by the pixel and the frame's time, so the
/// monitor and the export show the same grain on the same frame.
fn grain_at(uv: vec2<f32>, amount: f32) -> vec3<f32> {
    let n = hash(floor(uv * frame.size), fract(frame.time * 7.31)) - 0.5;
    return vec3<f32>(n * amount);
}

/// The strength of an edge at `uv`: Sobel on luminance, 0..1.
fn edge_at(uv: vec2<f32>) -> f32 {
    let t = texel();
    let tl = luma(sample(uv + vec2<f32>(-t.x, -t.y)).rgb);
    let tc = luma(sample(uv + vec2<f32>(0.0, -t.y)).rgb);
    let tr = luma(sample(uv + vec2<f32>(t.x, -t.y)).rgb);
    let ml = luma(sample(uv + vec2<f32>(-t.x, 0.0)).rgb);
    let mr = luma(sample(uv + vec2<f32>(t.x, 0.0)).rgb);
    let bl = luma(sample(uv + vec2<f32>(-t.x, t.y)).rgb);
    let bc = luma(sample(uv + vec2<f32>(0.0, t.y)).rgb);
    let br = luma(sample(uv + vec2<f32>(t.x, t.y)).rgb);
    let gx = (tr + 2.0 * mr + br) - (tl + 2.0 * ml + bl);
    let gy = (bl + 2.0 * bc + br) - (tl + 2.0 * tc + tr);
    return clamp(sqrt(gx * gx + gy * gy), 0.0, 1.0);
}
"#;

/// What every format 2 shader is given beside its library: the display
/// encoding - a look's space, and where a linear shader goes to do a sum
/// an eye drew - and a hash every GPU agrees on.
const FORMAT2_BASICS: &str = r#"
/// A hash in 0..1 from a point and a seed, for grain and dither: the bits
/// of all three mixed in integer arithmetic, so every GPU - and the
/// software one an export may run on - draws the same number, where the
/// last bits of a sine do not agree. Whole numbers make the steadiest
/// points: a pixel, a slice, a cell.
fn hash(p: vec2<f32>, seed: f32) -> f32 {
    var h = bitcast<u32>(p.x) ^ (bitcast<u32>(p.y) * 0x9e3779b9u) ^ (bitcast<u32>(seed) * 0x85ebca6bu);
    h = (h ^ (h >> 16u)) * 0x7feb352du;
    h = (h ^ (h >> 15u)) * 0x846ca68bu;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) / 16777216.0;
}

/// Light into the display encoding: BT.1886's 2.4 gamma, the curve an SDR
/// picture is written in, so SDR white is 1.0 and middle grey 0.49 - but
/// extended rather than clipped: a highlight climbs past 1.0, and a channel
/// negative outside Rec. 709's gamut stays negative.
fn to_display(rgb: vec3<f32>) -> vec3<f32> {
    return sign(rgb) * pow(abs(rgb), vec3<f32>(1.0 / 2.4));
}

/// The display encoding back into light.
fn from_display(encoded: vec3<f32>) -> vec3<f32> {
    return sign(encoded) * pow(abs(encoded), vec3<f32>(2.4));
}

/// A wheel's puck as the colour it pushes towards: the hue at its angle -
/// red at the top, then clockwise yellow, green, cyan, blue, magenta, as the
/// wheel is drawn - as far from grey as the puck is from the middle, with no
/// brightness of its own, so a push towards it turns a colour and leaves
/// its light. `wheel` is a wheel knob: its puck in x and y.
fn wheel_hue(wheel: vec3<f32>) -> vec3<f32> {
    let reach = min(length(wheel.xy), 1.0);
    if (reach < 1e-5) {
        return vec3<f32>(0.0);
    }
    let turn = fract(atan2(wheel.x, wheel.y) / 6.28318530718 + 1.0);
    let hue = clamp(abs(fract(vec3<f32>(turn) + vec3<f32>(1.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - vec3<f32>(3.0)) - vec3<f32>(1.0), vec3<f32>(0.0), vec3<f32>(1.0));
    let chroma = hue - vec3<f32>(luma(hue));
    return chroma / max(max(abs(chroma.r), abs(chroma.g)), abs(chroma.b)) * reach;
}

/// Lift, gamma and gain from three wheel knobs, on a display level: lift
/// moves black towards its colour and master and leaves white where it is,
/// gamma bends the midtones by its, gain scales every level by its. Between
/// black and white each is the classic three-way grade; past white, lift
/// and gamma carry a level on by as much as it is past, and gain scales it
/// like any other. At rest - pucks in the middle, masters at 0 - the
/// picture as it came.
fn grade_wheels(level: vec3<f32>, lift: vec3<f32>, gamma: vec3<f32>, gain: vec3<f32>) -> vec3<f32> {
    if (all(lift == vec3<f32>(0.0)) && all(gamma == vec3<f32>(0.0)) && all(gain == vec3<f32>(0.0))) {
        return level;
    }
    let black = wheel_hue(lift) * 0.25 + vec3<f32>(lift.z * 0.5);
    let bend = wheel_hue(gamma) * 0.5 + vec3<f32>(gamma.z);
    let scale = wheel_hue(gain) * 0.5 + vec3<f32>(gain.z);
    let below = min(level, vec3<f32>(1.0));
    var graded = below + black * (vec3<f32>(1.0) - below);
    if (any(bend != vec3<f32>(0.0))) {
        graded = sign(graded) * pow(abs(graded), exp2(-bend));
    }
    return (graded + (level - below)) * exp2(scale);
}

/// A curve knob at `level`: the curve through its points between black and
/// white, flat before its first point and after its last, and a level past
/// either end carried on past it by as much.
fn curve_at(curve: array<vec4<f32>, 8>, level: f32) -> f32 {
    var points = curve;
    let inside = clamp(level, 0.0, 1.0);
    let count = clamp(i32(points[0].w), 2, 8);
    var i = 0;
    for (var k = 1; k < 7; k++) {
        if (k < count - 1 && inside >= points[k].x) {
            i = k;
        }
    }
    let a = points[i];
    let b = points[i + 1];
    var y: f32;
    if (inside <= points[0].x) {
        y = points[0].y;
    } else if (inside >= points[count - 1].x) {
        y = points[count - 1].y;
    } else {
        let h = max(b.x - a.x, 1e-6);
        let s = (inside - a.x) / h;
        let s2 = s * s;
        let s3 = s2 * s;
        y = (2.0 * s3 - 3.0 * s2 + 1.0) * a.y + (s3 - 2.0 * s2 + s) * h * a.z
            + (3.0 * s2 - 2.0 * s3) * b.y + (s3 - s2) * h * b.z;
    }
    return y + (level - inside);
}

/// Whether a curve knob is the line from black to white, which leaves
/// every level where it was.
fn curve_is_straight(curve: array<vec4<f32>, 8>) -> bool {
    return curve[0].w < 2.5 && all(curve[0].xy == vec2<f32>(0.0)) && all(curve[1].xy == vec2<f32>(1.0));
}

/// Curves on a display level: the luma curve first - the level's luminance
/// moved along it and its colour scaled with it, so brightness changes and
/// hue and saturation do not - then each channel along its own.
fn grade_curves(level: vec3<f32>, bright: array<vec4<f32>, 8>, red: array<vec4<f32>, 8>, green: array<vec4<f32>, 8>, blue: array<vec4<f32>, 8>) -> vec3<f32> {
    if (curve_is_straight(bright) && curve_is_straight(red) && curve_is_straight(green) && curve_is_straight(blue)) {
        return level;
    }
    let y = luma(level);
    let moved = curve_at(bright, y);
    var c = select(level + vec3<f32>(moved - y), level * (moved / y), abs(y) > 1e-4);
    return vec3<f32>(curve_at(red, c.r), curve_at(green, c.g), curve_at(blue, c.b));
}

/// A colour held to what the working space's half floats store: a knob
/// pushed to its end makes the brightest light there is, never infinity,
/// which every pass after it would carry on as no number at all.
fn held(colour: vec4<f32>) -> vec4<f32> {
    return clamp(colour, vec4<f32>(-65504.0), vec4<f32>(65504.0));
}

/// The package's look-up table over a colour of the space its shader works
/// in - the display encoding, or log - whose table spans 0 to 1: a level
/// past either end is carried on past it by as much, where `lut` holds it
/// at the end, so a highlight past white keeps its place above the table's
/// white. The colour as it was when the package ships no table.
fn look(rgb: vec3<f32>) -> vec3<f32> {
    let inside = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return lut(inside) + (rgb - inside);
}
"#;

/// The format 2 library: helpers that hold for any level of light.
const LINEAR_LIBRARY: &str = r#"
// ── the scene-linear library (format 2) ──
//
// A format 2 shader samples the working space as it is, and nothing here
// clips it. A multiply - exposure, a white balance, a gain - belongs in
// light, where it is what a lens or a lamp would do. A judgement of the
// eye - contrast, a curve, a look - belongs in log, where a stop is the
// same distance at every level, so one setting reads the same on an SDR
// timeline and an HDR one.

/// Middle grey: 18 % of SDR white, the level a contrast turns about.
const MID_GREY: f32 = 0.18;

/// Light scaled by `stops`: 1 is twice the light, -1 half, 0 as shot.
fn exposure(rgb: vec3<f32>, stops: f32) -> vec3<f32> {
    return rgb * exp2(stops);
}

/// Light into log, on ACEScct's curve (Academy S-2016-001): above its toe a
/// stop is 1/17.52 of the scale at every level, middle grey sits at 0.414
/// and SDR white at 0.555; below 2^-7 a straight toe carries black, and the
/// negative channel of a colour outside Rec. 709, through without a pole.
fn to_log(rgb: vec3<f32>) -> vec3<f32> {
    let toe = rgb * 10.5402377416545 + vec3<f32>(0.0729055341958355);
    let curve = (log2(max(rgb, vec3<f32>(0.0078125))) + vec3<f32>(9.72)) / 17.52;
    return select(curve, toe, rgb <= vec3<f32>(0.0078125));
}

/// Log back into light: `to_log` undone, held to the largest value the
/// compositor's half floats store.
fn from_log(log: vec3<f32>) -> vec3<f32> {
    let toe = (log - vec3<f32>(0.0729055341958355)) / 10.5402377416545;
    let curve = min(exp2(log * 17.52 - vec3<f32>(9.72)), vec3<f32>(65504.0));
    return select(curve, toe, log <= vec3<f32>(0.155251141552511));
}

/// Contrast about middle grey, in stops: 1 as shot, 0 flat grey, 2 every
/// level twice as many stops from grey. Grey stays where it is, a highlight
/// above white is pushed or pulled like any other level, and however hard
/// the push, a level of light never goes below black: a channel with none
/// is left as it was.
fn contrast(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    let pushed = MID_GREY * pow(max(rgb, vec3<f32>(1e-10)) / MID_GREY, vec3<f32>(amount));
    return select(rgb, min(pushed, vec3<f32>(65504.0)), rgb > vec3<f32>(0.0));
}

/// Saturation about luminance, in light: 1 as shot, 0 grey, above 1
/// richer. A colour pushed past Rec. 709's gamut goes negative in a
/// channel rather than clipping, so turning it back down restores it.
fn saturation(rgb: vec3<f32>, amount: f32) -> vec3<f32> {
    return mix(vec3<f32>(luma(rgb)), rgb, amount);
}

/// CIE XYZ into cone responses, by Bradford's matrix: where a white balance
/// is worked out, as the eye adapts to the light.
const BRADFORD: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.8951, -0.7502, 0.0389),
    vec3<f32>(0.2664, 1.7135, -0.0685),
    vec3<f32>(-0.1614, 0.0367, 1.0296),
);

/// Light on Rec. 709's primaries into Bradford's cone responses, and back.
const CONES: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.4227252927, 0.0556997770, 0.0213826437),
    vec3<f32>(0.4913453244, 0.9615340509, 0.0876418678),
    vec3<f32>(0.0273579445, 0.0231838105, 0.9805081326),
);
const FROM_CONES: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(2.5380445168, -0.1460040576, -0.0422985104),
    vec3<f32>(-1.2932769979, 1.1166483055, -0.0716072203),
    vec3<f32>(-0.0402368843, -0.0223290262, 1.0227526883),
);

/// Where a black body at `kelvin` sits in CIE 1960's (u, v): Kim et al.'s
/// cubic fit to the Planckian locus, 1667 K to 25000 K.
fn planckian(kelvin: f32) -> vec2<f32> {
    let t = clamp(kelvin, 1667.0, 25000.0);
    let k = 1000.0 / t;
    var x: f32;
    if (t <= 4000.0) {
        x = ((-0.2661239 * k - 0.2343589) * k + 0.8776956) * k + 0.179910;
    } else {
        x = ((-3.0258469 * k + 2.1070379) * k + 0.2226347) * k + 0.240390;
    }
    var y: f32;
    if (t <= 2222.0) {
        y = ((-1.1063814 * x - 1.34811020) * x + 2.18555832) * x - 0.20219683;
    } else if (t <= 4000.0) {
        y = ((-0.9549476 * x - 1.37418593) * x + 2.09137015) * x - 0.16748867;
    } else {
        y = ((3.0817580 * x - 5.87338670) * x + 3.75112997) * x - 0.37001483;
    }
    return vec2<f32>(4.0 * x, 6.0 * y) / (-2.0 * x + 12.0 * y + 3.0);
}

/// The white of light at `kelvin`, moved `tint` across the locus towards
/// magenta - 100 is 0.07 in (u, v), minus towards green - as cone
/// responses.
fn white_cones(kelvin: f32, tint: f32) -> vec3<f32> {
    let here = planckian(kelvin);
    let along = normalize(planckian(kelvin * 1.01) - here);
    let across = vec2<f32>(along.y, -along.x);
    let magenta = select(across, -across, across.y > 0.0);
    let uv = here + magenta * (tint * 0.0007);
    let d = 2.0 * uv.x - 8.0 * uv.y + 4.0;
    let x = 3.0 * uv.x / d;
    let y = 2.0 * uv.y / d;
    return BRADFORD * vec3<f32>(x / y, 1.0, (1.0 - x - y) / y);
}

/// White balance: the picture as if lit at `kelvin` - lower warmer, 6500
/// as shot - and `tint` towards magenta, minus towards green, 0 as shot.
/// The eye's own adaptation, a von Kries scaling of Bradford's cone
/// responses from the light at 6500 K to the light asked for, in light,
/// so a highlight is balanced like any other level; a grey keeps its
/// brightness.
fn white_balance(rgb: vec3<f32>, kelvin: f32, tint: f32) -> vec3<f32> {
    let gains = white_cones(kelvin, tint) / white_cones(6500.0, 0.0);
    let adapt = FROM_CONES * mat3x3<f32>(CONES[0] * gains, CONES[1] * gains, CONES[2] * gains);
    return adapt * rgb / luma(adapt * vec3<f32>(1.0));
}
"#;

/// The vertex stage every shader draws with: one triangle over the whole
/// target, the clip trimming it to the square.
const VERTEX: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    var out: VsOut;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}
"#;

/// An effect's fragment under the format 1 contract: the package's colour
/// mixed over the untouched layer by intensity, then taken back into light.
const EFFECT_FRAGMENT_LEGACY: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = sample(in.uv);
    let treated = effect(in.uv);
    return legacy_out(mix(base, treated, clamp(frame.intensity, 0.0, 1.0)));
}
"#;

/// An effect's fragment under the scene-linear contract: the package's
/// colour mixed over the untouched layer by intensity, in light, and held
/// to what the working space's half floats store (see `held`).
const EFFECT_FRAGMENT_LINEAR: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = sample(in.uv);
    let treated = effect(in.uv);
    return held(mix(base, treated, clamp(frame.intensity, 0.0, 1.0)));
}
"#;

/// An effect's fragment in the display space: the package's colour taken
/// back into light, then mixed over the untouched layer by intensity there.
const EFFECT_FRAGMENT_DISPLAY: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSample(source, source_sampler, in.uv);
    let treated = effect(in.uv);
    let light = vec4<f32>(from_display(treated.rgb), treated.a);
    return held(mix(base, light, clamp(frame.intensity, 0.0, 1.0)));
}
"#;

/// An effect's fragment in log: the package's colour taken back into light,
/// then mixed over the untouched layer by intensity there.
const EFFECT_FRAGMENT_LOG: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = textureSample(source, source_sampler, in.uv);
    let treated = effect(in.uv);
    let light = vec4<f32>(from_log(treated.rgb), treated.a);
    return held(mix(base, light, clamp(frame.intensity, 0.0, 1.0)));
}
"#;

/// A transition's fragment under the format 1 contract: the package owns
/// the blend, so the pipeline does no mixing of its own.
const TRANSITION_FRAGMENT_LEGACY: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return legacy_out(transition(in.uv, frame.progress));
}
"#;

/// A transition's fragment under the scene-linear contract.
const TRANSITION_FRAGMENT_LINEAR: &str = r#"
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return transition(in.uv, frame.progress);
}
"#;

/// Which edition of the contract a package's shader is stitched into, by
/// the format its manifest declares; see the module doc.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Contract {
    /// Format 1: the picture gamma-encoded and clipped to `0..1`, the
    /// display-referred grading library, the result taken back into light.
    Legacy,
    /// Format 2 on: the working space as it is, and the scene-linear
    /// library.
    Linear,
    /// Format 2 on, `space = "display"`: the picture in the display
    /// encoding, unclipped, the display-space library, and the result
    /// taken back into light and mixed there.
    Display,
    /// Format 2 on, `space = "log"`: the picture in log (ACEScct), the
    /// scene-linear library, and the result taken back into light and mixed
    /// there - where a table made for ACEScct is read.
    Log,
}

impl Contract {
    /// The edition `manifest` asks for.
    pub fn of(manifest: &Manifest) -> Contract {
        let space = manifest
            .wgsl
            .as_ref()
            .map_or(Space::Linear, |wgsl| wgsl.space);
        match (manifest.scene_linear(), space) {
            (false, _) => Contract::Legacy,
            (true, Space::Linear) => Contract::Linear,
            (true, Space::Display) => Contract::Display,
            (true, Space::Log) => Contract::Log,
        }
    }

    /// Everything the host stitches after a package's body for `entry`:
    /// the head, the sampling, the basics, the library and the draw stages.
    fn host(self, entry: Entry) -> String {
        let (head, sampling, fragment) = match (entry, self) {
            (Entry::Effect, Contract::Legacy) => {
                (EFFECT_HEAD, EFFECT_SAMPLE_LEGACY, EFFECT_FRAGMENT_LEGACY)
            }
            (Entry::Effect, Contract::Linear) => {
                (EFFECT_HEAD, EFFECT_SAMPLE_LINEAR, EFFECT_FRAGMENT_LINEAR)
            }
            (Entry::Effect, Contract::Display) => {
                (EFFECT_HEAD, EFFECT_SAMPLE_DISPLAY, EFFECT_FRAGMENT_DISPLAY)
            }
            (Entry::Effect, Contract::Log) => (EFFECT_HEAD, EFFECT_SAMPLE_LOG, EFFECT_FRAGMENT_LOG),
            (Entry::Transition, Contract::Legacy) => (
                TRANSITION_HEAD,
                TRANSITION_SAMPLE_LEGACY,
                TRANSITION_FRAGMENT_LEGACY,
            ),
            // A transition has no [wgsl] table to ask for another space.
            (Entry::Transition, Contract::Linear | Contract::Display | Contract::Log) => (
                TRANSITION_HEAD,
                TRANSITION_SAMPLE_LINEAR,
                TRANSITION_FRAGMENT_LINEAR,
            ),
        };
        let library: &[&str] = match (entry, self) {
            (_, Contract::Legacy) => &[LEGACY_LIBRARY],
            (Entry::Effect, Contract::Display) => &[FORMAT2_BASICS, DISPLAY_LIBRARY],
            _ => &[FORMAT2_BASICS, LINEAR_LIBRARY],
        };
        let reading: &[&str] = match (entry, self) {
            (Entry::Effect, Contract::Linear | Contract::Display | Contract::Log) => {
                &[EFFECT_SAMPLE_PREMULTIPLIED]
            }
            _ => &[],
        };
        [
            &[head, sampling][..],
            reading,
            &[BASICS],
            library,
            &[VERTEX, fragment],
        ]
        .concat()
        .concat()
    }
}

/// Where one parameter lands in the uniform buffer.
#[derive(Clone, Debug, PartialEq)]
struct Slot {
    key: String,
    offset: usize,
    kind: ParamType,
}

/// A package's shader, stitched, checked and laid out.
#[derive(Clone, Debug)]
pub struct Shader {
    package: String,
    key: String,
    source: Arc<str>,
    slots: Vec<Slot>,
    span: usize,
    /// The passes before `effect`, for a package drawn in several.
    stages: Vec<StageRule>,
}

impl Shader {
    /// Stitches `body` into the host's contract - the edition the
    /// manifest's format asks for, with its passes - checks it, and reads
    /// the `Params` struct for where each of the manifest's parameters
    /// lands. Every declared parameter must be a field of the struct; a
    /// field the manifest does not declare is allowed and stays zero.
    pub fn compile(manifest: &Manifest, body: &str) -> Result<Shader, String> {
        let (source, slots, span) = stitch(manifest, body, Entry::Effect)?;
        let passes = manifest
            .wgsl
            .as_ref()
            .map_or(&[][..], |wgsl| wgsl.passes.as_slice());
        Ok(Shader {
            package: manifest.effect.id.clone(),
            key: pipeline_key(&manifest.effect.id, manifest.effect.version, &source),
            source,
            slots,
            span,
            stages: StageRule::of(manifest, passes)?,
        })
    }

    /// The stitched module.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The `Params` buffer for these resolved values, laid out to the
    /// struct. Every declared parameter is present in `values` by the time
    /// the catalogue calls this; anything missing reads as zero.
    pub fn params_bytes(&self, values: &BTreeMap<String, f64>, params: &[Param]) -> Vec<u8> {
        lay_params(&self.slots, self.span, values, params)
    }

    /// A pass over a layer with these values: the uniform buffer written
    /// from them, the values themselves for a renderer that reads by name,
    /// and the stages before the last, their pictures sized from them.
    pub fn pass(
        &self,
        values: &BTreeMap<String, f64>,
        params: &[Param],
        intensity: f32,
        lut: Option<Arc<Lut>>,
        reveal_map: Option<Arc<RevealMap>>,
    ) -> ShaderPass {
        ShaderPass {
            package: self.package.clone(),
            key: self.key.clone(),
            source: Arc::clone(&self.source),
            params: self.params_bytes(values, params),
            values: values.clone(),
            intensity,
            lut,
            reveal_map,
            stages: self.stages.iter().map(|rule| rule.stage(values)).collect(),
        }
    }
}

/// What the host binds at one slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Bound {
    /// A 2D float texture: a layer, a transition's picture, a reveal map.
    Picture,
    /// The 3D float texture of a look-up table.
    Table,
    /// A plain (non-comparison) sampler.
    Sampler,
    /// A uniform block: the frame, the parameters.
    Uniform,
}

impl Bound {
    fn name(self) -> &'static str {
        match self {
            Bound::Picture => "a 2D texture",
            Bound::Table => "a 3D texture",
            Bound::Sampler => "a sampler",
            Bound::Uniform => "a uniform block",
        }
    }
}

/// Where the first pass's picture is bound in group 0, after the layer and
/// its sampler; the next pass's is bound after it, and so on.
const FIRST_PICTURE: u32 = 2;

/// What the host provides at `(group, binding)` for `entry`, and so the
/// only thing a package may declare there: an effect's layer at (0,0)-(0,1),
/// the pictures of its `pictures` passes after it, and its reveal map at
/// group 3; a transition's two pictures at (0,0)-(0,3) and nothing at group
/// 3; the frame block and parameters at group 1 and the look-up table at
/// group 2 for both. A slot outside this, or one declared as something
/// else, is a pipeline the device cannot build.
fn provided(entry: Entry, pictures: usize, group: u32, binding: u32) -> Option<Bound> {
    let passes = FIRST_PICTURE..FIRST_PICTURE + pictures as u32;
    match (entry, group, binding) {
        (_, 0, 0) => Some(Bound::Picture),
        (_, 0, 1) => Some(Bound::Sampler),
        (Entry::Effect, 0, binding) if passes.contains(&binding) => Some(Bound::Picture),
        (Entry::Transition, 0, 2) => Some(Bound::Picture),
        (Entry::Transition, 0, 3) => Some(Bound::Sampler),
        (_, 1, 0) | (_, 1, 1) => Some(Bound::Uniform),
        (_, 2, 0) => Some(Bound::Table),
        (_, 2, 1) => Some(Bound::Sampler),
        (Entry::Effect, 3, 0) => Some(Bound::Picture),
        (Entry::Effect, 3, 1) => Some(Bound::Sampler),
        _ => None,
    }
}

/// What a global variable is declared as, in the host's terms.
fn declared(module: &naga::Module, global: &naga::GlobalVariable) -> Option<Bound> {
    match &module.types[global.ty].inner {
        naga::TypeInner::Image {
            dim,
            arrayed: false,
            class:
                naga::ImageClass::Sampled {
                    kind: naga::ScalarKind::Float,
                    multi: false,
                },
        } => match dim {
            naga::ImageDimension::D2 => Some(Bound::Picture),
            naga::ImageDimension::D3 => Some(Bound::Table),
            _ => None,
        },
        naga::TypeInner::Sampler { comparison: false } => Some(Bound::Sampler),
        _ if global.space == naga::AddressSpace::Uniform => Some(Bound::Uniform),
        _ => None,
    }
}

/// What a package may not do, however well it parses: bind anything the
/// host did not declare, bind a slot as something other than what the
/// host puts there, or loop without an end. A community shader runs on
/// the person's GPU with the host's rights, and a loop with no bound
/// hangs the device for every process on the machine; a binding the host
/// does not know is one it cannot serve. Caught at load, where a broken
/// package is a load error and not a black frame.
fn budget(module: &naga::Module, entry: Entry, pictures: usize) -> Result<(), String> {
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = &global.binding else {
            continue;
        };
        let Some(wanted) = provided(entry, pictures, binding.group, binding.binding) else {
            return Err(format!(
                "the shader binds @group({}) @binding({}), which the host does not provide",
                binding.group, binding.binding
            ));
        };
        if declared(module, global) != Some(wanted) {
            return Err(format!(
                "the shader binds @group({}) @binding({}) as something other than {}, which is what the host provides there",
                binding.group,
                binding.binding,
                wanted.name()
            ));
        }
    }
    for (_, function) in module.functions.iter() {
        bounded(&function.body)?;
    }
    for entry in &module.entry_points {
        bounded(&entry.function.body)?;
    }
    Ok(())
}

/// Every loop in `block` has a way out: a `break` in its body or a
/// `break if` in its continuing block.
fn bounded(block: &naga::Block) -> Result<(), String> {
    for statement in block.iter() {
        match statement {
            naga::Statement::Loop {
                body,
                continuing,
                break_if,
            } => {
                if break_if.is_none() && !breaks(body) {
                    return Err(
                        "the shader has a loop with no break: it would never end".to_owned()
                    );
                }
                bounded(body)?;
                bounded(continuing)?;
            }
            naga::Statement::Block(inner) => bounded(inner)?,
            naga::Statement::If { accept, reject, .. } => {
                bounded(accept)?;
                bounded(reject)?;
            }
            naga::Statement::Switch { cases, .. } => {
                for case in cases {
                    bounded(&case.body)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Whether `block` breaks out of the loop it is the body of, at any
/// depth short of a nested loop, whose breaks are its own.
fn breaks(block: &naga::Block) -> bool {
    block.iter().any(|statement| match statement {
        naga::Statement::Break => true,
        naga::Statement::Block(inner) => breaks(inner),
        naga::Statement::If { accept, reject, .. } => breaks(accept) || breaks(reject),
        naga::Statement::Switch { cases, .. } => cases.iter().any(|case| breaks(&case.body)),
        _ => false,
    })
}

/// What the host stitches for a package's passes: each pass's picture bound
/// in group 0 after the layer, read through `<target>_at` and measured by
/// `<target>_texel`, and the entry point that draws it. Nothing for a
/// package drawn in one pass.
fn passes_host(passes: &[Pass]) -> String {
    use std::fmt::Write as _;
    let mut host = String::new();
    if passes.is_empty() {
        return host;
    }
    host.push_str("\n// ── the passes' pictures; see concat-effects/src/shader.rs ──\n");
    for (index, pass) in passes.iter().enumerate() {
        let target = &pass.target;
        let binding = FIRST_PICTURE + index as u32;
        write!(
            host,
            r#"
@group(0) @binding({binding}) var {target}_picture: texture_2d<f32>;

/// The picture the `{target}` pass drew, at `uv`: what its function
/// returned, straight, bilinearly filtered between its pixels.
fn {target}_at(uv: vec2<f32>) -> vec4<f32> {{
    return textureSampleLevel({target}_picture, source_sampler, uv, 0.0);
}}

/// One pixel of the `{target}` pass's picture, as a fraction of it.
fn {target}_texel() -> vec2<f32> {{
    return vec2<f32>(1.0) / vec2<f32>(textureDimensions({target}_picture));
}}

@fragment
fn fs_{target}(in: VsOut) -> @location(0) vec4<f32> {{
    return {target}(in.uv);
}}
"#
        )
        .expect("a String takes every write");
    }
    host
}

/// Every pass reads only pictures drawn before it, and every picture is
/// read by a pass after it. A pass reading its own picture, or one not
/// drawn yet, would read a blank the host binds in its place; a picture no
/// pass reads is work for nothing. What a pass reads is every picture its
/// function, or anything that calls, touches.
fn read_in_order(module: &naga::Module, passes: &[Pass]) -> Result<(), String> {
    if passes.is_empty() {
        return Ok(());
    }
    let pictures: HashMap<naga::Handle<naga::GlobalVariable>, usize> = module
        .global_variables
        .iter()
        .filter_map(|(handle, global)| {
            let binding = global.binding.as_ref()?;
            let index = binding.binding.checked_sub(FIRST_PICTURE)? as usize;
            (binding.group == 0 && index < passes.len()).then_some((handle, index))
        })
        .collect();
    let mut unread: BTreeSet<usize> = (0..passes.len()).collect();
    let drawers = passes
        .iter()
        .map(|pass| pass.target.as_str())
        .chain([Entry::Effect.name()]);
    for (index, name) in drawers.enumerate() {
        let Some((function, _)) = module
            .functions
            .iter()
            .find(|(_, function)| function.name.as_deref() == Some(name))
        else {
            return Err(format!(
                "the shader declares no `fn {name}(uv: vec2<f32>) -> vec4<f32>`"
            ));
        };
        let read = pictures_read(module, function, &pictures);
        if let Some(&later) = read.range(index..).next() {
            return Err(if later == index {
                format!("the pass drawing `{name}` reads its own picture")
            } else {
                format!(
                    "the pass drawing `{name}` reads `{}`, which is drawn after it",
                    passes[later].target
                )
            });
        }
        unread.retain(|picture| !read.contains(picture));
    }
    match unread.first() {
        Some(&picture) => Err(format!(
            "the pass `{}` draws a picture no pass after it reads",
            passes[picture].target
        )),
        None => Ok(()),
    }
}

/// The pass pictures `function` reads: every one whose texture it, or any
/// function it calls, names.
fn pictures_read(
    module: &naga::Module,
    function: naga::Handle<naga::Function>,
    pictures: &HashMap<naga::Handle<naga::GlobalVariable>, usize>,
) -> BTreeSet<usize> {
    let mut read = BTreeSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut waiting = vec![function];
    while let Some(handle) = waiting.pop() {
        if !seen.insert(handle) {
            continue;
        }
        let function = &module.functions[handle];
        for (_, expression) in function.expressions.iter() {
            if let naga::Expression::GlobalVariable(global) = expression
                && let Some(&picture) = pictures.get(global)
            {
                read.insert(picture);
            }
        }
        calls(&function.body, &mut waiting);
    }
    read
}

/// Every function `block` calls, at any depth, pushed onto `into`.
fn calls(block: &naga::Block, into: &mut Vec<naga::Handle<naga::Function>>) {
    for statement in block.iter() {
        match statement {
            naga::Statement::Call { function, .. } => into.push(*function),
            naga::Statement::Block(inner) => calls(inner, into),
            naga::Statement::If { accept, reject, .. } => {
                calls(accept, into);
                calls(reject, into);
            }
            naga::Statement::Switch { cases, .. } => {
                for case in cases {
                    calls(&case.body, into);
                }
            }
            naga::Statement::Loop {
                body, continuing, ..
            } => {
                calls(body, into);
                calls(continuing, into);
            }
            _ => {}
        }
    }
}

/// A pass before `effect`, as the shader keeps it: the entry point that
/// draws it, and the expressions over the knobs its shrink is worked out
/// from, across and down.
#[derive(Clone, Debug)]
struct StageRule {
    entry: String,
    shrink: [Expr; 2],
}

impl StageRule {
    /// The rules for `passes`, their shrinks parsed and read only for the
    /// knobs `manifest` declares, and worked out at every knob's default,
    /// minimum and maximum, so an expression that fails is a load error
    /// and never a silently full-sized picture.
    fn of(manifest: &Manifest, passes: &[Pass]) -> Result<Vec<StageRule>, String> {
        let knobs: Vec<&str> = manifest
            .params
            .iter()
            .filter(|param| param.kind != ParamType::Point)
            .map(|param| param.key.as_str())
            .collect();
        let rules = passes
            .iter()
            .map(|pass| {
                let parse = |text: &str| {
                    let expr = Expr::parse(text).map_err(|error| {
                        format!("pass `{}`: shrink `{text}`: {error}", pass.target)
                    })?;
                    let mut names = Vec::new();
                    expr.names(&mut names);
                    match names.iter().find(|name| !knobs.contains(&name.as_str())) {
                        Some(name) => Err(format!(
                            "pass `{}`: shrink `{text}` reads `{name}`, which is not a knob",
                            pass.target
                        )),
                        None => Ok(expr),
                    }
                };
                let [across, down] = match &pass.shrink {
                    Some([across, down]) => [across.as_str(), down.as_str()],
                    None => ["1", "1"],
                };
                Ok(StageRule {
                    entry: format!("fs_{}", pass.target),
                    shrink: [parse(across)?, parse(down)?],
                })
            })
            .collect::<Result<Vec<StageRule>, String>>()?;
        for (at, pick) in [
            (
                "default",
                (|param: &Param| param.default) as fn(&Param) -> f64,
            ),
            ("minimum", |param: &Param| param.min),
            ("maximum", |param: &Param| param.max),
        ] {
            let values: BTreeMap<String, f64> = manifest
                .params
                .iter()
                .map(|param| (param.key.clone(), pick(param)))
                .collect();
            for (rule, pass) in rules.iter().zip(passes) {
                for expr in &rule.shrink {
                    shrink_of(expr, &values).map_err(|error| {
                        format!(
                            "pass `{}`: shrink at every knob's {at}: {error}",
                            pass.target
                        )
                    })?;
                }
            }
        }
        Ok(rules)
    }

    /// The stage for these resolved knob values.
    fn stage(&self, values: &BTreeMap<String, f64>) -> Stage {
        Stage {
            entry: self.entry.clone(),
            shrink: self
                .shrink
                .each_ref()
                .map(|expr| shrink_of(expr, values).unwrap_or(1)),
        }
    }
}

/// A shrink worked out from the knobs, rounded down to a power of two from
/// 1 to [`MAX_SHRINK`]: a few sizes a layer's pictures can be, so a knob
/// that rides does not ask the pool for a new size every frame. Below 2, or
/// not a number at all, is the layer's own size.
fn shrink_of(expr: &Expr, values: &BTreeMap<String, f64>) -> Result<u32, String> {
    let env: BTreeMap<String, Value> = values
        .iter()
        .map(|(key, value)| (key.clone(), Value::Float(*value)))
        .collect();
    let shrink = match expr.eval(&env).map_err(|error| error.to_string())? {
        Value::Int(whole) => whole as f64,
        Value::Float(real) => real,
        Value::Text(_) => return Err("text where a number was needed".to_owned()),
    };
    if shrink.is_nan() || shrink < 2.0 {
        return Ok(1);
    }
    Ok(1 << shrink.min(f64::from(MAX_SHRINK)).log2().floor() as u32)
}

/// What a compiled pipeline is cached under: the package's id and version,
/// and a fingerprint of the stitched source, so a shader edited in place
/// without a version bump still gets a pipeline of its own rather than
/// the stale one a running compositor holds.
fn pipeline_key(id: &str, version: u32, source: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut hasher);
    format!("{id}@{version}#{:016x}", hasher.finish())
}

fn is_f32(scalar: &naga::Scalar) -> bool {
    scalar.kind == naga::ScalarKind::Float && scalar.width == 4
}

/// Which entry a stitched module declares: the two shaders share everything
/// but their entry function's name and signature.
#[derive(Clone, Copy)]
enum Entry {
    Effect,
    Transition,
}

impl Entry {
    /// The function name the body must declare.
    fn name(self) -> &'static str {
        match self {
            Entry::Effect => "effect",
            Entry::Transition => "transition",
        }
    }

    /// The signature named in the "no such function" error.
    fn signature(self) -> &'static str {
        match self {
            Entry::Effect => "fn effect(uv: vec2<f32>) -> vec4<f32>",
            Entry::Transition => "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32>",
        }
    }
}

/// Stitches a package `body` into the host's contract for `entry`, in the
/// edition its manifest's format asks for, parses and validates the result
/// with naga, and reads its `Params` struct for where each declared
/// parameter lands. Shared by [`Shader`] and [`TransitionShader`], which
/// differ only in the host's half and the entry function. Returns the
/// finished module, the parameter slots in declaration order, and the
/// padded uniform span.
fn stitch(
    manifest: &Manifest,
    body: &str,
    entry: Entry,
) -> Result<(Arc<str>, Vec<Slot>, usize), String> {
    let passes: &[Pass] = match (entry, &manifest.wgsl) {
        (Entry::Effect, Some(wgsl)) => &wgsl.passes,
        _ => &[],
    };
    let declares_params = body
        .split("struct")
        .skip(1)
        .any(|rest| rest.trim_start().starts_with("Params"));
    if !body.contains(&format!("fn {}", entry.name())) {
        return Err(format!("the shader declares no `{}`", entry.signature()));
    }
    if let Some(pass) = passes
        .iter()
        .find(|pass| !body.contains(&format!("fn {}", pass.target)))
    {
        return Err(format!(
            "the shader declares no `fn {}(uv: vec2<f32>) -> vec4<f32>` to draw the pass `{}`",
            pass.target, pass.target
        ));
    }
    let host = Contract::of(manifest).host(entry) + &passes_host(passes);
    let mut source = String::with_capacity(host.len() + body.len() + 64);
    if !declares_params {
        // A package with no knobs still has to bind something.
        source.push_str("struct Params { _unused: f32 }\n");
    }
    source.push_str(body);
    source.push('\n');
    source.push_str(&host);

    let module =
        naga::front::wgsl::parse_str(&source).map_err(|error| error.emit_to_string(&source))?;
    // The baseline capabilities and nothing more: a shader that needs an
    // extension is refused here, by name, rather than by whichever device
    // it first meets.
    let mut validator = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    );
    validator
        .validate(&module)
        .map_err(|error| error.emit_to_string(&source))?;
    if !module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(entry.name()))
    {
        return Err(format!("the shader declares no `{}`", entry.signature()));
    }
    budget(&module, entry, passes.len())?;
    read_in_order(&module, passes)?;

    let (members, span) = module
        .types
        .iter()
        .find_map(|(_, ty)| match (&ty.name, &ty.inner) {
            (Some(name), naga::TypeInner::Struct { members, span }) if name == "Params" => {
                Some((members.clone(), *span as usize))
            }
            _ => None,
        })
        .ok_or_else(|| "the shader declares no `struct Params`".to_owned())?;

    let mut slots = Vec::new();
    for param in &manifest.params {
        let member = members
            .iter()
            .find(|member| member.name.as_deref() == Some(param.key.as_str()))
            .ok_or_else(|| format!("`Params` has no field `{}`", param.key))?;
        let inner = &module.types[member.ty].inner;
        // What the field holds: f32s a vector wide, or for a curve that
        // many vec4s.
        let (wanted, name) = match param.kind {
            ParamType::Point => (Width::Vector(2), "a vec2<f32>"),
            ParamType::Wheel => (Width::Vector(3), "a vec3<f32>"),
            ParamType::Color => (Width::Vector(4), "a vec4<f32>"),
            ParamType::Curve => (
                Width::Points(MAX_CURVE_POINTS as u32),
                "an array<vec4<f32>, 8>",
            ),
            _ => (Width::Vector(1), "an f32"),
        };
        let width = match inner {
            naga::TypeInner::Scalar(scalar) if is_f32(scalar) => Width::Vector(1),
            naga::TypeInner::Vector { size, scalar } if is_f32(scalar) => {
                Width::Vector(*size as u32)
            }
            naga::TypeInner::Array {
                base,
                size: naga::ArraySize::Constant(count),
                stride: 16,
            } if matches!(
                module.types[*base].inner,
                naga::TypeInner::Vector { size: naga::VectorSize::Quad, scalar } if is_f32(&scalar)
            ) =>
            {
                Width::Points(count.get())
            }
            _ => Width::Vector(0),
        };
        if width != wanted {
            return Err(format!("`Params.{}` must be {name}", param.key));
        }
        slots.push(Slot {
            key: param.key.clone(),
            offset: member.offset as usize,
            kind: param.kind,
        });
    }

    let span = span.max(ShaderPass::MIN_PARAMS).div_ceil(16) * 16;
    Ok((Arc::from(source), slots, span))
}

/// The `Params` uniform for `values`, laid out to `slots` over a buffer of
/// `span` bytes. Shared by both shaders, which store parameters identically.
fn lay_params(
    slots: &[Slot],
    span: usize,
    values: &BTreeMap<String, f64>,
    params: &[Param],
) -> Vec<u8> {
    let mut bytes = vec![0u8; span];
    let mut put = |offset: usize, value: f64| {
        let at = offset..offset + 4;
        if at.end <= bytes.len() {
            bytes[at].copy_from_slice(&(value as f32).to_le_bytes());
        }
    };
    let default_of = |key: &str| {
        params
            .iter()
            .find(|param| param.key == key)
            .map_or(0.0, |param| param.default)
    };
    for slot in slots {
        match slot.kind {
            ParamType::Wheel => {
                let sub = |axis: &str| values.get(&format!("{}.{axis}", slot.key)).copied();
                put(slot.offset, sub("x").unwrap_or(0.0));
                put(slot.offset + 4, sub("y").unwrap_or(0.0));
                put(
                    slot.offset + 8,
                    sub("m").unwrap_or_else(|| default_of(&slot.key)),
                );
            }
            ParamType::Curve => {
                let points = curve_points(values, &slot.key);
                let slopes = monotone_slopes(&points);
                for (index, (&(x, y), slope)) in points.iter().zip(slopes).enumerate() {
                    let at = slot.offset + index * 16;
                    put(at, x);
                    put(at + 4, y);
                    put(at + 8, slope);
                    put(at + 12, points.len() as f64);
                }
            }
            ParamType::Point => {
                put(
                    slot.offset,
                    values
                        .get(&format!("{}.x", slot.key))
                        .copied()
                        .unwrap_or(0.5),
                );
                put(
                    slot.offset + 4,
                    values
                        .get(&format!("{}.y", slot.key))
                        .copied()
                        .unwrap_or(0.5),
                );
            }
            ParamType::Color => {
                // Packed RGBA in one number, as the document stores it.
                let packed = values.get(&slot.key).copied().unwrap_or(0.0).max(0.0) as u32;
                for (index, shift) in [24u32, 16, 8, 0].into_iter().enumerate() {
                    put(
                        slot.offset + index * 4,
                        f64::from((packed >> shift) & 0xff) / 255.0,
                    );
                }
            }
            _ => {
                let fallback = params
                    .iter()
                    .find(|param| param.key == slot.key)
                    .map(|param| param.default)
                    .unwrap_or(0.0);
                put(
                    slot.offset,
                    values.get(&slot.key).copied().unwrap_or(fallback),
                );
            }
        }
    }
    bytes
}

/// How many f32s a `Params` field holds: a scalar or vector's width, or a
/// curve's points.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Width {
    Vector(u32),
    Points(u32),
}

/// A curve's points as the document stores them under `key`, in the unit
/// square and in order along it, at most [`MAX_CURVE_POINTS`]; a curve with
/// fewer than two is the straight line from black to white. Two points at
/// one place along the curve are one, the later.
pub fn curve_points(values: &BTreeMap<String, f64>, key: &str) -> Vec<(f64, f64)> {
    let mut points: Vec<(f64, f64)> = (0..MAX_CURVE_POINTS)
        .filter_map(|n| {
            let x = values.get(&format!("{key}.{n}.x"))?;
            let y = values.get(&format!("{key}.{n}.y"))?;
            (x.is_finite() && y.is_finite()).then(|| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)))
        })
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0));
    points.dedup_by(|later, earlier| {
        let same = (later.0 - earlier.0).abs() < 1e-6;
        if same {
            *earlier = *later;
        }
        same
    });
    if points.len() < 2 {
        return vec![(0.0, 0.0), (1.0, 1.0)];
    }
    points
}

/// The slope a curve leaves each of its points at: Fritsch and Carlson's
/// (1980), so a curve through points that climb climbs everywhere between
/// them, and one that turns never overshoots its points.
pub fn monotone_slopes(points: &[(f64, f64)]) -> Vec<f64> {
    let n = points.len();
    if n < 2 {
        return vec![1.0; n];
    }
    let secants: Vec<f64> = points
        .windows(2)
        .map(|pair| (pair[1].1 - pair[0].1) / (pair[1].0 - pair[0].0).max(1e-9))
        .collect();
    let mut slopes = vec![0.0; n];
    slopes[0] = secants[0];
    slopes[n - 1] = secants[n - 2];
    for k in 1..n - 1 {
        slopes[k] = if secants[k - 1] * secants[k] > 0.0 {
            (secants[k - 1] + secants[k]) / 2.0
        } else {
            0.0
        };
    }
    for k in 0..n - 1 {
        if secants[k] == 0.0 {
            slopes[k] = 0.0;
            slopes[k + 1] = 0.0;
            continue;
        }
        let a = slopes[k] / secants[k];
        let b = slopes[k + 1] / secants[k];
        let length = (a * a + b * b).sqrt();
        if length > 3.0 {
            slopes[k] = 3.0 / length * a * secants[k];
            slopes[k + 1] = 3.0 / length * b * secants[k];
        }
    }
    slopes
}

/// A curve through `points` (in order, as [`curve_points`] gives them) at
/// `level`, with the slopes [`monotone_slopes`] gives: what the shader's
/// `curve_at` works out on the GPU, for a window that draws the curve.
pub fn curve_value(points: &[(f64, f64)], slopes: &[f64], level: f64) -> f64 {
    let inside = level.clamp(0.0, 1.0);
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return level;
    };
    let y = if inside <= first.0 {
        first.1
    } else if inside >= last.0 {
        last.1
    } else {
        let i = points
            .windows(2)
            .position(|pair| inside < pair[1].0)
            .unwrap_or(points.len() - 2);
        let ((ax, ay), (bx, by)) = (points[i], points[i + 1]);
        let h = (bx - ax).max(1e-6);
        let s = (inside - ax) / h;
        let (s2, s3) = (s * s, s * s * s);
        (2.0 * s3 - 3.0 * s2 + 1.0) * ay
            + (s3 - 2.0 * s2 + s) * h * slopes[i]
            + (3.0 * s2 - 2.0 * s3) * by
            + (s3 - s2) * h * slopes[i + 1]
    };
    y + (level - inside)
}

/// A transition package's shader, stitched, checked and laid out.
#[derive(Clone, Debug)]
pub struct TransitionShader {
    key: String,
    source: Arc<str>,
    slots: Vec<Slot>,
    span: usize,
}

impl TransitionShader {
    /// Stitches `body` into the transition contract - two bound pictures and a
    /// progress, in the edition the manifest's format asks for - checks it,
    /// and reads the `Params` struct for its parameters' layout. Every
    /// declared parameter must be a field of the struct.
    pub fn compile(manifest: &Manifest, body: &str) -> Result<TransitionShader, String> {
        let (source, slots, span) = stitch(manifest, body, Entry::Transition)?;
        Ok(TransitionShader {
            key: pipeline_key(&manifest.effect.id, manifest.effect.version, &source),
            source,
            slots,
            span,
        })
    }

    /// The stitched module.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The `Params` buffer for these resolved values, laid out to the struct.
    pub fn params_bytes(&self, values: &BTreeMap<String, f64>, params: &[Param]) -> Vec<u8> {
        lay_params(&self.slots, self.span, values, params)
    }

    /// A combine of two layers at `progress` with these values.
    pub fn pass(
        &self,
        values: &BTreeMap<String, f64>,
        params: &[Param],
        progress: f32,
        lut: Option<Arc<Lut>>,
    ) -> TransitionPass {
        TransitionPass {
            key: self.key.clone(),
            source: Arc::clone(&self.source),
            params: self.params_bytes(values, params),
            progress,
            lut,
            xfade: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(params: &str) -> Manifest {
        Manifest::parse(&format!(
            r#"
[effect]
id = "test.thing"
name = "Thing"
kind = "effect"
{params}
[wgsl]
entry = "effect.wgsl"
"#
        ))
        .expect("a valid manifest")
    }

    #[test]
    fn a_body_is_stitched_checked_and_laid_out() {
        let manifest = manifest(
            r#"
[[param]]
key = "amount"
label = "Amount"
min = 0
max = 1
default = 0.25

[[param]]
key = "radius"
label = "Radius"
min = 0
max = 10
default = 2
"#,
        );
        let shader = Shader::compile(
            &manifest,
            r#"
struct Params { radius: f32, amount: f32 }
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let c = sample(uv);
    return vec4<f32>(c.rgb * params.amount + params.radius * 0.0, c.a);
}
"#,
        )
        .expect("compiles");
        assert!(shader.key.starts_with("test.thing@1#"), "{}", shader.key);
        assert!(shader.source().contains("fn fs_main"));
        // radius first at 0, amount at 4; the buffer padded to sixteen.
        let mut values = BTreeMap::new();
        values.insert("amount".to_owned(), 0.5);
        let bytes = shader.params_bytes(&values, &manifest.params);
        assert_eq!(bytes.len(), 16);
        assert_eq!(f32::from_le_bytes(bytes[0..4].try_into().unwrap()), 2.0);
        assert_eq!(f32::from_le_bytes(bytes[4..8].try_into().unwrap()), 0.5);
    }

    fn linear_manifest(kind: &str, table: &str) -> Manifest {
        Manifest::parse(&format!(
            "format = 2\n[effect]\nid = \"test.light\"\nname = \"Light\"\nkind = \"{kind}\"\n[{table}]\nentry = \"effect.wgsl\"\n"
        ))
        .expect("a valid manifest")
    }

    /// A format 2 shader samples the working space with nothing between it
    /// and the texture, is mixed back in light, and is given the
    /// scene-linear library in place of the display-referred one, whose
    /// helpers clip; a format 1 shader keeps the wrapper and the library it
    /// was written against.
    #[test]
    fn the_format_picks_the_edition_of_the_contract() {
        let linear = Shader::compile(
            &linear_manifest("effect", "wgsl"),
            "fn effect(uv: vec2<f32>) -> vec4<f32> {
                let c = sample(uv);
                let graded = saturation(contrast(exposure(c.rgb, 1.0), 1.2), 0.9);
                return vec4<f32>(from_log(to_log(graded)) * luma(vec3<f32>(MID_GREY)), c.a);
            }",
        )
        .expect("the scene-linear library is there");
        assert_eq!(
            Contract::of(&linear_manifest("effect", "wgsl")),
            Contract::Linear
        );
        assert!(!linear.source().contains("legacy_"), "no wrapper");
        assert!(
            linear
                .source()
                .contains("return held(mix(base, treated, clamp(frame.intensity, 0.0, 1.0)));"),
            "mixed in light, and held to what half floats store"
        );
        let clipping = Shader::compile(
            &linear_manifest("effect", "wgsl"),
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(s_curve(sample(uv).rgb, 1.0), 1.0); }",
        );
        assert!(clipping.is_err(), "a display-referred helper is not given");

        let legacy = Shader::compile(
            &manifest(""),
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(s_curve(sample(uv).rgb, 1.0), 1.0); }",
        )
        .expect("the display-referred library is there");
        assert_eq!(Contract::of(&manifest("")), Contract::Legacy);
        assert!(
            legacy
                .source()
                .contains("return legacy_in(textureSample(source, source_sampler, uv));")
        );
        assert!(legacy.source().contains("return legacy_out(mix("));

        let log = Manifest::parse(
            "format = 2\n[effect]\nid = \"test.log\"\nname = \"Log\"\nkind = \"filter\"\n\
             [wgsl]\nentry = \"effect.wgsl\"\nspace = \"log\"\n",
        )
        .expect("a valid manifest");
        assert_eq!(Contract::of(&log), Contract::Log);
        let table = Shader::compile(
            &log,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(look(c.rgb), c.a); }",
        )
        .expect("a table read in log");
        assert!(
            table
                .source()
                .contains("return vec4<f32>(to_log(c.rgb), c.a);")
                && table.source().contains("from_log(treated.rgb)"),
            "sampled in log and taken back into light"
        );

        let transition = TransitionShader::compile(
            &linear_manifest("transition", "transition"),
            "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
                return mix(from_at(uv), to_at(uv), progress) * exp2(0.0) + vec4<f32>(exposure(sample(uv).rgb, 0.0) * 0.0, 0.0);
            }",
        )
        .expect("a scene-linear transition");
        assert!(!transition.source().contains("legacy_"), "no wrapper");
    }

    #[test]
    fn a_package_with_no_knobs_still_binds() {
        let manifest = manifest("");
        let shader = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        )
        .expect("compiles");
        assert_eq!(shader.params_bytes(&BTreeMap::new(), &[]).len(), 16);
    }

    /// A community shader may not bind what the host did not declare,
    /// and may not loop without an end: both are refused at load, before
    /// the package can reach a device. A loop with a way out is fine.
    #[test]
    fn a_hostile_shader_is_refused_at_load() {
        let manifest = manifest("");
        let unbounded = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { var c = sample(uv); loop { c.r = c.r * 0.5; } return c; }",
        );
        assert!(
            unbounded.unwrap_err().contains("no break"),
            "an endless loop"
        );
        let nested = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { var c = sample(uv); for (var i = 0; i < 4; i++) { loop { c.r = c.r * 0.5; } } return c; }",
        );
        assert!(
            nested.unwrap_err().contains("no break"),
            "an endless loop inside a bounded one"
        );
        let extra = Shader::compile(
            &manifest,
            "@group(4) @binding(0) var other: texture_2d<f32>;\nfn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv) + textureSample(other, source_sampler, uv); }",
        );
        assert!(
            extra.unwrap_err().contains("@group(4)"),
            "a binding the host does not provide"
        );
        let bounded = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { var c = sample(uv); var i = 0; loop { i++; if (i > 3) { break; } c.r = c.r * 0.5; } for (var j = 0; j < 2; j++) { c.g = c.g * 0.5; } return c; }",
        );
        assert!(bounded.is_ok(), "{:?}", bounded.err());
    }

    #[test]
    fn a_missing_field_or_a_broken_body_is_refused() {
        let manifest = manifest(
            r#"
[[param]]
key = "amount"
label = "Amount"
"#,
        );
        let missing = Shader::compile(
            &manifest,
            "struct Params { other: f32 }\nfn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        );
        assert!(missing.unwrap_err().contains("no field `amount`"));
        let broken = Shader::compile(
            &manifest,
            "struct Params { amount: f32 }\nfn effect(uv: vec2<f32>) -> vec4<f32> { return nonsense; }",
        );
        assert!(broken.is_err());
        let no_effect = Shader::compile(&manifest, "struct Params { amount: f32 }");
        assert!(no_effect.unwrap_err().contains("fn effect"));
    }

    fn transition_manifest(params: &str) -> Manifest {
        Manifest::parse(&format!(
            r#"
[effect]
id = "test.wipe"
name = "Wipe"
kind = "transition"
{params}
[transition]
entry = "effect.wgsl"
"#
        ))
        .expect("a valid manifest")
    }

    #[test]
    fn a_transition_body_is_stitched_over_two_inputs() {
        let manifest = transition_manifest(
            r#"
[[param]]
key = "softness"
label = "Softness"
min = 0
max = 1
default = 0.5
"#,
        );
        let shader = TransitionShader::compile(
            &manifest,
            r#"
struct Params { softness: f32 }
fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> {
    // Reads both inputs, the shared grading library, and a knob.
    let a = soften(uv, params.softness * 4.0);
    return mix(from_at(uv), to_at(uv), clamp(progress, 0.0, 1.0)) + vec4<f32>(a * 0.0, 0.0);
}
"#,
        )
        .expect("compiles");
        assert!(shader.key.starts_with("test.wipe@1#"), "{}", shader.key);
        assert!(shader.source().contains("fn fs_main"));
        assert!(shader.source().contains("frame.progress"));
        assert!(shader.source().contains("to_texture"));
        let pass = shader.pass(
            &BTreeMap::from([("softness".to_owned(), 0.5)]),
            &manifest.params,
            0.25,
            None,
        );
        assert_eq!(pass.progress, 0.25);
        assert_eq!(pass.params.len(), 16);
    }

    #[test]
    fn a_transition_without_its_entry_is_refused() {
        let manifest = transition_manifest("");
        let no_entry = TransitionShader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return from_at(uv); }",
        );
        assert!(no_entry.unwrap_err().contains("fn transition"));
    }

    /// A shader may only declare the slots the host fills, as what the
    /// host puts there: an effect has no group 0 binding 2, and a sampler
    /// at group 3 binding 0 is not the reveal map that lives there.
    #[test]
    fn a_binding_of_the_wrong_kind_or_the_wrong_entry_is_refused() {
        let manifest = Manifest::parse(
            "[effect]\nid = \"test.bind\"\nname = \"Bind\"\nkind = \"effect\"\n[wgsl]\nentry = \"effect.wgsl\"\n",
        )
        .expect("a manifest");
        let wrong_kind = Shader::compile(
            &manifest,
            "@group(3) @binding(0) var extra: sampler;\nfn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        );
        let message = wrong_kind.expect_err("a sampler where a picture goes");
        assert!(message.contains("@group(3) @binding(0)"), "{message}");
        assert!(message.contains("2D texture"), "{message}");
        let wrong_entry = Shader::compile(
            &manifest,
            "@group(0) @binding(2) var other: texture_2d<f32>;\nfn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        );
        let message = wrong_entry.expect_err("a transition's slot in an effect");
        assert!(message.contains("does not provide"), "{message}");
    }

    /// A format 2 manifest of a package with a `radius` knob from 0 to 300,
    /// drawing `passes` before `effect`.
    fn in_passes(passes: &str) -> Manifest {
        Manifest::parse(&format!(
            "format = 2\n[effect]\nid = \"test.passes\"\nname = \"Passes\"\nkind = \"effect\"\n\
             [[param]]\nkey = \"radius\"\nlabel = \"Radius\"\nmax = 300\ndefault = 10\n\
             [wgsl]\nentry = \"effect.wgsl\"\n{passes}"
        ))
        .expect("a valid manifest")
    }

    const TWO_PASSES: &str = "[[wgsl.pass]]\ntarget = \"across\"\nshrink = [\"radius / 3\", \"1\"]\n\
        [[wgsl.pass]]\ntarget = \"down\"\nshrink = [\"radius / 3\", \"radius / 3\"]\n";

    const THROUGH_BOTH: &str = "struct Params { radius: f32 }\n\
        fn across(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
        fn down(uv: vec2<f32>) -> vec4<f32> { return across_at(uv) * across_texel().x; }\n\
        fn effect(uv: vec2<f32>) -> vec4<f32> { return down_at(uv); }";

    /// Each pass's picture is bound after the layer and read through its
    /// own functions, each pass has an entry point of its own, and the
    /// pass carries the stages with their pictures' shrinks worked out
    /// from the knobs: rounded down to a power of two, at most 64.
    #[test]
    fn a_package_drawn_in_passes_binds_and_sizes_each_picture() {
        let manifest = in_passes(TWO_PASSES);
        let shader = Shader::compile(&manifest, THROUGH_BOTH).expect("compiles");
        for line in [
            "@group(0) @binding(2) var across_picture: texture_2d<f32>;",
            "@group(0) @binding(3) var down_picture: texture_2d<f32>;",
            "fn across_at(uv: vec2<f32>) -> vec4<f32>",
            "fn down_texel() -> vec2<f32>",
            "fn fs_across(in: VsOut)",
            "fn fs_down(in: VsOut)",
            "fn fs_main(in: VsOut)",
        ] {
            assert!(shader.source().contains(line), "no `{line}`");
        }
        let stages = |radius: f64| {
            let values = BTreeMap::from([("radius".to_owned(), radius)]);
            shader
                .pass(&values, &manifest.params, 1.0, None, None)
                .stages
                .iter()
                .map(|stage| (stage.entry.clone(), stage.shrink))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            stages(10.0),
            [
                ("fs_across".to_owned(), [2, 1]),
                ("fs_down".to_owned(), [2, 2])
            ]
        );
        for (radius, shrink) in [(0.0, 1), (5.9, 1), (6.0, 2), (48.0, 16), (300.0, 64)] {
            assert_eq!(stages(radius)[1].1, [shrink, shrink], "radius {radius}");
        }
        assert_eq!(stages(f64::NAN)[1].1, [1, 1], "not a number");

        let single = Shader::compile(
            &in_passes(""),
            "struct Params { radius: f32 }\nfn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        )
        .expect("compiles");
        assert!(
            single
                .pass(&BTreeMap::new(), &[], 1.0, None, None)
                .stages
                .is_empty(),
            "a package drawn in one pass has no stages"
        );
    }

    /// A pass reads only the pictures drawn before it - through a helper as
    /// much as directly - and every picture is read by a pass after it.
    #[test]
    fn a_pass_reads_only_what_was_drawn_before_it() {
        let manifest = in_passes(TWO_PASSES);
        let refused = |body: &str| {
            Shader::compile(
                &manifest,
                &format!("struct Params {{ radius: f32 }}\n{body}"),
            )
            .expect_err("refused")
        };
        let own = refused(
            "fn across(uv: vec2<f32>) -> vec4<f32> { return across_at(uv); }\n\
             fn down(uv: vec2<f32>) -> vec4<f32> { return across_at(uv); }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { return down_at(uv); }",
        );
        assert!(own.contains("`across` reads its own picture"), "{own}");
        let later = refused(
            "fn peek(uv: vec2<f32>) -> vec2<f32> { return down_texel(); }\n\
             fn across(uv: vec2<f32>) -> vec4<f32> { return sample(uv + peek(uv)); }\n\
             fn down(uv: vec2<f32>) -> vec4<f32> { return across_at(uv); }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { return down_at(uv); }",
        );
        assert!(
            later.contains("`across` reads `down`, which is drawn after it"),
            "{later}"
        );
        let unread = refused(
            "fn across(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
             fn down(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { return down_at(uv); }",
        );
        assert!(
            unread.contains("`across` draws a picture no pass after it reads"),
            "{unread}"
        );
        let missing = refused(
            "fn across(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { return across_at(uv); }",
        );
        assert!(missing.contains("to draw the pass `down`"), "{missing}");
        let bound = Shader::compile(
            &in_passes("[[wgsl.pass]]\ntarget = \"across\"\n"),
            "struct Params { radius: f32 }\n\
             @group(0) @binding(3) var more: texture_2d<f32>;\n\
             fn across(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { return across_at(uv) + textureSampleLevel(more, source_sampler, uv, 0.0); }",
        )
        .expect_err("a binding past the pictures");
        assert!(bound.contains("@group(0) @binding(3)"), "{bound}");
    }

    /// A shrink is an expression over the package's knobs that comes to a
    /// number at every knob's default and bounds, or the package does not
    /// load.
    #[test]
    fn a_shrink_is_a_number_worked_out_from_the_knobs() {
        let with = |shrink: &str| {
            Shader::compile(
                &in_passes(&format!(
                    "[[wgsl.pass]]\ntarget = \"across\"\nshrink = [\"{shrink}\", \"1\"]\n"
                )),
                "struct Params { radius: f32 }\n\
                 fn across(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
                 fn effect(uv: vec2<f32>) -> vec4<f32> { return across_at(uv); }",
            )
        };
        with("max(radius / 4, 1)").expect("a knob and numbers");
        let unknown = with("WIDTH / 4").expect_err("the layer's size is not a knob");
        assert!(
            unknown.contains("reads `WIDTH`, which is not a knob"),
            "{unknown}"
        );
        let text = with("fixed(radius, 1)").expect_err("text");
        assert!(text.contains("text where a number was needed"), "{text}");
        let broken = with("radius /").expect_err("half an expression");
        assert!(broken.contains("shrink `radius /`"), "{broken}");
    }

    /// A wheel lays its puck and master into a vec3, and a curve its points
    /// in order into eight vec4s with the slopes that keep it from
    /// overshooting and the count of them; with no points a curve is the
    /// line from black to white.
    #[test]
    fn wheels_and_curves_are_laid_into_the_uniforms() {
        let manifest = Manifest::parse(
            "format = 2\n[effect]\nid = \"test.grade\"\nname = \"Grade\"\nkind = \"effect\"\n\
             [[param]]\nkey = \"lift\"\nlabel = \"Lift\"\ntype = \"wheel\"\nmin = -1\nmax = 1\ndefault = 0.25\n\
             [[param]]\nkey = \"luma\"\nlabel = \"Luma\"\ntype = \"curve\"\n\
             [wgsl]\nentry = \"effect.wgsl\"\nspace = \"display\"\n",
        )
        .expect("a manifest");
        let shader = Shader::compile(
            &manifest,
            "struct Params { lift: vec3<f32>, luma: array<vec4<f32>, 8> }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); let l = vec3<f32>(curve_at(params.luma, c.r)); return vec4<f32>(grade_wheels(l, params.lift, vec3<f32>(0.0), vec3<f32>(0.0)), c.a); }",
        )
        .expect("compiles");
        let floats = |bytes: Vec<u8>| -> Vec<f32> {
            bytes
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        };
        let at_rest = floats(shader.params_bytes(&BTreeMap::new(), &manifest.params));
        assert_eq!(
            &at_rest[..3],
            &[0.0, 0.0, 0.25],
            "the puck in the middle, the master at its default"
        );
        assert_eq!(
            &at_rest[4..12],
            &[0.0, 0.0, 1.0, 2.0, 1.0, 1.0, 1.0, 2.0],
            "the straight line"
        );
        let set: BTreeMap<String, f64> = [
            ("lift.x", 0.5),
            ("lift.y", -0.5),
            ("lift.m", -1.0),
            ("luma.1.x", 1.0),
            ("luma.1.y", 1.0),
            ("luma.0.x", 0.0),
            ("luma.0.y", 0.0),
            ("luma.2.x", 0.5),
            ("luma.2.y", 0.8),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        let laid = floats(shader.params_bytes(&set, &manifest.params));
        assert_eq!(&laid[..3], &[0.5, -0.5, -1.0]);
        let points: Vec<[f32; 4]> = laid[4..16]
            .chunks_exact(4)
            .map(|p| [p[0], p[1], p[2], p[3]])
            .collect();
        assert_eq!(
            points
                .iter()
                .map(|p| (p[0], p[1], p[3]))
                .collect::<Vec<_>>(),
            [(0.0, 0.0, 3.0), (0.5, 0.8, 3.0), (1.0, 1.0, 3.0)]
        );
        assert!(
            points.iter().all(|p| p[2] >= 0.0),
            "a climbing curve climbs everywhere: {points:?}"
        );
    }

    /// The slopes never let a curve overshoot its points: flat at a peak,
    /// and never steeper than three times a segment's own slope.
    #[test]
    fn a_curve_is_monotone_between_its_points() {
        let peak = monotone_slopes(&[(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)]);
        assert_eq!(peak[1], 0.0, "flat at the top");
        let steep = monotone_slopes(&[(0.0, 0.0), (0.1, 0.9), (0.2, 0.95), (1.0, 1.0)]);
        for (k, pair) in [(0.0, 0.0), (0.1, 0.9), (0.2, 0.95), (1.0, 1.0)]
            .windows(2)
            .enumerate()
        {
            let secant = (pair[1].1 - pair[0].1) / (pair[1].0 - pair[0].0);
            assert!(
                steep[k] <= 3.0 * secant + 1e-9 && steep[k + 1] <= 3.0 * secant + 1e-9,
                "{steep:?}"
            );
        }
        let values = |pairs: &[(&str, f64)]| -> BTreeMap<String, f64> {
            pairs
                .iter()
                .map(|(key, value)| ((*key).to_owned(), *value))
                .collect()
        };
        let bend = [(0.0, 0.0), (0.5, 0.6), (1.0, 1.0)];
        let slopes = monotone_slopes(&bend);
        assert!((curve_value(&bend, &slopes, 0.25) - 0.3125).abs() < 1e-9);
        assert_eq!(curve_value(&bend, &slopes, 1.5), 1.5, "past white carried");
        assert_eq!(
            curve_value(&bend, &slopes, -0.2),
            -0.2,
            "below black carried"
        );
        assert_eq!(
            curve_points(&values(&[("c.0.x", 0.3), ("c.0.y", 0.7)]), "c"),
            [(0.0, 0.0), (1.0, 1.0)],
            "one point is no curve"
        );
        assert_eq!(
            curve_points(
                &values(&[
                    ("c.0.x", 1.5),
                    ("c.0.y", -1.0),
                    ("c.3.x", 0.2),
                    ("c.3.y", 0.4),
                    ("c.5.x", 0.2),
                    ("c.5.y", 0.6)
                ]),
                "c"
            ),
            [(0.2, 0.6), (1.0, 0.0)],
            "held to the square, in order, one point to a place"
        );
    }

    /// The pipeline key follows the source, so a shader edited without a
    /// version bump does not keep a stale pipeline.
    #[test]
    fn the_key_changes_with_the_source() {
        let manifest = Manifest::parse(
            "[effect]\nid = \"test.key\"\nname = \"Key\"\nkind = \"effect\"\n[wgsl]\nentry = \"effect.wgsl\"\n",
        )
        .expect("a manifest");
        let one = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }",
        )
        .expect("compiles");
        let two = Shader::compile(
            &manifest,
            "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv) * 0.5; }",
        )
        .expect("compiles");
        assert_ne!(one.key, two.key);
        assert!(one.key.starts_with("test.key@1#"));
    }
}
