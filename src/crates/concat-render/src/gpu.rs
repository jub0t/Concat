// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The wgpu compositor.
//!
//! The one implementation of [`Compositor`](crate::Compositor). Layers are
//! uploaded as textures, drawn as transformed quads into an offscreen
//! target, and read back as a [`Frame`] or handed on as a texture.
//!
//! - Everything is drawn in half floats (`WORK`, `Rgba16Float`), in light:
//!   extended linear Rec. 709, 1.0 the white of an SDR picture (203 nits).
//!   A frame is converted into it on the GPU as it is uploaded, and the
//!   finished frame out of it once, at the end (`resolve`): as it is for an
//!   SDR timeline, rolled off by BT.2390 for an HDR one on an SDR screen.
//!   An HDR clip is conformed to SDR as it uploads for an SDR timeline, and
//!   keeps its light above white for an HDR one (`FramePlan::output`).
//! - Layer quads are sampled bilinearly with clamp-to-edge, as the tests'
//!   CPU oracle (`crate::reference`) does.
//!
//! Construction is fallible: [`WgpuCompositor::new`] takes the machine's GPU,
//! or its software adapter where it has none (WARP on Windows, lavapipe on
//! Linux), and a machine with neither gets `None`, which the caller reports.
//! Never panic over a missing GPU.
//!
//! Two outputs. [`Compositor::composite`] reads the frame back for the
//! encoder. [`WgpuCompositor::composite_texture`] leaves it on the GPU as a
//! texture the window can show directly - when the compositor was built on
//! the window's own device with [`WgpuCompositor::with_device`], that is the
//! monitor with no copy anywhere.

use std::collections::HashMap;

use concat_core::frame::Frame;
use concat_core::shader::{Lut, RevealMap, ShaderPass, TransitionPass};
use concat_core::timeline::Blend;

use crate::compositor::Compositor;
use crate::plan::{FramePlan, Geometry, PlannedLayer, PlannedTreatment, Shading};

/// Bytes per row must be a multiple of this for a texture-to-buffer copy.
const ROW_ALIGN: usize = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;

/// One vertex of a layer quad: clip-space position, the texel it samples,
/// and everything that weighs the pixel riding along - the opacity, the
/// fades folded into a scale and an offset of the colour, the wipes as
/// two edges, and where on the picture the pixel is - so no uniforms are
/// needed and one draw call is one layer.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
    scale: f32,
    offset: [f32; 3],
    edges: [f32; 2],
    /// `0..1` across the picture as it is seen: what the mask is sampled
    /// at and what the wipes measure.
    pic: [f32; 2],
}

/// Vertex data as raw bytes. `Vertex` is `repr(C)` and all `f32`, so its byte
/// representation is well-defined; this avoids pulling in bytemuck.
fn as_bytes(vertices: &[Vertex]) -> &[u8] {
    // SAFETY: Vertex is repr(C) with only f32 fields - no padding, no
    // invalid bit patterns, alignment of u8 is 1.
    unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(vertices),
        )
    }
}

const SHADER: &str = r#"
struct VsIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) opacity: f32,
    @location(3) scale: f32,
    @location(4) offset: vec3<f32>,
    @location(5) edges: vec2<f32>,
    @location(6) pic: vec2<f32>,
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
    @location(2) scale: f32,
    @location(3) offset: vec3<f32>,
    @location(4) edges: vec2<f32>,
    @location(5) pic: vec2<f32>,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.opacity = in.opacity;
    out.scale = in.scale;
    out.offset = in.offset;
    out.edges = in.edges;
    out.pic = in.pic;
    return out;
}

@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;
@group(1) @binding(0) var mask_texture: texture_2d<f32>;
@group(1) @binding(1) var mask_sampler: sampler;
// The ground as it stood before this layer, for the two blends that need
// to see it; bound only for their pipelines.
@group(2) @binding(0) var ground_texture: texture_2d<f32>;
@group(2) @binding(1) var ground_sampler: sampler;

// The layer's straight colour and its alpha at this fragment, before any
// blend: the same lines the CPU reference computes.
fn shade(in: VsOut) -> vec4<f32> {
    let colour = textureSample(layer_texture, layer_sampler, in.uv);
    let mask = textureSample(mask_texture, mask_sampler, in.pic);
    // The wipes: a pixel past the moving edge is not drawn at all.
    let kept = select(0.0, 1.0, in.pic.x < in.edges.x && in.pic.x >= in.edges.y);
    let alpha = colour.a * in.opacity * mask.a * kept;
    // The fades: the colour scaled and offset.
    let shaded = colour.rgb * in.scale + in.offset;
    return vec4<f32>(shaded, alpha);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    // Premultiplied output; the pipeline blends ONE / ONE_MINUS_SRC_ALPHA,
    // which together is the same source-over the CPU path computes.
    return vec4<f32>(s.rgb * s.a, s.a);
}

fn ground_at(position: vec4<f32>) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(ground_texture));
    return textureSample(ground_texture, ground_sampler, position.xy / size).rgb;
}

// Lighten and Darken weigh the lighter (darker) of the layer and the
// ground in by the layer's alpha - a white layer at 30 % over mid grey
// lightens it 30 % of the way to white - which no fixed-function blend
// expresses. The ground is a copy taken just before this draw; the
// result is premultiplied and blended source-over, so what lands is
// max(colour, ground) * alpha + ground * (1 - alpha), the CPU's own line.
@fragment
fn fs_lighten(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    return vec4<f32>(max(s.rgb, ground_at(in.position)) * s.a, s.a);
}

@fragment
fn fs_darken(in: VsOut) -> @location(0) vec4<f32> {
    let s = shade(in);
    return vec4<f32>(min(s.rgb, ground_at(in.position)) * s.a, s.a);
}
"#;

/// The format everything is drawn in: half floats a channel, so a stack
/// of layers and effects keeps its precision, and later its light above
/// one, between the upload and the one conversion to eight bits at the
/// end (see `resolve`). Uploaded frames are converted into it on the GPU
/// as they arrive (see `upload`).
const WORK: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What a texture in the pool costs a pixel.
const WORK_BYTES: u64 = 8;

/// A picture copied texel for texel into a target of the same size, through
/// the colour conversion at each end of the working space: an uploaded
/// frame into it (`fs_upload`), and a finished frame out of it into the
/// eight bits a screen and an encoder take (`fs_resolve`). A triangle that
/// covers the target, and a load at each fragment's own texel, so nothing
/// is filtered on the way.
///
/// The working space is linear light on Rec. 709 primaries, extended: a
/// value may be negative (a colour outside Rec. 709, as a Rec. 2020 source
/// has) or above one (a highlight), with 1.0 the white of an SDR picture
/// (203 nits, ITU-R BT.2408) - the layout Windows calls scRGB and Apple's
/// EDR extended linear sRGB. Blending and every fade then behave as light
/// does. Rec. 709 primaries rather than Rec. 2020's because the store is
/// half floats: in a basis the picture is not in, a primary leaks a few
/// ten-thousandths into its neighbours, and the output's 1/2.4 power makes
/// that a visible tint in the shadows. An eight-bit frame is gamma-encoded
/// Rec. 709, taken as BT.1886's 2.4 gamma; the resolve undoes exactly what
/// the upload did, so a picture with nothing on it comes out as it went in.
const COPY_SHADER: &str = r#"
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_upload(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = textureLoad(picture, vec2<i32>(at.xy), 0);
    return vec4<f32>(pow(max(colour.rgb, vec3<f32>(0.0)), vec3<f32>(2.4)), colour.a);
}

@fragment
fn fs_resolve(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = textureLoad(picture, vec2<i32>(at.xy), 0);
    let clipped = clamp(colour.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(pow(clipped, vec3<f32>(1.0 / 2.4)), colour.a);
}

// An HDR file: the light on Rec. 2020's primaries, encoded HLG or PQ, and
// handed back as sixteen-bit integers for the encoder, opaque.
fn to_2020(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vec3<f32>(0.627403896, 0.329283039, 0.043313065), rgb),
        dot(vec3<f32>(0.069097289, 0.919540395, 0.011362316), rgb),
        dot(vec3<f32>(0.016391439, 0.088013308, 0.895595253), rgb),
    );
}

fn deep_out(signal: vec3<f32>) -> vec4<u32> {
    let clipped = clamp(signal, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<u32>(vec3<u32>(round(clipped * 65535.0)), 65535u);
}

// BT.2100's HLG for a 1000-nit display, the inverse of the upload's: the
// display's light back to the scene's through the inverse OOTF (a system
// gamma of 1.2), then ARIB STD-B67's OETF - which puts SDR white, 203 nits,
// at 75 % of the signal.
@fragment
fn fs_resolve_hlg(@builtin(position) at: vec4<f32>) -> @location(0) vec4<u32> {
    let colour = textureLoad(picture, vec2<i32>(at.xy), 0);
    let nits = max(to_2020(colour.rgb), vec3<f32>(0.0)) * 203.0;
    let yd = max(dot(nits, vec3<f32>(0.2627, 0.6780, 0.0593)), 1e-6);
    let scene = clamp(nits / 1000.0 * pow(yd / 1000.0, -0.2 / 1.2), vec3<f32>(0.0), vec3<f32>(1.0));
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    let low = sqrt(3.0 * scene);
    let high = a * log(max(12.0 * scene - b, vec3<f32>(1e-6))) + c;
    return deep_out(select(high, low, scene <= vec3<f32>(1.0 / 12.0)));
}

// SMPTE ST 2084, the light in nits as the signal.
@fragment
fn fs_resolve_pq(@builtin(position) at: vec4<f32>) -> @location(0) vec4<u32> {
    let colour = textureLoad(picture, vec2<i32>(at.xy), 0);
    let nits = max(to_2020(colour.rgb), vec3<f32>(0.0)) * 203.0;
    return deep_out(vec3<f32>(pq_signal(nits.r), pq_signal(nits.g), pq_signal(nits.b)));
}

// An HDR timeline for an SDR screen: its light rolled off by BT.2390 as an
// HDR clip's is on an SDR timeline - the same maths, so a lone clip looks
// the same on either - then encoded as SDR is.
@fragment
fn fs_resolve_tone_mapped(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = textureLoad(picture, vec2<i32>(at.xy), 0);
    let light = to_sdr(colour.rgb * 203.0) / 203.0;
    let clipped = clamp(light, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(pow(clipped, vec3<f32>(1.0 / 2.4)), colour.a);
}
"#;

/// SMPTE ST 2084 and BT.2390's roll-off, which the deep upload and the
/// resolve both reach for: appended to each of their modules.
const TONE_MAP: &str = r#"
// SMPTE ST 2084: a signal to nits, and back.
const PQ_M1: f32 = 0.1593017578125;
const PQ_M2: f32 = 78.84375;
const PQ_C1: f32 = 0.8359375;
const PQ_C2: f32 = 18.8515625;
const PQ_C3: f32 = 18.6875;

fn pq_nits(signal: vec3<f32>) -> vec3<f32> {
    let e = pow(clamp(signal, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / PQ_M2));
    let y = pow(max(e - PQ_C1, vec3<f32>(0.0)) / (PQ_C2 - PQ_C3 * e), vec3<f32>(1.0 / PQ_M1));
    return y * 10000.0;
}

fn pq_signal(nits: f32) -> f32 {
    let y = pow(clamp(nits / 10000.0, 0.0, 1.0), PQ_M1);
    return pow((PQ_C1 + PQ_C2 * y) / (1.0 + PQ_C3 * y), PQ_M2);
}

// BT.2390's EETF from a 1000-nit master to SDR white (203 nits), on the
// brightest channel, the colour scaled with it.
fn to_sdr(nits: vec3<f32>) -> vec3<f32> {
    let peak = max(nits.r, max(nits.g, nits.b));
    if (peak <= 0.0) {
        return nits;
    }
    let master = pq_signal(1000.0);
    let e1 = pq_signal(peak) / master;
    let top = pq_signal(203.0) / master;
    let knee = 1.5 * top - 0.5;
    var e2 = e1;
    if (e1 > knee) {
        let t = (min(e1, 1.0) - knee) / (1.0 - knee);
        let t2 = t * t;
        let t3 = t2 * t;
        e2 = (2.0 * t3 - 3.0 * t2 + 1.0) * knee
            + (t3 - 2.0 * t2 + t) * (1.0 - knee)
            + (-2.0 * t3 + 3.0 * t2) * top;
    }
    let mapped = pq_nits(vec3<f32>(e2 * master)).r;
    return nits * (mapped / peak);
}
"#;

/// A deep frame (sixteen bits a channel, Rec. 2020, its source's own signal)
/// copied into the working space: the upload of an HDR or wide-gamut clip.
/// One entry a signal (`concat_core::frame::Signal`): the transfer undone
/// into light, 1.0 the white of an SDR picture (203 nits, ITU-R BT.2408),
/// and the primaries brought to Rec. 709's, where a colour outside them is
/// negative rather than clipped.
///
/// On an SDR timeline an HDR clip is conformed to SDR as it arrives, as
/// Final Cut does a clip at a time: BT.2390's roll-off (ITU-R BT.2390, the
/// EETF) takes its highlights from a 1000-nit master down to SDR white, on
/// the brightest channel so a hue keeps its place. On an HDR timeline it
/// keeps them (the `_hdr` entries), and the resolve rolls the whole frame
/// off instead where the screen is SDR.
const DEEP_SHADER: &str = r#"
@group(0) @binding(0) var deep: texture_2d<u32>;

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn deep_at(at: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(textureLoad(deep, vec2<i32>(at.xy), 0)) / 65535.0;
}

// Rec. 2020 primaries to Rec. 709's, linear light (ITU-R BT.2087's matrix
// inverted, to full precision).
fn from_2020(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(vec3<f32>(1.660491002, -0.587641139, -0.072849863), rgb),
        dot(vec3<f32>(-0.124550475, 1.132899897, -0.008349423), rgb),
        dot(vec3<f32>(-0.018150763, -0.100578898, 1.118729661), rgb),
    );
}

@fragment
fn fs_upload_sdr_wide(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = deep_at(at);
    let lin = pow(max(colour.rgb, vec3<f32>(0.0)), vec3<f32>(2.4));
    return vec4<f32>(from_2020(lin), colour.a);
}

// ARIB STD-B67's inverse OETF to scene light, then BT.2100's OOTF for a
// 1000-nit display (a system gamma of 1.2), which puts HLG's reference
// white - 75 % of the signal - at 203 nits.
fn hlg_nits(signal: vec3<f32>) -> vec3<f32> {
    let a = 0.17883277;
    let b = 0.28466892;
    let c = 0.55991073;
    let e = clamp(signal, vec3<f32>(0.0), vec3<f32>(1.0));
    let low = e * e / 3.0;
    let high = (exp((e - c) / a) + b) / 12.0;
    let scene = select(high, low, e <= vec3<f32>(0.5));
    let ys = dot(scene, vec3<f32>(0.2627, 0.6780, 0.0593));
    return 1000.0 * pow(max(ys, 1e-6), 0.2) * scene;
}

// On an SDR timeline, conformed to SDR as they arrive.
@fragment
fn fs_upload_pq(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = deep_at(at);
    return vec4<f32>(from_2020(to_sdr(pq_nits(colour.rgb)) / 203.0), colour.a);
}

@fragment
fn fs_upload_hlg(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = deep_at(at);
    return vec4<f32>(from_2020(to_sdr(hlg_nits(colour.rgb)) / 203.0), colour.a);
}

// On an HDR timeline, kept: the highlights stay above white, 1000 nits of
// the master at 4.93.
@fragment
fn fs_upload_pq_hdr(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = deep_at(at);
    return vec4<f32>(from_2020(pq_nits(colour.rgb) / 203.0), colour.a);
}

@fragment
fn fs_upload_hlg_hdr(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
    let colour = deep_at(at);
    return vec4<f32>(from_2020(hlg_nits(colour.rgb) / 203.0), colour.a);
}
"#;

/// One package's shader, compiled once and kept: its pipeline, and the two
/// uniform buffers every pass through it rewrites.
struct CompiledShader {
    pipeline: wgpu::RenderPipeline,
    frame: wgpu::Buffer,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// For a package drawn in several passes, the pipeline of each stage
    /// before the last, in the pass's order; `pipeline` draws the last.
    stages: Vec<wgpu::RenderPipeline>,
}

/// A 2D float texture a pass samples, at `binding` of its group.
fn picture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// A filtering sampler, at `binding` of its group.
fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// One draw of a composite: the pooled texture drawn, how it meets the
/// ground, and the pooled texture masking it - the one white pixel for a
/// layer without a mask.
struct Draw {
    size: (u32, u32),
    texture: usize,
    blend: Blend,
    mask: (u32, u32, usize),
    /// For a Lighten or Darken layer: the pooled texture, at the output
    /// size, that the ground is copied into just before the draw, for its
    /// fragment stage to sample. See `fs_lighten` in [`SHADER`].
    ground: Option<usize>,
}

/// A cached layer texture and its bind group, reusable for any layer of the
/// same size.
struct PooledTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    /// The identity of the frame uploaded into it, or zero for a texture
    /// a pass drew: what lets a frame the device already holds - a still,
    /// a title, a paused clip, a frame scrubbed back over - skip its upload.
    holds: u64,
    /// The composite it was last drawn in, counted by `prepare`: what the
    /// least recently used frame is picked by when the pool is full.
    drawn: u64,
}

/// What the layer pool may hold, in bytes, before it overwrites a frame it
/// still has rather than growing: the source-texture cache's budget. The
/// pool is also the cache - a frame uploaded stays in its texture until
/// the texture is wanted for something else - so a scrub back over ground
/// the monitor has shown finds its frames on the device. 512 MB is 64
/// frames at 1080p, or 256 at the monitor's usual 960 by 540.
const POOL_BUDGET: u64 = 512 * 1024 * 1024;

/// The LUTs and the reveal maps kept on the device, at most: a look is a
/// megabyte at 65 a side, and each one a person tries stayed on the device
/// for the life of the compositor. The least recently used go first, never
/// one the composite in hand uses.
const LUT_CACHE: usize = 32;
const REVEAL_CACHE: usize = 64;

/// And at most this many textures, whatever their size: many small
/// distinct frames - a title's layers, masks - would otherwise pile up
/// under the byte budget for as long as none of them is a megabyte.
const POOL_TEXTURES: usize = 1024;

/// The reusable output target and its readback buffer, for one output size.
struct Target {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    staging: wgpu::Buffer,
    padded_row: usize,
}

/// Presentable output textures, in a ring: the window may still be
/// sampling the last one while the next is drawn.
struct Presentable {
    width: u32,
    height: u32,
    ring: Vec<wgpu::Texture>,
    next: usize,
}

/// How many presentable textures are kept: the one on screen, the one being
/// drawn, and one so a late frame never waits on either.
const PRESENT_RING: usize = 3;

/// A compositor that draws on the GPU. See the module docs.
pub struct WgpuCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// One pipeline per blend mode, indexed as `Blend::ALL` is.
    pipelines: Vec<wgpu::RenderPipeline>,
    /// Lighten, then Darken: the two blends that sample the ground, drawn
    /// with a third bind group and plain source-over.
    ground_pipelines: [wgpu::RenderPipeline; 2],
    /// An uploaded frame into the working format.
    upload_pipeline: wgpu::RenderPipeline,
    /// A finished frame out of the working format into eight bits: as it
    /// is for an SDR timeline, and rolled off for an HDR one on an SDR
    /// screen.
    resolve_pipeline: wgpu::RenderPipeline,
    tone_mapped_pipeline: wgpu::RenderPipeline,
    /// What the plan being drawn is output in (`FramePlan::output`): taken
    /// up by `prepare`, and read by the uploads and the resolve after it.
    output: concat_core::frame::Signal,
    /// An HDR file out of the working space: HLG, then PQ, into sixteen-bit
    /// integers the encoder takes as they are.
    hdr_pipelines: [wgpu::RenderPipeline; 2],
    /// Frames of an HDR plan come back for an HDR file, not rolled off for
    /// an SDR one (`Compositor::deliver_hdr`).
    deliver_hdr: bool,
    /// The readback target an HDR frame is resolved into: sixteen-bit
    /// integers, eight bytes a pixel.
    deep_target: Option<Target>,
    /// The eight-bit textures frames are written into on their way to the
    /// pool, one a size, each with its bind group.
    staging: HashMap<(u32, u32), (wgpu::Texture, wgpu::BindGroup)>,
    /// A deep frame into the working space, by signal: Rec. 2020 gamma,
    /// HLG and PQ conformed to SDR, then HLG and PQ kept HDR (see
    /// `DEEP_SHADER`).
    deep_pipelines: [wgpu::RenderPipeline; 5],
    /// The layout a deep staging texture is bound with: sixteen-bit
    /// integers, read texel by texel.
    deep_layout: wgpu::BindGroupLayout,
    /// The sixteen-bit staging textures, one a size.
    deep_staging: HashMap<(u32, u32), (wgpu::Texture, wgpu::BindGroup)>,
    bind_layout: wgpu::BindGroupLayout,
    /// Group 1 of a shader pass: the frame block and the package's params.
    uniform_layout: wgpu::BindGroupLayout,
    /// Group 2 of a shader pass: the package's look-up table, a 3D texture.
    lut_layout: wgpu::BindGroupLayout,
    /// Group 3 of a shader pass: a title's per-word reveal map, a 2D
    /// texture; see `concat_core::RevealMap`.
    reveal_layout: wgpu::BindGroupLayout,
    /// Group 0 of a transition: the outgoing and incoming pictures, each a
    /// texture and its sampler.
    transition_layout: wgpu::BindGroupLayout,
    /// Group 0 of a package drawn in several passes, by how many pictures
    /// its passes draw: the layer and its sampler, then each picture; see
    /// [`WgpuCompositor::pictures_layout`].
    pictures_layouts: HashMap<usize, wgpu::BindGroupLayout>,
    /// What a pass binds in place of a picture not drawn yet - its own
    /// among them: one transparent pixel.
    blank: wgpu::TextureView,
    /// Uploaded tables by their id, the identity among them; see `lut_group`.
    luts: HashMap<u64, wgpu::BindGroup>,
    /// The composite each cached LUT was last used in; see [`LUT_CACHE`].
    luts_drawn: HashMap<u64, u64>,
    /// Uploaded reveal maps by their id, the identity among them; see
    /// `reveal_group`.
    reveals: HashMap<u64, wgpu::BindGroup>,
    /// The composite each cached reveal map was last used in.
    reveals_drawn: HashMap<u64, u64>,
    /// Compiled passes by their key; see `ShaderPass::key`.
    shaders: HashMap<String, CompiledShader>,
    /// Compiled transitions by their key; see `TransitionPass::key`.
    transitions: HashMap<String, CompiledShader>,
    /// Passes and transitions the driver refused a pipeline for, by key:
    /// tried once, skipped from then on, never asked for again.
    refused: std::collections::HashSet<String>,
    sampler: wgpu::Sampler,
    vertices: wgpu::Buffer,
    vertex_capacity: usize,
    /// Layer textures pooled by size; `used` counts how many of a size this
    /// frame has claimed, and resets every composite. `idle` counts the
    /// composites a size has gone unclaimed: a timeline moves past a clip
    /// size forever, and its textures should not outlive that by much.
    pool: HashMap<(u32, u32), Vec<PooledTexture>>,
    /// What the pool's textures hold, in bytes; see [`POOL_BUDGET`].
    pool_bytes: u64,
    /// How many textures the pool holds; see [`POOL_TEXTURES`].
    pool_textures: usize,
    /// Composites drawn so far; see [`PooledTexture::drawn`].
    composites: u64,
    /// Frames uploaded so far: what a cache hit does not add to.
    uploads: u64,
    used: HashMap<(u32, u32), usize>,
    idle: HashMap<(u32, u32), u32>,
    target: Option<Target>,
    presentable: Option<Presentable>,
    /// The two pictures a packaged transition combines, drawn and kept on
    /// the GPU for [`WgpuCompositor::render_transition_texture`]: their own
    /// targets, since `prepare` hands the layer pool out afresh per plan and
    /// the second picture's plan would draw over the first. By size.
    stages: Option<(u32, u32, [wgpu::Texture; 2])>,
    /// A one-pixel opaque white picture: the mask of a layer without one.
    white: std::sync::Arc<Frame>,
    /// Set when a readback fails - a lost or reset device. The compositor
    /// then answers every composite from the CPU reference instead: slower,
    /// always correct, and never a panic in the middle of an export.
    dead: bool,
}

impl WgpuCompositor {
    /// Builds a compositor on the best available adapter, or `None` when the
    /// machine has nothing usable - callers fall back to the CPU path.
    ///
    /// Native only: it blocks on the adapter and device requests, and on the
    /// web there is no thread to block. A web caller awaits those requests
    /// itself and hands the result to [`WgpuCompositor::with_device`].
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        // The GPU where there is one; the software adapter where there is
        // not, which draws the same pictures, slower - the one road left
        // since the CPU compositor went.
        let adapter = [false, true].into_iter().find_map(|software| {
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                force_fallback_adapter: software,
                ..Default::default()
            }))
            .ok()
        })?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Self::with_device(device, queue))
    }

    /// A second compositor on this one's device, with pipelines, pools and
    /// caches of its own: work on another thread - the effect cards -
    /// shares the GPU's time with this one and nothing else, so it never
    /// waits on this one's lock or evicts what this one keeps.
    pub fn sibling(&self) -> Self {
        Self::with_device(self.device.clone(), self.queue.clone())
    }

    /// Builds a compositor on a device the caller owns - the window's, so a
    /// texture this draws is one the window can show.
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("concat compositor"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat layer"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // What a shader pass binds at group 1: the host's frame block and
        // the package's own `Params`, both uniforms.
        let uniform_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat pass uniforms"),
            entries: &[uniform_entry(0), uniform_entry(1)],
        });
        // Group 2: a look-up table. Every pass binds one - the identity when
        // the package has none - so one pipeline layout serves them all.
        let lut_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat pass lut"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // Group 3: a title's reveal map. Every pass binds one - a map that
        // reveals everything when the package has none, or the pass is not
        // over a title at all - so the same pipeline layout serves every
        // shader pass whether or not it reads `reveal_order()`.
        let reveal_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat pass reveal map"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // Group 0 of a transition: two pictures, each a texture and a
        // sampler - the outgoing at 0/1, the incoming at 2/3.
        let transition_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat transition inputs"),
            entries: &[
                picture_entry(0),
                sampler_entry(1),
                picture_entry(2),
                sampler_entry(3),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("concat compositor"),
            // Group 0 is the layer, group 1 its mask: the same shape, a
            // texture and a sampler.
            bind_group_layouts: &[Some(&bind_layout), Some(&bind_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![
                0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Float32,
                4 => Float32x3, 5 => Float32x2, 6 => Float32x2
            ],
        };

        // ONE / ONE_MINUS_SRC_ALPHA over premultiplied shader output is
        // source-over. The other modes are the fixed-function blends the
        // CPU reference spells the same way (see `Blend`), one pipeline
        // each, since a blend state is baked into a pipeline. Alpha always
        // accumulates as source-over; the readback forces the final frame
        // opaque regardless.
        let alpha = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
            operation: wgpu::BlendOperation::Add,
        };
        let pipelines: Vec<wgpu::RenderPipeline> = Blend::ALL
            .into_iter()
            .map(|mode| {
                use wgpu::{BlendFactor, BlendOperation};
                let color = match mode {
                    Blend::Normal => wgpu::BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::OneMinusSrcAlpha,
                        operation: BlendOperation::Add,
                    },
                    Blend::Multiply => wgpu::BlendComponent {
                        src_factor: BlendFactor::Dst,
                        dst_factor: BlendFactor::OneMinusSrcAlpha,
                        operation: BlendOperation::Add,
                    },
                    Blend::Screen => wgpu::BlendComponent {
                        src_factor: BlendFactor::OneMinusDst,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    Blend::Add => wgpu::BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Add,
                    },
                    Blend::Lighten => wgpu::BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Max,
                    },
                    Blend::Darken => wgpu::BlendComponent {
                        src_factor: BlendFactor::One,
                        dst_factor: BlendFactor::One,
                        operation: BlendOperation::Min,
                    },
                };
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("concat compositor"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[Some(vertex_layout.clone())],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: WORK,
                            blend: Some(wgpu::BlendState { color, alpha }),
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
            .collect();

        // Lighten and Darken: source-over of a fragment that has already
        // taken the max or min against a copy of the ground, bound as a
        // third group of the same shape as the layer and its mask.
        let ground_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("concat compositor ground"),
            bind_group_layouts: &[Some(&bind_layout), Some(&bind_layout), Some(&bind_layout)],
            immediate_size: 0,
        });
        let ground_pipelines = ["fs_lighten", "fs_darken"].map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&ground_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(vertex_layout.clone())],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORK,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha,
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        });

        // The two copies in and out of the working format; see COPY_SHADER.
        let copy_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("concat copy"),
            source: wgpu::ShaderSource::Wgsl(format!("{COPY_SHADER}{TONE_MAP}").into()),
        });
        let copy_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("concat copy"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let copy_into = |format: wgpu::TextureFormat, entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&copy_layout),
                vertex: wgpu::VertexState {
                    module: &copy_shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &copy_shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let upload_pipeline = copy_into(WORK, "fs_upload");
        let resolve_pipeline = copy_into(wgpu::TextureFormat::Rgba8Unorm, "fs_resolve");
        let tone_mapped_pipeline =
            copy_into(wgpu::TextureFormat::Rgba8Unorm, "fs_resolve_tone_mapped");
        let hdr_pipelines = ["fs_resolve_hlg", "fs_resolve_pq"]
            .map(|entry| copy_into(wgpu::TextureFormat::Rgba16Uint, entry));

        let deep_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat deep"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let deep_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("concat deep"),
            source: wgpu::ShaderSource::Wgsl(format!("{DEEP_SHADER}{TONE_MAP}").into()),
        });
        let deep_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("concat deep"),
            bind_group_layouts: &[Some(&deep_layout)],
            immediate_size: 0,
        });
        let deep_pipelines = [
            "fs_upload_sdr_wide",
            "fs_upload_hlg",
            "fs_upload_pq",
            "fs_upload_hlg_hdr",
            "fs_upload_pq_hdr",
        ]
        .map(|entry| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&deep_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &deep_shader,
                    entry_point: Some("vs_full"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &deep_shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORK,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("concat layer"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("concat quads"),
            size: (std::mem::size_of::<Vertex>() * 6 * 8) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Never written: wgpu clears a texture before its first use, so it
        // reads as nothing at all.
        let blank = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("concat blank picture"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WORK,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            device,
            queue,
            pipelines,
            ground_pipelines,
            upload_pipeline,
            resolve_pipeline,
            tone_mapped_pipeline,
            output: concat_core::frame::Signal::Sdr,
            hdr_pipelines,
            deliver_hdr: false,
            deep_target: None,
            staging: HashMap::new(),
            deep_pipelines,
            deep_layout,
            deep_staging: HashMap::new(),
            bind_layout,
            uniform_layout,
            lut_layout,
            reveal_layout,
            transition_layout,
            pictures_layouts: HashMap::new(),
            blank,
            luts: HashMap::new(),
            luts_drawn: HashMap::new(),
            reveals: HashMap::new(),
            reveals_drawn: HashMap::new(),
            shaders: HashMap::new(),
            transitions: HashMap::new(),
            refused: std::collections::HashSet::new(),
            sampler,
            vertices,
            vertex_capacity: 6 * 8,
            pool: HashMap::new(),
            pool_bytes: 0,
            pool_textures: 0,
            composites: 0,
            uploads: 0,
            used: HashMap::new(),
            target: None,
            presentable: None,
            stages: None,
            idle: HashMap::new(),
            // Shared, never cloned: a cloned Frame is a new picture with a
            // new identity, and every composite would upload it again.
            white: {
                let mut white = Frame::transparent(1, 1);
                white.fill([255, 255, 255, 255]);
                std::sync::Arc::new(white)
            },
            dead: false,
        }
    }

    /// Whether the device has been lost. A dead compositor answers
    /// [`Compositor::composite`] from the CPU and refuses textures.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// Waits until the device has done everything asked of it so far: how
    /// a timing knows that a texture [`WgpuCompositor::render_texture`]
    /// handed back is drawn, without reading it back. False when the
    /// device did not answer.
    pub fn finish(&self) -> bool {
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .is_ok()
    }

    /// The next presentable texture for this output size.
    fn presentable(&mut self, width: u32, height: u32) -> wgpu::Texture {
        let stale = self
            .presentable
            .as_ref()
            .is_none_or(|p| p.width != width || p.height != height);
        if stale {
            let ring = (0..PRESENT_RING)
                .map(|_| {
                    self.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("concat monitor"),
                        size: wgpu::Extent3d {
                            width,
                            height,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::TEXTURE_BINDING
                            | wgpu::TextureUsages::COPY_SRC,
                        view_formats: &[],
                    })
                })
                .collect();
            self.presentable = Some(Presentable {
                width,
                height,
                ring,
                next: 0,
            });
        }
        let presentable = self.presentable.as_mut().expect("just ensured");
        let texture = presentable.ring[presentable.next].clone();
        presentable.next = (presentable.next + 1) % PRESENT_RING;
        texture
    }

    /// Every layer of `plan` uploaded, treated and placed, with every
    /// treatment applied over the stack beneath its track without a pixel
    /// leaving the GPU: the stack below a treatment is drawn into a pooled
    /// texture, the passes run over that, and the result, blended back
    /// over the untreated stack by the strength, becomes the ground the
    /// rest is drawn on. What comes back are the draws and quads for the
    /// final render, into whichever target the caller wants.
    fn prepare(&mut self, plan: &FramePlan) -> (Vec<Draw>, Vec<Vertex>) {
        self.output = plan.output;
        self.used.values_mut().for_each(|used| *used = 0);
        self.composites += 1;
        let (width, height) = (plan.width, plan.height);
        let seconds = plan.seconds();
        let mut treatments: Vec<&PlannedTreatment> = plan.treatments.iter().collect();
        treatments.sort_by_key(|treatment| treatment.track);
        let mut ground: Option<usize> = None;
        let mut next = 0;
        for treatment in treatments {
            let mut draws = Vec::new();
            let mut vertices = Vec::new();
            if let Some(index) = ground {
                self.push_pooled(&mut draws, &mut vertices, width, height, index, 1.0);
            }
            while next < plan.layers.len() && plan.layers[next].track < treatment.track {
                self.push_layer(&mut draws, &mut vertices, plan, &plan.layers[next]);
                next += 1;
            }
            let below = self.render_pooled(width, height, &draws, &vertices, wgpu::Color::BLACK);
            let strength = treatment.strength.clamp(0.0, 1.0);
            if strength <= 0.0 || treatment.effects.is_empty() {
                ground = Some(below);
                continue;
            }
            // A treatment has no single clip of its own - it runs over
            // whatever stack sits beneath it - so `clip_time` falls back
            // to the timeline's own clock, exactly what a package read
            // before this existed.
            let treated = self.run_passes(width, height, below, &treatment.effects, seconds, 0.0);
            ground = Some(if strength >= 1.0 {
                treated
            } else {
                let mut draws = Vec::new();
                let mut vertices = Vec::new();
                self.push_pooled(&mut draws, &mut vertices, width, height, below, 1.0);
                self.push_pooled(&mut draws, &mut vertices, width, height, treated, strength);
                self.render_pooled(width, height, &draws, &vertices, wgpu::Color::BLACK)
            });
        }
        let mut draws = Vec::new();
        let mut vertices = Vec::new();
        if let Some(index) = ground {
            self.push_pooled(&mut draws, &mut vertices, width, height, index, 1.0);
        }
        for layer in &plan.layers[next..] {
            self.push_layer(&mut draws, &mut vertices, plan, layer);
        }
        (draws, vertices)
    }

    /// One layer's draw: its picture uploaded, made and treated, its quad
    /// placed. The picture the quad samples is the source itself, with the
    /// crop and the flips folded into the texel coordinates, unless the
    /// effects have to see it cropped, flipped and fitted first - then it
    /// is made at its fitted size the way the CPU reference makes it, and
    /// the effects run over that.
    fn push_layer(
        &mut self,
        draws: &mut Vec<Draw>,
        vertices: &mut Vec<Vertex>,
        plan: &FramePlan,
        layer: &PlannedLayer,
    ) {
        let opacity = layer.weight();
        if opacity <= 0.0 {
            return;
        }
        let Some(source) = &layer.source else {
            return;
        };
        let geometry = layer.geometry(source, plan.width, plan.height);
        let seconds = plan.seconds();
        let clip_start = layer.clip_start.as_f64() as f32;
        let (size, texture, uvs, flips) = if layer.needs_preparing(&geometry) {
            let uploaded = self.upload(source);
            let made = self.make_picture(
                uploaded,
                (source.width(), source.height()),
                &geometry,
                layer.flip_h,
                layer.flip_v,
            );
            let treated = self.run_passes(
                geometry.fitted.0,
                geometry.fitted.1,
                made,
                &layer.effects,
                seconds,
                clip_start,
            );
            (
                geometry.fitted,
                treated,
                geometry.prepared(),
                (false, false),
            )
        } else {
            let mut index = self.upload(source);
            if !layer.effects.is_empty() {
                index = self.run_passes(
                    source.width(),
                    source.height(),
                    index,
                    &layer.effects,
                    seconds,
                    clip_start,
                );
            }
            (
                (source.width(), source.height()),
                index,
                geometry,
                (layer.flip_h, layer.flip_v),
            )
        };
        let mask = self.mask_of(layer.mask.as_deref());
        let ground = matches!(layer.blend, Blend::Lighten | Blend::Darken)
            .then(|| self.claim(plan.width, plan.height));
        draws.push(Draw {
            size,
            texture,
            blend: layer.blend,
            mask,
            ground,
        });
        vertices.extend_from_slice(&Self::quad(
            &geometry,
            &uvs,
            flips,
            opacity,
            layer.shading(),
            plan.width,
            plan.height,
        ));
    }

    /// The mask a draw binds: the layer's, uploaded, or the one white
    /// pixel for a layer without one.
    fn mask_of(&mut self, mask: Option<&Frame>) -> (u32, u32, usize) {
        match mask {
            Some(mask) => (mask.width(), mask.height(), self.upload(mask)),
            None => {
                let white = std::sync::Arc::clone(&self.white);
                (1, 1, self.upload(&white))
            }
        }
    }

    /// The source through its crop, flips and fit, drawn into a pooled
    /// texture of the fitted size over nothing: the picture as it will be
    /// seen, for the effects to run over. `source` is the pooled index of
    /// the upload.
    fn make_picture(
        &mut self,
        source: usize,
        source_size: (u32, u32),
        geometry: &Geometry,
        flip_h: bool,
        flip_v: bool,
    ) -> usize {
        let (width, height) = geometry.fitted;
        let mask = self.mask_of(None);
        let draws = vec![Draw {
            size: source_size,
            texture: source,
            blend: Blend::Normal,
            ground: None,
            mask,
        }];
        let corner = |x: f32, y: f32, u: f32, v: f32| {
            let (su, sv) = geometry.uv_of(u, v, flip_h, flip_v);
            Vertex {
                position: [x, y],
                uv: [su, sv],
                opacity: 1.0,
                scale: 1.0,
                offset: [0.0; 3],
                edges: [2.0, -1.0],
                pic: [u, v],
            }
        };
        let vertices = [
            corner(-1.0, 1.0, 0.0, 0.0),
            corner(1.0, 1.0, 1.0, 0.0),
            corner(-1.0, -1.0, 0.0, 1.0),
            corner(1.0, 1.0, 1.0, 0.0),
            corner(1.0, -1.0, 1.0, 1.0),
            corner(-1.0, -1.0, 0.0, 1.0),
        ];
        self.render_pooled(width, height, &draws, &vertices, wgpu::Color::TRANSPARENT)
    }

    /// A draw of a pooled texture the size of the output, over the whole
    /// of it: how a stack already drawn is used as the ground for more.
    fn push_pooled(
        &mut self,
        draws: &mut Vec<Draw>,
        vertices: &mut Vec<Vertex>,
        width: u32,
        height: u32,
        index: usize,
        opacity: f32,
    ) {
        let mask = self.mask_of(None);
        draws.push(Draw {
            size: (width, height),
            texture: index,
            blend: Blend::Normal,
            ground: None,
            mask,
        });
        let corner = |x: f32, y: f32, u: f32, v: f32| Vertex {
            position: [x, y],
            uv: [u, v],
            opacity: opacity.clamp(0.0, 1.0),
            scale: 1.0,
            offset: [0.0; 3],
            edges: [2.0, -1.0],
            pic: [u, v],
        };
        vertices.extend_from_slice(&[
            corner(-1.0, 1.0, 0.0, 0.0),
            corner(1.0, 1.0, 1.0, 0.0),
            corner(-1.0, -1.0, 0.0, 1.0),
            corner(1.0, 1.0, 1.0, 0.0),
            corner(1.0, -1.0, 1.0, 1.0),
            corner(-1.0, -1.0, 0.0, 1.0),
        ]);
    }

    /// Writes the quads for the next render, growing the buffer when a
    /// frame has more layers than any before it.
    fn write_vertices(&mut self, vertices: &[Vertex]) {
        if vertices.len() > self.vertex_capacity {
            self.vertex_capacity = vertices.len().next_power_of_two();
            self.vertices = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("concat quads"),
                size: (std::mem::size_of::<Vertex>() * self.vertex_capacity) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !vertices.is_empty() {
            self.queue
                .write_buffer(&self.vertices, 0, as_bytes(vertices));
        }
    }

    /// Renders `draws` into a fresh pooled texture of `width` by `height`
    /// over `clear`, and returns its index: a stack drawn so far, kept on
    /// the GPU as the ground for a treatment or for the layers above it,
    /// or a picture made for its effects.
    fn render_pooled(
        &mut self,
        width: u32,
        height: u32,
        draws: &[Draw],
        vertices: &[Vertex],
        clear: wgpu::Color,
    ) -> usize {
        let target = self.claim(width, height);
        self.write_vertices(vertices);
        let texture = &self.pool[&(width, height)][target].texture;
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self.encode(&view, texture, draws, clear);
        self.queue.submit([encoder.finish()]);
        target
    }

    /// Draws `plan` into a texture that stays on the GPU, and hands it
    /// back: `Rgba8Unorm`, bindable and renderable, exactly the plan's
    /// size. The texture is one of a small ring, so the caller may keep
    /// showing the previous one while this draws. `None` when the device
    /// is dead; the caller then falls back to [`Compositor::render`] on a
    /// CPU compositor.
    pub fn render_texture(&mut self, plan: &FramePlan) -> Option<wgpu::Texture> {
        if self.dead {
            return None;
        }
        let (draws, vertices) = self.prepare(plan);
        self.write_vertices(&vertices);
        let (canvas, canvas_texture, canvas_view) = self.canvas(plan.width, plan.height);
        let texture = self.presentable(plan.width, plan.height);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encode(&canvas_view, &canvas_texture, &draws, wgpu::Color::BLACK);
        self.resolve(&mut encoder, (plan.width, plan.height), canvas, &view);
        self.queue.submit([encoder.finish()]);
        self.retire();
        Some(texture)
    }

    /// Runs `pass` once over a sixteen-pixel-square picture and waits at
    /// most `timeout` for the device: what a community package has to
    /// survive before it is enabled. A pass the driver refuses is an
    /// error; a pass that does not finish in time is an error too, and
    /// this compositor is dead from then on, since a device mid-hang
    /// cannot be trusted with the next frame.
    pub fn trial(&mut self, pass: &ShaderPass, timeout: std::time::Duration) -> Result<(), String> {
        self.trial_at(pass, 16, timeout)
    }

    /// [`WgpuCompositor::trial`] over a picture `side` pixels square. A
    /// loop that is bounded but enormous costs a sixteen-pixel trial
    /// nothing and a real frame minutes; a trial at a few hundred pixels
    /// a side is what tells the two apart (audit 2026-09-23, #7).
    pub fn trial_at(
        &mut self,
        pass: &ShaderPass,
        side: u32,
        timeout: std::time::Duration,
    ) -> Result<(), String> {
        if self.dead {
            return Err("the GPU device is dead".to_owned());
        }
        let side = side.max(1);
        let mut picture = Frame::transparent(side, side);
        picture.fill([128, 96, 64, 255]);
        let mut layer =
            PlannedLayer::picture(crate::plan::detached_clip(), std::sync::Arc::new(picture));
        layer.effects = vec![pass.clone()];
        let plan = FramePlan {
            layers: vec![layer],
            ..FramePlan::empty(side, side)
        };
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let (draws, vertices) = self.prepare(&plan);
        if self.refused.contains(&pass.key) {
            let _ = pollster::block_on(scope.pop());
            return Err("the driver refused the pass's pipeline".to_owned());
        }
        self.write_vertices(&vertices);
        let (canvas, canvas_texture, canvas_view) = self.canvas(side, side);
        let texture = self.presentable(side, side);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encode(&canvas_view, &canvas_texture, &draws, wgpu::Color::BLACK);
        self.resolve(&mut encoder, (side, side), canvas, &view);
        self.queue.submit([encoder.finish()]);
        let waited = self.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(timeout),
        });
        let refused = pollster::block_on(scope.pop());
        self.retire();
        if let Some(error) = refused {
            return Err(format!("the driver refused the pass: {error}"));
        }
        match waited {
            Ok(_) => Ok(()),
            Err(error) => {
                self.dead = true;
                Err(format!(
                    "the pass did not finish within {timeout:?}: {error}"
                ))
            }
        }
    }

    /// `passes` over a picture `side` pixels square of one colour, straight
    /// RGBA in the working space, and the colour of its middle pixel after
    /// them, read back in the half floats the working space holds: what an
    /// effect package's probes are checked against, with nothing between
    /// the shaders and the numbers - no upload's gamma on the way in, no
    /// resolve's clipping on the way out. The passes see the timeline at
    /// `seconds`, their clip just begun. None when the device is dead or
    /// the read-back fails.
    pub fn probe(
        &mut self,
        passes: &[ShaderPass],
        colour: [f32; 4],
        side: u32,
        seconds: f32,
    ) -> Option<[f32; 4]> {
        if self.dead {
            return None;
        }
        let side = side.max(1);
        self.used.values_mut().for_each(|used| *used = 0);
        self.composites += 1;
        let source = self.filled(side, colour);
        let drawn = self.run_passes(side, side, source, passes, seconds, seconds);
        self.middle_of(side, drawn)
    }

    /// [`WgpuCompositor::probe`] for a transition: `pass` combining a
    /// picture of `from` with one of `to`, and the middle pixel of the cut.
    /// None when the device is dead, the shader will not build, or the
    /// read-back fails.
    pub fn probe_transition(
        &mut self,
        pass: &TransitionPass,
        from: [f32; 4],
        to: [f32; 4],
        side: u32,
        seconds: f32,
    ) -> Option<[f32; 4]> {
        if self.dead {
            return None;
        }
        let side = side.max(1);
        self.used.values_mut().for_each(|used| *used = 0);
        self.composites += 1;
        let from_index = self.filled(side, from);
        let to_index = self.filled(side, to);
        let target = self.claim(side, side);
        let pool = &self.pool[&(side, side)];
        let view = |index: usize| {
            pool[index]
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (from_view, to_view, target_view) = (view(from_index), view(to_index), view(target));
        let Some(encoder) = self.encode_transition(
            side,
            side,
            seconds,
            &from_view,
            &to_view,
            pass,
            &target_view,
        ) else {
            self.retire();
            return None;
        };
        self.queue.submit([encoder.finish()]);
        self.middle_of(side, target)
    }

    /// A pooled texture `side` square claimed and filled with `colour`,
    /// written straight into its half floats.
    fn filled(&mut self, side: u32, colour: [f32; 4]) -> usize {
        let index = self.claim(side, side);
        let texel: Vec<u8> = colour
            .iter()
            .flat_map(|channel| half::f16::from_f32(*channel).to_le_bytes())
            .collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.pool[&(side, side)][index].texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texel.repeat((side * side) as usize),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * WORK_BYTES as u32),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: 1,
            },
        );
        index
    }

    /// The middle texel of the pooled texture at `index`, `side` square,
    /// read back as floats; the composite retired whether or not it reads.
    fn middle_of(&mut self, side: u32, index: usize) -> Option<[f32; 4]> {
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("concat probe"),
            size: wgpu::COPY_BYTES_PER_ROW_ALIGNMENT.into(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("concat probe"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.pool[&(side, side)][index].texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: side / 2,
                    y: side / 2,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: Some(1),
                },
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        let (mapped_tx, mapped_rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped_tx.send(result);
        });
        let waited = self.device.poll(wgpu::PollType::wait_indefinitely());
        self.retire();
        if waited.is_err() || !matches!(mapped_rx.try_recv(), Ok(Ok(()))) {
            return None;
        }
        let data = slice.get_mapped_range().ok()?;
        let mut out = [0.0; 4];
        for (channel, bytes) in out
            .iter_mut()
            .zip(data[..WORK_BYTES as usize].chunks_exact(2))
        {
            *channel = half::f16::from_le_bytes([bytes[0], bytes[1]]).to_f32();
        }
        Some(out)
    }

    /// The render passes: every draw over `clear` into `view`, which is a
    /// view of `target`. One pass, except that a Lighten or Darken layer
    /// needs the ground as it stands: the pass ends, the target is copied
    /// into the draw's ground texture, and a new pass loads what is there
    /// and carries on. Returns the encoder so the caller can add a readback
    /// before submitting.
    fn encode(
        &self,
        view: &wgpu::TextureView,
        target: &wgpu::Texture,
        draws: &[Draw],
        clear: wgpu::Color,
    ) -> wgpu::CommandEncoder {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("concat composite"),
            });
        let size = (target.width(), target.height());
        let mut load = wgpu::LoadOp::Clear(clear);
        let mut pending: Vec<(usize, &Draw)> = Vec::new();
        for (index, draw) in draws.iter().enumerate() {
            let Some(ground) = draw.ground else {
                pending.push((index, draw));
                continue;
            };
            self.draw_segment(&mut encoder, view, size, load, &pending);
            pending.clear();
            load = wgpu::LoadOp::Load;
            encoder.copy_texture_to_texture(
                target.as_image_copy(),
                self.pool[&size][ground].texture.as_image_copy(),
                wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
            );
            self.draw_segment(&mut encoder, view, size, load, &[(index, draw)]);
        }
        if !pending.is_empty() || matches!(load, wgpu::LoadOp::Clear(_)) {
            self.draw_segment(&mut encoder, view, size, load, &pending);
        }
        encoder
    }

    /// One render pass over `view`, `size` pixels: these draws, in order,
    /// each with the pipeline its blend wants.
    fn draw_segment(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        size: (u32, u32),
        load: wgpu::LoadOp<wgpu::Color>,
        draws: &[(usize, &Draw)],
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("concat composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        for &(index, draw) in draws {
            match draw.ground {
                Some(ground) => {
                    let which = usize::from(draw.blend == Blend::Darken);
                    pass.set_pipeline(&self.ground_pipelines[which]);
                    pass.set_bind_group(2, &self.pool[&size][ground].bind_group, &[]);
                }
                None => {
                    let which = Blend::ALL
                        .iter()
                        .position(|mode| *mode == draw.blend)
                        .unwrap_or(0);
                    pass.set_pipeline(&self.pipelines[which]);
                }
            }
            pass.set_bind_group(0, &self.pool[&draw.size][draw.texture].bind_group, &[]);
            let (mask_w, mask_h, mask) = draw.mask;
            pass.set_bind_group(1, &self.pool[&(mask_w, mask_h)][mask].bind_group, &[]);
            let first = (index * 6) as u32;
            pass.draw(first..first + 6, 0..1);
        }
    }

    /// Retires texture sizes the timeline has moved past. 300 unclaimed
    /// composites (ten seconds of 30fps export) says a size is gone for
    /// good, not just between two clips of it.
    fn retire(&mut self) {
        for (&key, used) in &self.used {
            let idle = self.idle.entry(key).or_insert(0);
            *idle = if *used == 0 { *idle + 1 } else { 0 };
        }
        let doomed: Vec<(u32, u32)> = self
            .idle
            .iter()
            .filter(|(_, idle)| **idle > 300)
            .map(|(key, _)| *key)
            .collect();
        for key in doomed {
            if let Some(pool) = self.pool.remove(&key) {
                self.pool_bytes -=
                    pool.len() as u64 * u64::from(key.0) * u64::from(key.1) * WORK_BYTES;
                self.pool_textures -= pool.len();
            }
            self.used.remove(&key);
            self.idle.remove(&key);
            self.staging.remove(&key);
            self.deep_staging.remove(&key);
        }
        Self::trim(
            &mut self.luts,
            &mut self.luts_drawn,
            LUT_CACHE,
            self.composites,
        );
        Self::trim(
            &mut self.reveals,
            &mut self.reveals_drawn,
            REVEAL_CACHE,
            self.composites,
        );
        // Back under budget, least recently drawn first, never a texture
        // this composite claimed: a pool grows past the budget only when
        // one composite needs that much.
        while self.pool_bytes > POOL_BUDGET || self.pool_textures > POOL_TEXTURES {
            let oldest = self
                .pool
                .iter()
                .flat_map(|(key, pool)| {
                    let claimed = self.used.get(key).copied().unwrap_or(0);
                    (claimed..pool.len()).map(move |slot| (*key, slot, pool[slot].drawn))
                })
                .min_by_key(|(_, _, drawn)| *drawn);
            let Some((key, slot, _)) = oldest else {
                break;
            };
            self.pool
                .get_mut(&key)
                .expect("found above")
                .swap_remove(slot);
            self.pool_bytes -= u64::from(key.0) * u64::from(key.1) * WORK_BYTES;
            self.pool_textures -= 1;
        }
    }

    /// `cache` down to `cap` entries, least recently used first by `drawn`,
    /// keeping whatever composite `now` used.
    fn trim(
        cache: &mut HashMap<u64, wgpu::BindGroup>,
        drawn: &mut HashMap<u64, u64>,
        cap: usize,
        now: u64,
    ) {
        if cache.len() <= cap {
            return;
        }
        let mut oldest: Vec<(u64, u64)> = cache
            .keys()
            .map(|id| (drawn.get(id).copied().unwrap_or(0), *id))
            .filter(|(when, _)| *when < now)
            .collect();
        oldest.sort_unstable();
        for (_, id) in oldest.into_iter().take(cache.len() - cap) {
            cache.remove(&id);
            drawn.remove(&id);
        }
    }

    /// Whether the frame being drawn goes to an HDR file: an HDR plan, with
    /// HDR delivery on (`Compositor::deliver_hdr`).
    fn delivering_hdr(&self) -> bool {
        self.deliver_hdr && self.output != concat_core::frame::Signal::Sdr
    }

    /// The reusable readback target for this output size: eight bits a
    /// channel for an SDR frame, sixteen-bit integers for an HDR file's.
    fn target(&mut self, width: u32, height: u32) -> &Target {
        let deep = self.delivering_hdr();
        let slot = if deep {
            &mut self.deep_target
        } else {
            &mut self.target
        };
        let stale = slot
            .as_ref()
            .is_none_or(|target| target.width != width || target.height != height);
        if stale {
            let (format, bytes) = if deep {
                (wgpu::TextureFormat::Rgba16Uint, 8)
            } else {
                (wgpu::TextureFormat::Rgba8Unorm, 4)
            };
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("concat output"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let padded_row = (width as usize * bytes).div_ceil(ROW_ALIGN) * ROW_ALIGN;
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("concat readback"),
                size: (padded_row * height as usize) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            *slot = Some(Target {
                width,
                height,
                texture,
                staging,
                padded_row,
            });
        }
        slot.as_ref().expect("just ensured")
    }

    /// The readback target `target` last ensured for the frame in hand.
    fn current_target(&self) -> &Target {
        if self.delivering_hdr() {
            self.deep_target.as_ref()
        } else {
            self.target.as_ref()
        }
        .expect("ensured by target")
    }

    /// Claims a pooled texture of the layer's size holding the frame's
    /// pixels: the one that already does, moved into this frame's claimed
    /// run, or a fresh claim with the pixels uploaded into it.
    fn upload(&mut self, frame: &Frame) -> usize {
        use concat_core::frame::{Depth, Signal};
        let key = (frame.width(), frame.height());
        // An HDR frame kept HDR for an HDR timeline is another picture
        // from the same frame conformed for an SDR one, so it is held
        // apart: frame ids count up from one, and never reach the top bit.
        let keep = frame.depth() == Depth::Sixteen
            && matches!(frame.signal(), Signal::Hlg | Signal::Pq)
            && self.output != Signal::Sdr;
        let identity = frame.id() | if keep { 1 << 63 } else { 0 };
        let used = self.used.get(&key).copied().unwrap_or(0);
        if let Some(pool) = self.pool.get_mut(&key)
            && let Some(found) = (used..pool.len()).find(|&slot| pool[slot].holds == identity)
        {
            pool.swap(used, found);
            pool[used].drawn = self.composites;
            *self.used.entry(key).or_insert(0) = used + 1;
            return used;
        }
        let index = self.claim(frame.width(), frame.height());
        self.uploads += 1;
        self.pool.get_mut(&key).expect("just claimed")[index].holds = identity;
        // The eight bits go into this size's staging texture, and a copy
        // on the GPU brings them into the pooled texture in the working
        // format - the one place a frame's pixels change format, and where
        // a clip's own colour will be brought into the working space. The
        // write lands before the submit that copies it, and the next
        // upload's write after it, so one staging texture a size serves.
        let deep = frame.depth() == Depth::Sixteen;
        let (staging, staging_group) = if deep {
            self.deep_staging_for(frame.width(), frame.height())
        } else {
            self.staging_for(frame.width(), frame.height())
        };
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &staging,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width() * frame.bytes_per_pixel() as u32),
                rows_per_image: Some(frame.height()),
            },
            wgpu::Extent3d {
                width: frame.width(),
                height: frame.height(),
                depth_or_array_layers: 1,
            },
        );
        let view = self.pool[&key][index]
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("concat upload"),
            });
        let pipeline = if deep {
            &self.deep_pipelines[match (frame.signal(), keep) {
                (Signal::Hlg, false) => 1,
                (Signal::Pq, false) => 2,
                (Signal::Hlg, true) => 3,
                (Signal::Pq, true) => 4,
                (Signal::Sdr | Signal::SdrWide, _) => 0,
            }]
        } else {
            &self.upload_pipeline
        };
        Self::copy(&mut encoder, pipeline, &staging_group, &view);
        self.queue.submit([encoder.finish()]);
        index
    }

    /// A pooled texture of the output's size, claimed for the composite in
    /// hand, to draw the frame into in the working format before it is
    /// resolved into eight bits: its index, the texture, and a view of it.
    fn canvas(&mut self, width: u32, height: u32) -> (usize, wgpu::Texture, wgpu::TextureView) {
        let index = self.claim(width, height);
        let texture = self.pool[&(width, height)][index].texture.clone();
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (index, texture, view)
    }

    /// Records the finished frame on the pooled `canvas` of `size` copied
    /// into `into`, eight bits a channel: the one conversion out of the
    /// working format, where the timeline's output transform is - as it is
    /// for SDR, rolled off for an HDR timeline on an SDR screen.
    fn resolve(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        size: (u32, u32),
        canvas: usize,
        into: &wgpu::TextureView,
    ) {
        let pipeline = if self.delivering_hdr() {
            &self.hdr_pipelines[usize::from(self.output == concat_core::frame::Signal::Pq)]
        } else if self.output == concat_core::frame::Signal::Sdr {
            &self.resolve_pipeline
        } else {
            &self.tone_mapped_pipeline
        };
        Self::copy(
            encoder,
            pipeline,
            &self.pool[&size][canvas].bind_group,
            into,
        );
    }

    /// This size's sixteen-bit staging texture and its bind group, made on
    /// first use.
    fn deep_staging_for(&mut self, width: u32, height: u32) -> (wgpu::Texture, wgpu::BindGroup) {
        if let Some((texture, group)) = self.deep_staging.get(&(width, height)) {
            return (texture.clone(), group.clone());
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concat deep staging"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat deep staging"),
            layout: &self.deep_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        self.deep_staging
            .insert((width, height), (texture.clone(), group.clone()));
        (texture, group)
    }

    /// This size's staging texture and its bind group, made on first use.
    fn staging_for(&mut self, width: u32, height: u32) -> (wgpu::Texture, wgpu::BindGroup) {
        if let Some((texture, group)) = self.staging.get(&(width, height)) {
            return (texture.clone(), group.clone());
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concat staging"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let group = self.bind_group_of(&texture, "concat staging");
        self.staging
            .insert((width, height), (texture.clone(), group.clone()));
        (texture, group)
    }

    /// A bind group for `texture` in the layer layout: the picture and the
    /// sampler.
    fn bind_group_of(&self, texture: &wgpu::Texture, label: &str) -> wgpu::BindGroup {
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Records a texel-for-texel copy of the picture bound by `from` into
    /// `into`, through `pipeline`: the upload's or the resolve's.
    fn copy(
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::RenderPipeline,
        from: &wgpu::BindGroup,
        into: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("concat copy"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: into,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipeline);
        pass.set_bind_group(0, from, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Claims a pooled texture of this size, blank, for a pass to draw into.
    fn claim(&mut self, width: u32, height: u32) -> usize {
        let key = (width, height);
        let bytes = u64::from(width) * u64::from(height) * WORK_BYTES;
        let used = *self.used.entry(key).or_insert(0);
        let pool = self.pool.entry(key).or_default();
        // The texture to draw into, from the ones not claimed yet this
        // composite: one holding no frame, first; else a new one, while the
        // pool is under budget, so the frames it holds stay cached; else
        // the least recently drawn frame gives its texture up.
        let spare = (used..pool.len())
            .find(|&slot| pool[slot].holds == 0)
            .or_else(|| {
                (self.pool_bytes + bytes > POOL_BUDGET || self.pool_textures >= POOL_TEXTURES)
                    .then(|| (used..pool.len()).min_by_key(|&slot| pool[slot].drawn))
                    .flatten()
            });
        if let Some(slot) = spare {
            pool.swap(used, slot);
        } else {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("concat layer"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WORK,
                // A render attachment too: a shader pass draws one pooled
                // texture into another of the same size. A copy source and
                // destination for the ground a Lighten or Darken reads.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("concat layer"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            pool.push(PooledTexture {
                texture,
                bind_group,
                holds: 0,
                drawn: 0,
            });
            let last = pool.len() - 1;
            pool.swap(used, last);
            self.pool_bytes += bytes;
            self.pool_textures += 1;
        }

        let pool = self.pool.get_mut(&key).expect("claimed from above");
        // Whatever is drawn into it next is not the frame it held.
        pool[used].holds = 0;
        pool[used].drawn = self.composites;
        *self.used.get_mut(&key).expect("counted above") = used + 1;
        used
    }

    /// The compiled pipeline for a pass, built the first time its key is
    /// seen. The catalogue validated the module at load, so a failure here
    /// is a driver disagreement: it is caught in an error scope, logged,
    /// and the pass is skipped from then on - the layer draws untreated -
    /// rather than reaching wgpu's uncaptured-error handler, which ends
    /// the process (audit 2026-09-23, #7).
    fn shader(&mut self, pass: &ShaderPass) {
        if self.shaders.contains_key(&pass.key) || self.refused.contains(&pass.key) {
            return;
        }
        // A package drawn in one pass binds its layer as the pool made it;
        // one drawn in several binds its passes' pictures beside it.
        let inputs = if pass.stages.is_empty() {
            self.bind_layout.clone()
        } else {
            self.pictures_layout(pass.stages.len())
        };
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&pass.key),
                source: wgpu::ShaderSource::Wgsl(pass.source.as_ref().into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&pass.key),
                bind_group_layouts: &[
                    Some(&inputs),
                    Some(&self.uniform_layout),
                    Some(&self.lut_layout),
                    Some(&self.reveal_layout),
                ],
                immediate_size: 0,
            });
        let pipeline_of = |entry: &str| {
            self.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&pass.key),
                    layout: Some(&layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("vs_main"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: WORK,
                            // A pass replaces: mixing by intensity is the
                            // shader's own last line.
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
        };
        let pipeline = pipeline_of("fs_main");
        let stages = pass
            .stages
            .iter()
            .map(|stage| pipeline_of(&stage.entry))
            .collect();
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::error!(
                "pass {}: the driver refused its pipeline: {error}",
                pass.key
            );
            self.refused.insert(pass.key.clone());
            return;
        }
        let frame = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("concat pass frame"),
            // size(vec2), time, intensity, clip_time, padded to Frame's own
            // 8-byte alignment (from its vec2 member): 20 bytes rounds to 24.
            size: 24,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("concat pass params"),
            size: pass.params.len().max(ShaderPass::MIN_PARAMS) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat pass uniforms"),
            layout: &self.uniform_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        self.shaders.insert(
            pass.key.clone(),
            CompiledShader {
                pipeline,
                frame,
                params,
                bind_group,
                stages,
            },
        );
    }

    /// Group 0 of a package whose passes draw `pictures` pictures: the
    /// layer and its sampler where a single pass binds them, then each
    /// picture, in the passes' order. Made once for each count.
    fn pictures_layout(&mut self, pictures: usize) -> wgpu::BindGroupLayout {
        if let Some(layout) = self.pictures_layouts.get(&pictures) {
            return layout.clone();
        }
        let entries: Vec<wgpu::BindGroupLayoutEntry> = [picture_entry(0), sampler_entry(1)]
            .into_iter()
            .chain((0..pictures as u32).map(|picture| picture_entry(2 + picture)))
            .collect();
        let layout = self
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("concat pass pictures"),
                entries: &entries,
            });
        self.pictures_layouts.insert(pictures, layout.clone());
        layout
    }

    /// The compiled pipeline for a transition, built the first time its key is
    /// seen. Mirrors [`WgpuCompositor::shader`] but binds two input pictures at
    /// group 0 and lets the shader own the blend.
    fn transition_shader(&mut self, pass: &TransitionPass) {
        if self.transitions.contains_key(&pass.key) || self.refused.contains(&pass.key) {
            return;
        }
        let scope = self.device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(&pass.key),
                source: wgpu::ShaderSource::Wgsl(pass.source.as_ref().into()),
            });
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(&pass.key),
                bind_group_layouts: &[
                    Some(&self.transition_layout),
                    Some(&self.uniform_layout),
                    Some(&self.lut_layout),
                ],
                immediate_size: 0,
            });
        let pipeline = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&pass.key),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: WORK,
                        // The transition owns the mix; the pipeline does none.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        if let Some(error) = pollster::block_on(scope.pop()) {
            log::error!(
                "transition {}: the driver refused its pipeline: {error}",
                pass.key
            );
            self.refused.insert(pass.key.clone());
            return;
        }
        let frame = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("concat transition frame"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let params = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("concat transition params"),
            size: pass.params.len().max(ShaderPass::MIN_PARAMS) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat transition uniforms"),
            layout: &self.uniform_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        self.transitions.insert(
            pass.key.clone(),
            CompiledShader {
                pipeline,
                frame,
                params,
                bind_group,
                stages: Vec::new(),
            },
        );
    }

    /// The render pass that combines two pictures through `pass`, the
    /// transition's shader, into `target`: recorded, not submitted. `None`
    /// when the shader will not build.
    #[allow(clippy::too_many_arguments)]
    fn encode_transition(
        &mut self,
        width: u32,
        height: u32,
        time: f32,
        from_view: &wgpu::TextureView,
        to_view: &wgpu::TextureView,
        pass: &TransitionPass,
        target: &wgpu::TextureView,
    ) -> Option<wgpu::CommandEncoder> {
        self.transition_shader(pass);
        if !self.transitions.contains_key(&pass.key) {
            return None;
        }
        let lut_id = self.lut_group(pass.lut.as_deref());
        let inputs = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat transition inputs"),
            layout: &self.transition_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(from_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(to_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        // The frame block carries progress where a pass carries intensity.
        {
            let shader = &self.transitions[&pass.key];
            let frame_block: [f32; 4] = [width as f32, height as f32, time, pass.progress];
            let frame_bytes: Vec<u8> = frame_block.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.queue.write_buffer(&shader.frame, 0, &frame_bytes);
            let mut params = pass.params.clone();
            params.resize(shader.params.size() as usize, 0);
            self.queue.write_buffer(&shader.params, 0, &params);
        }

        let shader = &self.transitions[&pass.key];
        let lut_group = &self.luts[&lut_id];
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("concat transition"),
            });
        {
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("concat transition"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            render.set_pipeline(&shader.pipeline);
            render.set_bind_group(0, &inputs, &[]);
            render.set_bind_group(1, &shader.bind_group, &[]);
            render.set_bind_group(2, lut_group, &[]);
            render.draw(0..3, 0..1);
        }
        Some(encoder)
    }

    /// A packaged transition drawn whole on the GPU and handed back as a
    /// texture, as [`WgpuCompositor::render_texture`] hands back a plan:
    /// `from` and `to` drawn into targets of their own, then combined
    /// through `pass` into a presentable texture. Nothing is read back or
    /// uploaded again, where [`Compositor::combine`] reads both pictures
    /// back and uploads them - three round trips a frame, which is what
    /// the monitor paid for every frame of a transition. `None` when the
    /// device is dead or the shader will not build; the caller falls back.
    pub fn render_transition_texture(
        &mut self,
        from: &FramePlan,
        to: &FramePlan,
        pass: &TransitionPass,
    ) -> Option<wgpu::Texture> {
        if self.dead {
            return None;
        }
        let (width, height, time) = (from.width, from.height, from.seconds());
        let [from_texture, to_texture] = self.stages(width, height);
        for (plan, texture) in [(from, &from_texture), (to, &to_texture)] {
            let (draws, vertices) = self.prepare(plan);
            self.write_vertices(&vertices);
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let encoder = self.encode(&view, texture, &draws, wgpu::Color::BLACK);
            self.queue.submit([encoder.finish()]);
        }
        let from_view = from_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let to_view = to_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let (canvas, _, canvas_view) = self.canvas(width, height);
        let out = self.presentable(width, height);
        let out_view = out.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encode_transition(
            width,
            height,
            time,
            &from_view,
            &to_view,
            pass,
            &canvas_view,
        )?;
        self.resolve(&mut encoder, (width, height), canvas, &out_view);
        self.queue.submit([encoder.finish()]);
        self.retire();
        Some(out)
    }

    /// The two transition stages at this size, made on first use.
    fn stages(&mut self, width: u32, height: u32) -> [wgpu::Texture; 2] {
        if let Some((w, h, stages)) = &self.stages
            && (*w, *h) == (width, height)
        {
            return stages.clone();
        }
        let make = || {
            self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("concat transition stage"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: WORK,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let stages = [make(), make()];
        self.stages = Some((width, height, stages.clone()));
        stages
    }

    /// The bind group for a pass's table, uploaded the first time its id is
    /// seen, and the identity's for a pass without one. Returns the id the
    /// group is filed under.
    fn lut_group(&mut self, lut: Option<&Lut>) -> u64 {
        static IDENTITY: std::sync::OnceLock<Lut> = std::sync::OnceLock::new();
        let lut = lut.unwrap_or_else(|| IDENTITY.get_or_init(|| Lut::identity(2)));
        self.luts_drawn.insert(lut.id, self.composites);
        if self.luts.contains_key(&lut.id) {
            return lut.id;
        }
        let size = lut.size;
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concat lut"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            // Half floats, filterable everywhere: see `Lut` for why not
            // bytes.
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let halves: Vec<u8> = lut
            .rgba
            .iter()
            .flat_map(|value| half::f16::from_f32(*value).to_le_bytes())
            .collect();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &halves,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * WORK_BYTES as u32),
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat lut"),
            layout: &self.lut_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.luts.insert(lut.id, bind_group);
        lut.id
    }

    /// The bind group for a title's reveal map, uploaded the first time its
    /// id is seen, and the identity's - reveals everything - for a pass
    /// without one. Mirrors [`WgpuCompositor::lut_group`]. Returns the id
    /// the group is filed under.
    fn reveal_group(&mut self, reveal: Option<&RevealMap>) -> u64 {
        static IDENTITY: std::sync::OnceLock<RevealMap> = std::sync::OnceLock::new();
        let reveal = reveal.unwrap_or_else(|| IDENTITY.get_or_init(RevealMap::identity));
        self.reveals_drawn.insert(reveal.id, self.composites);
        if self.reveals.contains_key(&reveal.id) {
            return reveal.id;
        }
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concat reveal map"),
            size: wgpu::Extent3d {
                width: reveal.width,
                height: reveal.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &reveal.gray,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(reveal.width),
                rows_per_image: Some(reveal.height),
            },
            wgpu::Extent3d {
                width: reveal.width,
                height: reveal.height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat reveal map"),
            layout: &self.reveal_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        self.reveals.insert(reveal.id, bind_group);
        reveal.id
    }

    /// Runs `passes` over the pooled texture `source` of `width` × `height`,
    /// and returns the index of the last texture drawn. The passes take
    /// turns between two pooled textures of the same size - each reads the
    /// one the pass before it drew and draws into the other - so a stack of
    /// effects costs two textures, not one a pass: at 8K a pass's texture
    /// is 265 MB. `source` is only ever read, so a cached frame stays as it
    /// was. Each pass is its own submission, which is what makes drawing
    /// into the texture two passes back safe, and keeps the uniforms a pass
    /// wrote the ones it reads.
    ///
    /// A package drawn in several passes draws its stages into pictures of
    /// their own first, in the same submission (see [`Self::run_stages`]).
    /// Those are claimed from the pool the first time a size is wanted and
    /// handed from one package to the next, so a stack of blurs costs one
    /// set of them.
    fn run_passes(
        &mut self,
        width: u32,
        height: u32,
        source: usize,
        passes: &[ShaderPass],
        time: f32,
        clip_start: f32,
    ) -> usize {
        let mut current = source;
        let mut turns: [Option<usize>; 2] = [None, None];
        let mut drawn = 0;
        let mut spare: HashMap<(u32, u32), Vec<usize>> = HashMap::new();
        for pass in passes {
            // A pass the driver refused does not take a turn, so the next
            // one never draws into the texture it reads.
            let target = match turns[drawn % 2] {
                Some(target) => target,
                None => {
                    let target = self.claim(width, height);
                    turns[drawn % 2] = Some(target);
                    target
                }
            };
            self.shader(pass);
            let lut_id = self.lut_group(pass.lut.as_deref());
            let reveal_id = self.reveal_group(pass.reveal_map.as_deref());
            // A pass the driver refused leaves the picture as it was.
            if !self.shaders.contains_key(&pass.key) {
                continue;
            }
            if !pass.stages.is_empty() {
                self.run_stages(
                    (width, height),
                    (current, target),
                    pass,
                    [time, time - clip_start],
                    (lut_id, reveal_id),
                    &mut spare,
                );
                current = target;
                drawn += 1;
                continue;
            }
            let shader = &self.shaders[&pass.key];
            let lut_group = &self.luts[&lut_id];
            let reveal_group = &self.reveals[&reveal_id];
            let frame_block: [f32; 6] = [
                width as f32,
                height as f32,
                time,
                pass.intensity,
                time - clip_start,
                0.0,
            ];
            let frame_bytes: Vec<u8> = frame_block.iter().flat_map(|v| v.to_le_bytes()).collect();
            self.queue.write_buffer(&shader.frame, 0, &frame_bytes);
            let mut params = pass.params.clone();
            params.resize(shader.params.size() as usize, 0);
            self.queue.write_buffer(&shader.params, 0, &params);

            let pool = &self.pool[&(width, height)];
            let view = pool[target]
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("concat pass"),
                });
            {
                let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("concat pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                render.set_pipeline(&shader.pipeline);
                render.set_bind_group(0, &pool[current].bind_group, &[]);
                render.set_bind_group(1, &shader.bind_group, &[]);
                render.set_bind_group(2, lut_group, &[]);
                render.set_bind_group(3, reveal_group, &[]);
                render.draw(0..3, 0..1);
            }
            self.queue.submit([encoder.finish()]);
            current = target;
            drawn += 1;
        }
        current
    }

    /// One package drawn in several passes, over the pooled texture `layer`,
    /// `width` by `height`, and into the pooled texture `into` of the same
    /// size: each stage into a picture of its own - from `spare` where one
    /// of its size is free, else claimed - then the last pass into `into`,
    /// all in one submission. Every pass binds the layer and the pictures
    /// drawn before it, a blank for the rest, its own among them. `time` is
    /// the timeline's seconds and `clip_time` the clip's; `lut_id` and
    /// `reveal_id` name the tables every pass binds. The pictures go back
    /// to `spare` for the next package.
    fn run_stages(
        &mut self,
        (width, height): (u32, u32),
        (layer, into): (usize, usize),
        pass: &ShaderPass,
        [time, clip_time]: [f32; 2],
        (lut_id, reveal_id): (u64, u64),
        spare: &mut HashMap<(u32, u32), Vec<usize>>,
    ) {
        let pictures: Vec<((u32, u32), usize)> = pass
            .stages
            .iter()
            .map(|stage| {
                let size = stage.size(width, height);
                let index = spare
                    .get_mut(&size)
                    .and_then(Vec::pop)
                    .unwrap_or_else(|| self.claim(size.0, size.1));
                (size, index)
            })
            .collect();
        let shader = &self.shaders[&pass.key];
        // Every pass of the package reads the same frame block: the layer's
        // size - what a knob in pixels is measured against - whatever the
        // size of the picture it draws.
        let frame_block: [f32; 6] = [
            width as f32,
            height as f32,
            time,
            pass.intensity,
            clip_time,
            0.0,
        ];
        let frame_bytes: Vec<u8> = frame_block.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.queue.write_buffer(&shader.frame, 0, &frame_bytes);
        let mut params = pass.params.clone();
        params.resize(shader.params.size() as usize, 0);
        self.queue.write_buffer(&shader.params, 0, &params);

        let view_of = |size: (u32, u32), index: usize| {
            self.pool[&size][index]
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let layer_view = view_of((width, height), layer);
        let into_view = view_of((width, height), into);
        let picture_views: Vec<wgpu::TextureView> = pictures
            .iter()
            .map(|&(size, index)| view_of(size, index))
            .collect();
        let layout = &self.pictures_layouts[&pictures.len()];
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("concat passes"),
            });
        for stage in 0..=pictures.len() {
            let entries: Vec<wgpu::BindGroupEntry> =
                [
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&layer_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ]
                .into_iter()
                .chain(picture_views.iter().enumerate().map(|(picture, view)| {
                    wgpu::BindGroupEntry {
                        binding: 2 + picture as u32,
                        resource: wgpu::BindingResource::TextureView(if picture < stage {
                            view
                        } else {
                            &self.blank
                        }),
                    }
                }))
                .collect();
            let inputs = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("concat pass pictures"),
                layout,
                entries: &entries,
            });
            let (pipeline, view) = match shader.stages.get(stage) {
                Some(pipeline) => (pipeline, &picture_views[stage]),
                None => (&shader.pipeline, &into_view),
            };
            let mut render = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("concat pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            render.set_pipeline(pipeline);
            render.set_bind_group(0, &inputs, &[]);
            render.set_bind_group(1, &shader.bind_group, &[]);
            render.set_bind_group(2, &self.luts[&lut_id], &[]);
            render.set_bind_group(3, &self.reveals[&reveal_id], &[]);
            render.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        for (size, index) in pictures {
            spare.entry(size).or_default().push(index);
        }
    }

    /// The six vertices of one layer's quad: the fitted picture scaled,
    /// turned about its centre and moved as the geometry says, the forward
    /// form of the CPU path's inverse map, sampling `uvs` through the
    /// flips, weighed by the opacity and the shading.
    fn quad(
        geometry: &Geometry,
        uvs: &Geometry,
        (flip_h, flip_v): (bool, bool),
        opacity: f32,
        shading: Shading,
        out_width: u32,
        out_height: u32,
    ) -> [Vertex; 6] {
        let (fitted_w, fitted_h) = (geometry.fitted.0 as f32, geometry.fitted.1 as f32);
        let (centre_x, centre_y) = geometry.centre;
        let (sin, cos) = geometry.rotation.sin_cos();
        let (scale_x, scale_y) = geometry.scale;

        let corner = |sx: f32, sy: f32, u: f32, v: f32| {
            // Picture-space offset from the centre, scaled per axis, then
            // rotated clockwise in y-down coordinates.
            let dx = sx * fitted_w / 2.0 * scale_x;
            let dy = sy * fitted_h / 2.0 * scale_y;
            let px = centre_x + dx * cos - dy * sin;
            let py = centre_y + dx * sin + dy * cos;
            let (tu, tv) = uvs.uv_of(u, v, flip_h, flip_v);
            Vertex {
                position: [
                    px / out_width as f32 * 2.0 - 1.0,
                    1.0 - py / out_height as f32 * 2.0,
                ],
                uv: [tu, tv],
                opacity,
                scale: shading.scale,
                offset: shading.offset,
                edges: [shading.left_edge, shading.right_edge],
                pic: [u, v],
            }
        };

        let top_left = corner(-1.0, -1.0, 0.0, 0.0);
        let top_right = corner(1.0, -1.0, 1.0, 0.0);
        let bottom_left = corner(-1.0, 1.0, 0.0, 1.0);
        let bottom_right = corner(1.0, 1.0, 1.0, 1.0);
        [
            top_left,
            top_right,
            bottom_left,
            top_right,
            bottom_right,
            bottom_left,
        ]
    }

    /// Copies the rendered target back into a [`Frame`], forcing it opaque.
    ///
    /// `None` means the mapping failed - a lost or reset device, the one GPU
    /// failure the constructor's never-panic policy cannot rule out up
    /// front. The caller falls back to the CPU compositor rather than
    /// panicking mid-export.
    /// `None` when the device did not deliver the pixels - a lost device,
    /// a failed map - and the caller then marks this compositor dead. The
    /// map's own result is what decides, not the poll's: a poll can return
    /// without the map having been served.
    fn read_back(&mut self) -> Option<Frame> {
        let deep = self.delivering_hdr();
        let target = if deep {
            self.deep_target.as_ref()?
        } else {
            self.target.as_ref()?
        };
        let (width, height, padded_row) = (target.width, target.height, target.padded_row);

        let slice = target.staging.slice(..);
        let (mapped_tx, mapped_rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = mapped_tx.send(result);
        });
        if self
            .device
            .poll(wgpu::PollType::wait_indefinitely())
            .is_err()
        {
            return None;
        }
        if !matches!(mapped_rx.try_recv(), Ok(Ok(()))) {
            return None;
        }

        if deep {
            // Sixteen-bit integers, opaque already: the rows as they are.
            let row_bytes = width as usize * 8;
            let mut pixels = vec![0u8; row_bytes * height as usize];
            {
                let data = slice.get_mapped_range().ok()?;
                for row in 0..height as usize {
                    let from = &data[row * padded_row..row * padded_row + row_bytes];
                    pixels[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(from);
                }
            }
            target.staging.unmap();
            return Frame::from_rgba64(width, height, pixels, self.output);
        }
        let mut frame = Frame::transparent(width, height);
        {
            // A range that will not map is a failed map, as above.
            let data = slice.get_mapped_range().ok()?;
            let row_bytes = width as usize * 4;
            let pixels = frame.pixels_mut();
            for row in 0..height as usize {
                let from = &data[row * padded_row..row * padded_row + row_bytes];
                pixels[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(from);
            }
            // The output goes to a screen or an encoder; neither has anything
            // to show through, and blending may have left alpha short of one.
            for pixel in pixels.chunks_exact_mut(4) {
                pixel[3] = 255;
            }
        }
        target.staging.unmap();
        Some(frame)
    }
}

impl Compositor for WgpuCompositor {
    fn render(&mut self, plan: &FramePlan) -> Frame {
        // A dead device never comes back for this instance. What it hands
        // back is black, never a mid-export panic, and `lost` says so: the
        // export stops with an error rather than write a file of black.
        if self.dead {
            return Frame::black(plan.width, plan.height);
        }
        let (draws, vertices) = self.prepare(plan);
        self.write_vertices(&vertices);
        match self.render_and_read(plan.width, plan.height, &draws) {
            Some(frame) => frame,
            None => {
                self.dead = true;
                Frame::black(plan.width, plan.height)
            }
        }
    }

    fn lost(&self) -> bool {
        self.dead
    }

    fn deliver_hdr(&mut self, on: bool) {
        self.deliver_hdr = on;
    }

    fn combine(
        &mut self,
        width: u32,
        height: u32,
        time: f32,
        from: &Frame,
        to: &Frame,
        pass: &TransitionPass,
    ) -> Option<Frame> {
        if self.dead {
            return None;
        }
        // The two pictures, uploaded into the working space as any frame
        // is, so the shader reads what the device-kept path hands it.
        self.used.values_mut().for_each(|used| *used = 0);
        let view_of = |gpu: &Self, frame: &Frame, index: usize| {
            gpu.pool[&(frame.width(), frame.height())][index]
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let from_index = self.upload(from);
        let to_index = self.upload(to);
        let from_view = view_of(self, from, from_index);
        let to_view = view_of(self, to, to_index);
        let (canvas, _, canvas_view) = self.canvas(width, height);
        self.target(width, height);
        let target_texture = self.current_target().texture.clone();
        let view = target_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encode_transition(
            width,
            height,
            time,
            &from_view,
            &to_view,
            pass,
            &canvas_view,
        )?;
        self.resolve(&mut encoder, (width, height), canvas, &view);
        {
            let target = self.current_target();
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &target.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &target.staging,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(target.padded_row as u32),
                        rows_per_image: Some(height),
                    },
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );
        }
        self.queue.submit([encoder.finish()]);
        let frame = self.read_back();
        if frame.is_none() {
            self.dead = true;
        }
        frame
    }
}

impl WgpuCompositor {
    /// Draws into the readback target and copies it out: one submit, one
    /// wait. `None` when the device did not deliver the pixels.
    fn render_and_read(&mut self, width: u32, height: u32, draws: &[Draw]) -> Option<Frame> {
        let (canvas, canvas_texture, canvas_view) = self.canvas(width, height);
        self.target(width, height);
        let target = self.current_target();
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encode(&canvas_view, &canvas_texture, draws, wgpu::Color::BLACK);
        self.resolve(&mut encoder, (width, height), canvas, &view);
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &target.staging,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(target.padded_row as u32),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        self.queue.submit([encoder.finish()]);
        self.retire();
        self.read_back()
    }
}

#[cfg(test)]
mod tests;
