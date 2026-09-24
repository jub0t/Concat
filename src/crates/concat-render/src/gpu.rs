// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The wgpu compositor.
//!
//! The second implementation of [`Compositor`](crate::Compositor):
//! [`CpuCompositor`](crate::CpuCompositor) stays the reference, and this
//! one exists to be fast. Layers are uploaded as textures, drawn as
//! transformed quads into an offscreen target, and read back as a [`Frame`].
//!
//! Deliberate parity choices, so the two backends can be diffed:
//!
//! - The target format is `Rgba8Unorm`, *not* the sRGB variant. Blending
//!   therefore happens on stored (gamma-encoded) values, exactly as the CPU
//!   path does. When the day comes to blend in linear light, both backends
//!   change together.
//! - Layer quads are sampled bilinearly with clamp-to-edge, matching the CPU
//!   path's bilinear inverse mapping.
//!
//! Construction is fallible: a machine with no usable adapter gets `None`, and
//! callers fall back to the CPU. Never panic over a missing GPU.
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

use crate::compositor::{Compositor, CpuCompositor};
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

/// One package's shader, compiled once and kept: its pipeline, and the two
/// uniform buffers every pass through it rewrites.
struct CompiledShader {
    pipeline: wgpu::RenderPipeline,
    frame: wgpu::Buffer,
    params: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
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
    /// a title, a paused clip - skip its upload.
    holds: u64,
}

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
    /// Uploaded tables by their id, the identity among them; see `lut_group`.
    luts: HashMap<u64, wgpu::BindGroup>,
    /// Uploaded reveal maps by their id, the identity among them; see
    /// `reveal_group`.
    reveals: HashMap<u64, wgpu::BindGroup>,
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
    used: HashMap<(u32, u32), usize>,
    idle: HashMap<(u32, u32), u32>,
    target: Option<Target>,
    presentable: Option<Presentable>,
    /// A one-pixel opaque white picture: the mask of a layer without one.
    white: Frame,
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
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(Self::with_device(device, queue))
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
        let texture_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let transition_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("concat transition inputs"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                texture_entry(2),
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
                        buffers: std::slice::from_ref(&vertex_layout),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
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
                    buffers: std::slice::from_ref(&vertex_layout),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
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

        Self {
            device,
            queue,
            pipelines,
            ground_pipelines,
            bind_layout,
            uniform_layout,
            lut_layout,
            reveal_layout,
            transition_layout,
            luts: HashMap::new(),
            reveals: HashMap::new(),
            shaders: HashMap::new(),
            transitions: HashMap::new(),
            refused: std::collections::HashSet::new(),
            sampler,
            vertices,
            vertex_capacity: 6 * 8,
            pool: HashMap::new(),
            used: HashMap::new(),
            target: None,
            presentable: None,
            idle: HashMap::new(),
            white: {
                let mut white = Frame::transparent(1, 1);
                white.fill([255, 255, 255, 255]);
                white
            },
            dead: false,
        }
    }

    /// Whether the device has been lost. A dead compositor answers
    /// [`Compositor::composite`] from the CPU and refuses textures.
    pub fn is_dead(&self) -> bool {
        self.dead
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
        self.used.values_mut().for_each(|used| *used = 0);
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
                let white = self.white.clone();
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
        let texture = self.presentable(plan.width, plan.height);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self.encode(&view, &texture, &draws, wgpu::Color::BLACK);
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
        let texture = self.presentable(side, side);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let encoder = self.encode(&view, &texture, &draws, wgpu::Color::BLACK);
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
            self.pool.remove(&key);
            self.used.remove(&key);
            self.idle.remove(&key);
        }
    }

    /// The reusable render target for this output size.
    fn target(&mut self, width: u32, height: u32) -> &Target {
        let stale = self
            .target
            .as_ref()
            .is_none_or(|target| target.width != width || target.height != height);
        if stale {
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
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let padded_row = (width as usize * 4).div_ceil(ROW_ALIGN) * ROW_ALIGN;
            let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("concat readback"),
                size: (padded_row * height as usize) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            self.target = Some(Target {
                width,
                height,
                texture,
                staging,
                padded_row,
            });
        }
        self.target.as_ref().expect("just ensured")
    }

    /// Claims a pooled texture of the layer's size holding the frame's
    /// pixels: the one that already does, moved into this frame's claimed
    /// run, or a fresh claim with the pixels uploaded into it.
    fn upload(&mut self, frame: &Frame) -> usize {
        let key = (frame.width(), frame.height());
        let identity = frame.id();
        let used = self.used.get(&key).copied().unwrap_or(0);
        if let Some(pool) = self.pool.get_mut(&key)
            && let Some(found) = (used..pool.len()).find(|&slot| pool[slot].holds == identity)
        {
            pool.swap(used, found);
            *self.used.entry(key).or_insert(0) = used + 1;
            return used;
        }
        let index = self.claim(frame.width(), frame.height());
        let pooled = &mut self.pool.get_mut(&key).expect("just claimed")[index];
        pooled.holds = identity;
        let texture = &pooled.texture;
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            frame.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width() * 4),
                rows_per_image: Some(frame.height()),
            },
            wgpu::Extent3d {
                width: frame.width(),
                height: frame.height(),
                depth_or_array_layers: 1,
            },
        );
        index
    }

    /// Claims a pooled texture of this size, blank, for a pass to draw into.
    fn claim(&mut self, width: u32, height: u32) -> usize {
        let key = (width, height);
        let used = self.used.entry(key).or_insert(0);
        let pool = self.pool.entry(key).or_default();

        if *used == pool.len() {
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
                format: wgpu::TextureFormat::Rgba8Unorm,
                // A render attachment too: a shader pass draws one pooled
                // texture into another of the same size.
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
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
            });
        }

        let index = *used;
        *used += 1;
        // Whatever is drawn into it next is not the frame it held.
        pool[index].holds = 0;
        index
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
                    Some(&self.bind_layout),
                    Some(&self.uniform_layout),
                    Some(&self.lut_layout),
                    Some(&self.reveal_layout),
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
                        format: wgpu::TextureFormat::Rgba8Unorm,
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
            });
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
            },
        );
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
                        format: wgpu::TextureFormat::Rgba8Unorm,
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
            },
        );
    }

    /// A texture holding a whole frame's pixels, for a transition input. Made
    /// fresh each combine rather than pooled: a transition is a short window,
    /// and its two inputs are full-frame, so the pool would only churn.
    fn input_texture(&self, frame: &Frame) -> wgpu::Texture {
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concat transition input"),
            size: wgpu::Extent3d {
                width: frame.width(),
                height: frame.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
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
            frame.pixels(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(frame.width() * 4),
                rows_per_image: Some(frame.height()),
            },
            wgpu::Extent3d {
                width: frame.width(),
                height: frame.height(),
                depth_or_array_layers: 1,
            },
        );
        texture
    }

    /// The bind group for a pass's table, uploaded the first time its id is
    /// seen, and the identity's for a pass without one. Returns the id the
    /// group is filed under.
    fn lut_group(&mut self, lut: Option<&Lut>) -> u64 {
        static IDENTITY: std::sync::OnceLock<Lut> = std::sync::OnceLock::new();
        let lut = lut.unwrap_or_else(|| IDENTITY.get_or_init(|| Lut::identity(2)));
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
            format: wgpu::TextureFormat::Rgba8Unorm,
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
            &lut.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 4),
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
    /// each drawing into a fresh pooled texture of the same size, and
    /// returns the index of the last one drawn. Each pass is its own
    /// submission so the uniforms it wrote are the ones it reads.
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
        for pass in passes {
            let target = self.claim(width, height);
            self.shader(pass);
            let lut_id = self.lut_group(pass.lut.as_deref());
            let reveal_id = self.reveal_group(pass.reveal_map.as_deref());
            // A pass the driver refused leaves the picture as it was.
            let Some(shader) = self.shaders.get(&pass.key) else {
                continue;
            };
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
        }
        current
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
        let target = self.target.as_ref()?;
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

        let mut frame = Frame::transparent(width, height);
        {
            let data = slice.get_mapped_range();
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
        // A dead device never comes back for this instance; the CPU
        // reference is the same pixels, slower - never a mid-export panic.
        if self.dead {
            return CpuCompositor.render(plan);
        }
        let (draws, vertices) = self.prepare(plan);
        self.write_vertices(&vertices);
        match self.render_and_read(plan.width, plan.height, &draws) {
            Some(frame) => frame,
            None => {
                self.dead = true;
                CpuCompositor.render(plan)
            }
        }
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
        self.transition_shader(pass);
        if !self.transitions.contains_key(&pass.key) {
            return None;
        }
        let lut_id = self.lut_group(pass.lut.as_deref());

        // The two pictures, uploaded and bound at group 0.
        let from_texture = self.input_texture(from);
        let to_texture = self.input_texture(to);
        let from_view = from_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let to_view = to_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let inputs = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("concat transition inputs"),
            layout: &self.transition_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&from_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&to_view),
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

        self.target(width, height);
        {
            let target = self.target.as_ref().expect("just ensured");
            let view = target
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
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
                        view: &view,
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
        }
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
        self.target(width, height);
        let target = self.target.as_ref().expect("just ensured");
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.encode(&view, &target.texture, draws, wgpu::Color::BLACK);
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
