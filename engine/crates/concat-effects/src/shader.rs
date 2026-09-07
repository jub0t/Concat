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

use concat_core::{Lut, ShaderPass};

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
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;
@group(1) @binding(0) var<uniform> frame: Frame;
@group(1) @binding(1) var<uniform> params: Params;
@group(2) @binding(0) var lut_texture: texture_3d<f32>;
@group(2) @binding(1) var lut_sampler: sampler;

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

// ── the grading library ──
//
// Every look is a few of these in different amounts, so they live here,
// once, rather than in each package. Each has an FFmpeg twin a manifest's
// chain can reach for: `saturation` is eq=saturation, `contrast` is
// eq=contrast, `fade` and `s_curve` are curves, `split_tone` and
// `tint_midtones` are colorbalance, `white_balance` is colortemperature,
// `vignette` is vignette, `mono` is colorchannelmixer.

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
    return rgb * (kelvin(k) / kelvin(6500.0));
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
        let declares_params = body
            .split("struct")
            .skip(1)
            .any(|rest| rest.trim_start().starts_with("Params"));
        if !body.contains("fn effect") {
            return Err(
                "the shader declares no `fn effect(uv: vec2<f32>) -> vec4<f32>`".to_owned(),
            );
        }
        let mut source = String::with_capacity(PRELUDE.len() + body.len() + POSTLUDE.len() + 64);
        if !declares_params {
            // A package with no knobs still has to bind something.
            source.push_str("struct Params { _unused: f32 }\n");
        }
        source.push_str(body);
        source.push('\n');
        source.push_str(PRELUDE);
        source.push_str(POSTLUDE);

        let module =
            naga::front::wgsl::parse_str(&source).map_err(|error| error.emit_to_string(&source))?;
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        );
        validator
            .validate(&module)
            .map_err(|error| error.emit_to_string(&source))?;
        if !module
            .functions
            .iter()
            .any(|(_, function)| function.name.as_deref() == Some("effect"))
        {
            return Err(
                "the shader declares no `fn effect(uv: vec2<f32>) -> vec4<f32>`".to_owned(),
            );
        }

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

        Ok(Shader {
            key: format!("{}@{}", manifest.effect.id, manifest.effect.version),
            source: Arc::from(source),
            slots,
            span: span.max(ShaderPass::MIN_PARAMS).div_ceil(16) * 16,
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
        let mut bytes = vec![0u8; self.span];
        let mut put = |offset: usize, value: f64| {
            let at = offset..offset + 4;
            if at.end <= bytes.len() {
                bytes[at].copy_from_slice(&(value as f32).to_le_bytes());
            }
        };
        for slot in &self.slots {
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

    /// A pass over a layer with these values.
    pub fn pass(
        &self,
        values: &BTreeMap<String, f64>,
        params: &[Param],
        intensity: f32,
        lut: Option<Arc<Lut>>,
    ) -> ShaderPass {
        ShaderPass {
            key: self.key.clone(),
            source: Arc::clone(&self.source),
            params: self.params_bytes(values, params),
            intensity,
            lut,
        }
    }
}

fn is_f32(scalar: &naga::Scalar) -> bool {
    scalar.kind == naga::ScalarKind::Float && scalar.width == 4
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
        assert_eq!(shader.key, "test.thing@1");
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
}
