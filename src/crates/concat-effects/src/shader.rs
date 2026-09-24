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
//! The stitched module is parsed and validated when the package loads, the
//! same way a chain template is, so a broken shader is a load error and not
//! a black frame. Its `Params` struct is read back through naga for the
//! offset of every field, which is how a clip's settings become the bytes
//! of a uniform buffer without a package having to say anything about
//! layout.

use std::collections::BTreeMap;
use std::sync::Arc;

use concat_core::{Lut, RevealMap, ShaderPass, TransitionPass};

use crate::manifest::{Manifest, Param, ParamType};

/// What every package's shader can see. Group 0 is the layer, group 1 the
/// host's frame block and the package's own parameters.
pub const PRELUDE: &str = r#"// ── the host's half of the contract; see concat-effects/src/shader.rs ──
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

/// The layer's colour at `uv`, straight alpha.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(source, source_sampler, uv);
}

/// One pixel, as a fraction of the layer.
fn texel() -> vec2<f32> {
    return vec2<f32>(1.0, 1.0) / frame.size;
}

/// Luminance, Rec. 709.
fn luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// The package's look-up table applied to a colour - the identity when
/// the package ships none, so the call is always safe. Sampled at the
/// texel centres, so the table's ends land on black and white exactly.
fn lut(rgb: vec3<f32>) -> vec3<f32> {
    let n = f32(textureDimensions(lut_texture).x);
    let uvw = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * (n - 1.0) / n + vec3<f32>(0.5 / n);
    return textureSampleLevel(lut_texture, lut_sampler, uvw, 0.0).rgb;
}

/// A hash in 0..1 from a point and a seed, for grain and dither.
fn hash(p: vec2<f32>, seed: f32) -> f32 {
    let q = vec3<f32>(p, seed);
    return fract(sin(dot(q, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

/// A title's per-word reveal order at `uv`, 0..1 - the word painted there,
/// 0 for the first and 1 for the last, and 0 wherever no word was. A
/// pass over anything but a title reads the identity map here, which is
/// 0 everywhere: `reveal_order(uv) <= progress` is then always true, so a
/// package built on it is a no-op off a title with no special casing.
fn reveal_order(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(reveal_texture, reveal_sampler, uv, 0.0).r;
}

// ── the grading library ──
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

/// The stages the host draws with: a full-screen triangle, and a fragment
/// that mixes the package's colour over the untouched layer by intensity.
pub const POSTLUDE: &str = r#"
struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VsOut {
    // One triangle over the whole target; the clip trims it to the square.
    let x = f32(i32(index & 1u) * 4 - 1);
    let y = f32(i32(index >> 1u) * 4 - 1);
    var out: VsOut;
    out.position = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, 1.0 - (y + 1.0) * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let base = sample(in.uv);
    let treated = effect(in.uv);
    return mix(base, treated, clamp(frame.intensity, 0.0, 1.0));
}
"#;

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
}

impl Shader {
    /// Stitches `body` into the host's contract, checks it, and reads the
    /// `Params` struct for where each of the manifest's parameters lands.
    /// Every declared parameter must be a field of the struct; a field the
    /// manifest does not declare is allowed and stays zero.
    pub fn compile(manifest: &Manifest, body: &str) -> Result<Shader, String> {
        let (source, slots, span) = stitch(manifest, body, Entry::Effect, PRELUDE, POSTLUDE)?;
        Ok(Shader {
            package: manifest.effect.id.clone(),
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

    /// The `Params` buffer for these resolved values, laid out to the
    /// struct. Every declared parameter is present in `values` by the time
    /// the catalogue calls this; anything missing reads as zero.
    pub fn params_bytes(&self, values: &BTreeMap<String, f64>, params: &[Param]) -> Vec<u8> {
        lay_params(&self.slots, self.span, values, params)
    }

    /// A pass over a layer with these values: the uniform buffer written
    /// from them, and the values themselves for a renderer that reads by
    /// name.
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

/// What the host provides at `(group, binding)` for `entry`, and so the
/// only thing a package may declare there: an effect's layer at (0,0)-(0,1)
/// and its reveal map at group 3; a transition's two pictures at (0,0)-(0,3)
/// and nothing at group 3; the frame block and parameters at group 1 and
/// the look-up table at group 2 for both. A slot outside this, or one
/// declared as something else, is a pipeline the device cannot build.
fn provided(entry: Entry, group: u32, binding: u32) -> Option<Bound> {
    match (entry, group, binding) {
        (_, 0, 0) => Some(Bound::Picture),
        (_, 0, 1) => Some(Bound::Sampler),
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
fn budget(module: &naga::Module, entry: Entry) -> Result<(), String> {
    for (_, global) in module.global_variables.iter() {
        let Some(binding) = &global.binding else {
            continue;
        };
        let Some(wanted) = provided(entry, binding.group, binding.binding) else {
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

/// Stitches a package `body` into the host's contract, parses and validates
/// the result with naga, and reads its `Params` struct for where each declared
/// parameter lands. Shared by [`Shader`] and [`TransitionShader`], which differ
/// only in their prelude/postlude and entry function. Returns the finished
/// module, the parameter slots in declaration order, and the padded uniform
/// span.
fn stitch(
    manifest: &Manifest,
    body: &str,
    entry: Entry,
    prelude: &str,
    postlude: &str,
) -> Result<(Arc<str>, Vec<Slot>, usize), String> {
    let declares_params = body
        .split("struct")
        .skip(1)
        .any(|rest| rest.trim_start().starts_with("Params"));
    if !body.contains(&format!("fn {}", entry.name())) {
        return Err(format!("the shader declares no `{}`", entry.signature()));
    }
    let mut source = String::with_capacity(prelude.len() + body.len() + postlude.len() + 64);
    if !declares_params {
        // A package with no knobs still has to bind something.
        source.push_str("struct Params { _unused: f32 }\n");
    }
    source.push_str(body);
    source.push('\n');
    source.push_str(prelude);
    source.push_str(postlude);

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
    budget(&module, entry)?;

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
        let wanted = match param.kind {
            ParamType::Point => 2,
            ParamType::Color => 4,
            _ => 1,
        };
        let width = match inner {
            naga::TypeInner::Scalar(scalar) if is_f32(scalar) => 1,
            naga::TypeInner::Vector { size, scalar } if is_f32(scalar) => *size as usize,
            _ => 0,
        };
        if width != wanted {
            return Err(format!(
                "`Params.{}` must be {}",
                param.key,
                match wanted {
                    2 => "a vec2<f32>",
                    4 => "a vec4<f32>",
                    _ => "an f32",
                }
            ));
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
    for slot in slots {
        match slot.kind {
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

/// The marker that opens the grading library inside [`PRELUDE`]. A transition
/// reuses that same library by slicing it out here rather than copying it.
const GRADING_MARKER: &str = "// ── the grading library ──";

/// The shared grading library: the second half of [`PRELUDE`], from the
/// grading marker to the end. It only reads `sample`, `texel`, `luma`, `hash`,
/// `frame.size` and `frame.time`, all of which the transition head supplies,
/// so it works unchanged over two inputs.
fn grading() -> &'static str {
    let at = PRELUDE
        .find(GRADING_MARKER)
        .expect("the prelude carries a grading library");
    &PRELUDE[at..]
}

/// The transition head: the host's half of a two-input transition. It binds
/// the outgoing picture and the incoming one at group 0, repurposes the
/// frame's spare slot as `progress`, and re-declares the same basics the
/// effect head does so the shared grading library (appended after this) works.
/// `sample` reads the outgoing picture, so a helper like `soften` needs no
/// wiring.
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

/// The outgoing picture's colour at `uv`, straight alpha.
fn from_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(from_texture, from_sampler, uv);
}

/// The incoming picture's colour at `uv`, straight alpha.
fn to_at(uv: vec2<f32>) -> vec4<f32> {
    return textureSample(to_texture, to_sampler, uv);
}

/// What the shared grading helpers read: the outgoing picture, so `soften`
/// and its like work on the from-side with no wiring by the author.
fn sample(uv: vec2<f32>) -> vec4<f32> {
    return from_at(uv);
}

/// One pixel, as a fraction of the layer.
fn texel() -> vec2<f32> {
    return vec2<f32>(1.0, 1.0) / frame.size;
}

/// Luminance, Rec. 709.
fn luma(rgb: vec3<f32>) -> f32 {
    return dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
}

/// The package's look-up table applied to a colour - the identity when the
/// package ships none. Sampled at the texel centres.
fn lut(rgb: vec3<f32>) -> vec3<f32> {
    let n = f32(textureDimensions(lut_texture).x);
    let uvw = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)) * (n - 1.0) / n + vec3<f32>(0.5 / n);
    return textureSampleLevel(lut_texture, lut_sampler, uvw, 0.0).rgb;
}

/// A hash in 0..1 from a point and a seed, for grain and dither.
fn hash(p: vec2<f32>, seed: f32) -> f32 {
    let q = vec3<f32>(p, seed);
    return fract(sin(dot(q, vec3<f32>(12.9898, 78.233, 37.719))) * 43758.5453);
}

"#;

/// The transition's draw stages: a full-screen triangle, and a fragment that
/// hands the whole result to the package's `transition` - it owns the blend,
/// so the pipeline does no mixing of its own.
const TRANSITION_POSTLUDE: &str = r#"
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

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return transition(in.uv, frame.progress);
}
"#;

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
    /// progress - checks it, and reads the `Params` struct for its parameters'
    /// layout. Every declared parameter must be a field of the struct.
    pub fn compile(manifest: &Manifest, body: &str) -> Result<TransitionShader, String> {
        let prelude = format!("{TRANSITION_HEAD}{}", grading());
        let (source, slots, span) = stitch(
            manifest,
            body,
            Entry::Transition,
            &prelude,
            TRANSITION_POSTLUDE,
        )?;
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
