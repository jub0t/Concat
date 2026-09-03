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

use std::collections::HashMap;

use concat_core::frame::Frame;

use crate::compositor::{Compositor, CpuCompositor, Layer};

/// Bytes per row must be a multiple of this for a texture-to-buffer copy.
const ROW_ALIGN: usize = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;

/// One vertex of a layer quad: clip-space position, texel coordinates, and
/// the layer's opacity riding along so no uniforms are needed.
#[repr(C)]
#[derive(Clone, Copy)]
struct Vertex {
    position: [f32; 2],
    uv: [f32; 2],
    opacity: f32,
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
}

struct VsOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) opacity: f32,
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.opacity = in.opacity;
    return out;
}

@group(0) @binding(0) var layer_texture: texture_2d<f32>;
@group(0) @binding(1) var layer_sampler: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let colour = textureSample(layer_texture, layer_sampler, in.uv);
    let alpha = colour.a * in.opacity;
    // Premultiplied output; the pipeline blends ONE / ONE_MINUS_SRC_ALPHA,
    // which together is the same source-over the CPU path computes.
    return vec4<f32>(colour.rgb * alpha, alpha);
}
"#;

/// A cached layer texture and its bind group, reusable for any layer of the
/// same size.
struct PooledTexture {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// The reusable output target and its readback buffer, for one output size.
struct Target {
    width: u32,
    height: u32,
    texture: wgpu::Texture,
    staging: wgpu::Buffer,
    padded_row: usize,
}

/// A compositor that draws on the GPU. See the module docs.
pub struct WgpuCompositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
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
    /// Set when a readback fails - a lost or reset device. The compositor
    /// then answers every composite from the CPU reference instead: slower,
    /// always correct, and never a panic in the middle of an export.
    dead: bool,
}

impl WgpuCompositor {
    /// Builds a compositor on the best available adapter, or `None` when the
    /// machine has nothing usable - callers fall back to the CPU path.
    pub fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;

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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("concat compositor"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32],
        };

        // ONE / ONE_MINUS_SRC_ALPHA over premultiplied shader output is
        // source-over. Alpha accumulates the same way; the readback forces the
        // final frame opaque regardless.
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("concat compositor"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(vertex_layout)],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    blend: Some(blend),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
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

        Some(Self {
            device,
            queue,
            pipeline,
            bind_layout,
            sampler,
            vertices,
            vertex_capacity: 6 * 8,
            pool: HashMap::new(),
            used: HashMap::new(),
            target: None,
            idle: HashMap::new(),
            dead: false,
        })
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

    /// Claims a pooled texture of the layer's size, uploading its pixels.
    fn upload(&mut self, frame: &Frame) -> usize {
        let key = (frame.width(), frame.height());
        let used = self.used.entry(key).or_insert(0);
        let pool = self.pool.entry(key).or_default();

        if *used == pool.len() {
            let texture = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("concat layer"),
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
            });
        }

        let index = *used;
        *used += 1;

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &pool[index].texture,
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

    /// The six vertices of one layer's quad, transformed the same way the CPU
    /// path's inverse map is derived: scale, then clockwise rotation, about
    /// the layer's centre, then translation.
    fn quad(layer: &Layer<'_>, out_width: u32, out_height: u32) -> [Vertex; 6] {
        let placement = layer.placement;
        let width = layer.frame.width() as f32;
        let height = layer.frame.height() as f32;

        let centre_x = layer.x as f32 + width / 2.0 + placement.translate_x;
        let centre_y = layer.y as f32 + height / 2.0 + placement.translate_y;
        let (sin, cos) = placement.rotation.sin_cos();
        let scale = placement.scale.max(1e-6);

        let corner = |sx: f32, sy: f32, u: f32, v: f32| {
            // Source-space offset from centre, scaled, then rotated clockwise
            // in y-down coordinates - the forward form of the CPU inverse map.
            let dx = sx * width / 2.0 * scale;
            let dy = sy * height / 2.0 * scale;
            let px = centre_x + dx * cos - dy * sin;
            let py = centre_y + dx * sin + dy * cos;
            Vertex {
                position: [
                    px / out_width as f32 * 2.0 - 1.0,
                    1.0 - py / out_height as f32 * 2.0,
                ],
                uv: [u, v],
                opacity: layer.opacity.clamp(0.0, 1.0),
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
    fn read_back(&mut self) -> Option<Frame> {
        let target = self.target.as_ref().expect("composite rendered first");
        let (width, height, padded_row) = (target.width, target.height, target.padded_row);

        let slice = target.staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());

        let mut frame = Frame::transparent(width, height);
        {
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
    fn composite(&mut self, width: u32, height: u32, layers: &[Layer<'_>]) -> Frame {
        // A dead device never comes back for this instance; the CPU
        // reference is the same pixels, slower - never a mid-export panic.
        if self.dead {
            return CpuCompositor.composite(width, height, layers);
        }

        self.used.values_mut().for_each(|used| *used = 0);

        // Upload every visible layer and build its quad.
        let mut draws: Vec<(u32, u32, usize)> = Vec::with_capacity(layers.len());
        let mut vertices: Vec<Vertex> = Vec::with_capacity(layers.len() * 6);
        for layer in layers {
            if layer.opacity <= 0.0 {
                continue;
            }
            let index = self.upload(layer.frame);
            draws.push((layer.frame.width(), layer.frame.height(), index));
            vertices.extend_from_slice(&Self::quad(layer, width, height));
        }

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
                .write_buffer(&self.vertices, 0, as_bytes(&vertices));
        }

        self.target(width, height);
        let target = self.target.as_ref().expect("just ensured");
        let view = target
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("concat composite"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("concat composite"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            for (draw, (layer_width, layer_height, pooled)) in draws.iter().enumerate() {
                let bind_group = &self.pool[&(*layer_width, *layer_height)][*pooled].bind_group;
                pass.set_bind_group(0, bind_group, &[]);
                let first = (draw * 6) as u32;
                pass.draw(first..first + 6, 0..1);
            }
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

        // Retire texture sizes the timeline has moved past. 300 unclaimed
        // composites (ten seconds of 30fps export) says a size is gone for
        // good, not just between two clips of it.
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

        match self.read_back() {
            Some(frame) => frame,
            None => {
                self.dead = true;
                CpuCompositor.composite(width, height, layers)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CpuCompositor;
    use crate::compositor::Placement;

    fn gpu() -> Option<WgpuCompositor> {
        let compositor = WgpuCompositor::new();
        if compositor.is_none() {
            eprintln!("no usable GPU adapter; skipping");
        }
        compositor
    }

    fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
        let mut frame = Frame::transparent(width, height);
        frame.fill(rgba);
        frame
    }

    /// Both backends composite the same layers; every pixel must agree within
    /// `tolerance` per channel. The CPU path is the reference.
    fn assert_matches_cpu(width: u32, height: u32, layers: &[Layer<'_>], tolerance: i32) {
        let Some(mut gpu) = gpu() else { return };
        let expected = CpuCompositor.composite(width, height, layers);
        let actual = gpu.composite(width, height, layers);

        for y in 0..height {
            for x in 0..width {
                let want = expected.pixel(x, y).expect("in bounds");
                let got = actual.pixel(x, y).expect("in bounds");
                for channel in 0..4 {
                    let difference = (i32::from(want[channel]) - i32::from(got[channel])).abs();
                    assert!(
                        difference <= tolerance,
                        "({x},{y}) channel {channel}: cpu {want:?} vs gpu {got:?}",
                    );
                }
            }
        }
    }

    #[test]
    fn empty_output_is_opaque_black() {
        let Some(mut gpu) = gpu() else { return };
        let frame = gpu.composite(4, 4, &[]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
        assert_eq!(frame.pixel(3, 3), Some([0, 0, 0, 255]));
    }

    #[test]
    fn plain_layers_match_the_cpu_reference() {
        let red = solid(4, 4, [255, 0, 0, 255]);
        let blue = solid(2, 2, [0, 0, 255, 255]);
        let layers = [Layer::new(&red), Layer::new(&blue).at(1, 1)];
        assert_matches_cpu(4, 4, &layers, 1);
    }

    #[test]
    fn opacity_blending_matches_the_cpu_reference() {
        let white = solid(4, 4, [255, 255, 255, 255]);
        let half_alpha = solid(4, 4, [40, 200, 90, 128]);
        let layers = [
            Layer::new(&white).with_opacity(0.5),
            Layer::new(&half_alpha).with_opacity(0.7),
        ];
        assert_matches_cpu(4, 4, &layers, 2);
    }

    #[test]
    fn offset_layers_clip_like_the_cpu_reference() {
        let red = solid(4, 4, [255, 0, 0, 255]);
        let layers = [Layer::new(&red).at(-2, 3)];
        assert_matches_cpu(6, 6, &layers, 1);
    }

    #[test]
    fn scaling_matches_the_cpu_reference_away_from_edges() {
        let Some(mut gpu) = gpu() else { return };
        let red = solid(4, 4, [255, 0, 0, 255]);
        let placement = Placement {
            scale: 2.0,
            ..Placement::IDENTITY
        };
        let layers = [Layer::new(&red).at(2, 2).with_placement(placement)];

        // Interior pixels are unambiguous; the half-pixel border where the two
        // backends rasterise differently is not asserted.
        let frame = gpu.composite(8, 8, &layers);
        for (x, y) in [(1, 1), (4, 4), (6, 6)] {
            assert_eq!(frame.pixel(x, y), Some([255, 0, 0, 255]), "at ({x},{y})");
        }
        assert_eq!(
            CpuCompositor.composite(8, 8, &layers).pixel(4, 4),
            Some([255, 0, 0, 255])
        );
    }

    #[test]
    fn a_half_turn_swaps_the_ends() {
        let Some(mut gpu) = gpu() else { return };
        let mut strip = Frame::transparent(2, 1);
        strip.set_pixel(0, 0, [255, 0, 0, 255]);
        strip.set_pixel(1, 0, [0, 0, 255, 255]);
        let placement = Placement {
            rotation: std::f32::consts::PI,
            ..Placement::IDENTITY
        };
        let frame = gpu.composite(2, 1, &[Layer::new(&strip).with_placement(placement)]);
        assert_eq!(frame.pixel(0, 0), Some([0, 0, 255, 255]));
        assert_eq!(frame.pixel(1, 0), Some([255, 0, 0, 255]));
    }

    #[test]
    fn output_size_changes_are_handled() {
        let Some(mut gpu) = gpu() else { return };
        let red = solid(2, 2, [255, 0, 0, 255]);
        // 3 wide: an unpadded row is 12 bytes, so this exercises row padding.
        let small = gpu.composite(3, 2, &[Layer::new(&red)]);
        assert_eq!(small.pixel(1, 1), Some([255, 0, 0, 255]));
        let large = gpu.composite(16, 8, &[Layer::new(&red).at(14, 6)]);
        assert_eq!(large.pixel(15, 7), Some([255, 0, 0, 255]));
        assert_eq!(large.pixel(0, 0), Some([0, 0, 0, 255]));
    }
}
