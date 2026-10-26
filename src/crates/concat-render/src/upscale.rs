// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Enhance's network on the GPU, in compute shaders of our own.
//!
//! The network is a stack of 3x3 convolutions with a PReLU after each but
//! the last, then a pixel shuffle with the input, enlarged, added (see
//! `concat_vision::compact`). Run by ONNX Runtime it ran on the CPU almost
//! everywhere - Windows' bundled runtime has no GPU provider at all, and
//! CoreML hands back to the CPU what it will not take - at about 27
//! seconds a 1080p frame: 1.2 million multiply-adds a pixel is a CPU's
//! afternoon and a GPU's second. Here each layer is one dispatch on a
//! device of the upscaler's own, so a long job never holds the window's.
//!
//! Activations live in storage buffers, four channels to a `vec4` and the
//! channels of a pixel together: `[(y * width + x) * groups + group]`.
//! Each invocation works out sixteen output channels of a 2x2 block of
//! pixels, a `mat4x4` of weights to each input `vec4` under each of the
//! nine taps; a padded edge reads as zero, as the model's padding is. The last pass shuffles
//! the final layer's channels into the larger picture and adds the input
//! to it, writing three planes, which is the ONNX model's answer exactly:
//! the tiling and pasting around it do not know which ran.

use std::sync::mpsc;

use wgpu::util::DeviceExt;

/// One 3x3 convolution and its PReLU, as the network file holds them; see
/// `concat_vision::compact::Conv`.
#[derive(Clone, Debug)]
pub struct ConvLayer {
    /// Channels read.
    pub in_channels: usize,
    /// Channels written.
    pub out_channels: usize,
    /// `[out][in][3][3]`.
    pub weight: Vec<f32>,
    /// One per output channel.
    pub bias: Vec<f32>,
    /// The PReLU's slope per output channel, or `None` for none.
    pub prelu: Option<Vec<f32>>,
}

const SHADER_COMMON: &str = r#"
struct Dims {
    width: u32,
    height: u32,
    in_groups: u32,
    out_groups: u32,
    prelu: u32,
    scale: u32,
}

@group(0) @binding(0) var<uniform> dims: Dims;
@group(0) @binding(1) var<storage, read> src: array<vec4<T>>;
@group(0) @binding(2) var<storage, read_write> dst: array<vec4<T>>;
@group(0) @binding(3) var<storage, read> weights: array<mat4x4<T>>;
@group(0) @binding(4) var<storage, read> bias: array<vec4<f32>>;
@group(0) @binding(5) var<storage, read> slope: array<vec4<f32>>;

// `conv` is generated (see `conv_shader`): every tap and channel group
// unrolled so the accumulators and the neighbourhood are plain registers.

@group(0) @binding(6) var<storage, read> picture: array<vec4<T>>;
@group(0) @binding(7) var<storage, read_write> planes: array<f32>;

// The last layer's channels, `3 * scale * scale` of them, laid out as the
// picture `scale` times larger - channel `c * scale * scale + dy * scale + dx`
// is colour `c` at `(x * scale + dx, y * scale + dy)`, PyTorch's order - with
// the input added, enlarged by nearest neighbour.
@compute @workgroup_size(8, 8, 1)
fn shuffle(@builtin(global_invocation_id) id: vec3<u32>) {
    let s = dims.scale;
    let big_w = dims.width * s;
    let big_h = dims.height * s;
    if (id.x >= big_w || id.y >= big_h) {
        return;
    }
    let x = id.x / s;
    let y = id.y / s;
    let dx = id.x % s;
    let dy = id.y % s;
    let at = y * dims.width + x;
    let input = picture[at];
    for (var c = 0u; c < 3u; c++) {
        let channel = c * s * s + dy * s + dx;
        let value = f32(src[at * dims.in_groups + channel / 4u][channel % 4u]);
        planes[c * big_w * big_h + id.y * big_w + id.x] = value + f32(input[c]);
    }
}
"#;

/// The whole module, in half floats where the device has them: the
/// activations and weights stored at sixteen bits, which halves both the
/// memory read and the registers a block of pixels takes, and the sums
/// kept at thirty-two so 576 of them a channel do not drift.
fn shader(half: bool) -> String {
    let (head, ty) = if half { ("enable f16;\n", "f16") } else { ("", "f32") };
    let body = SHADER_COMMON.to_owned() + &conv_shader();
    format!("{head}alias T = {ty};\n{body}")
}

/// The convolution kernel. Each invocation works out a 2x2 block of pixels
/// and sixteen output channels (four groups) of each, so every weight
/// matrix read serves four pixels and every input read up to sixteen
/// channels under up to four taps. Written out with every tap and group
/// unrolled: a local array indexed by a loop variable cannot stay in
/// registers, and the first version, which did that, spilled to memory
/// and ran at a third of the GPU.
fn conv_shader() -> String {
    conv_shader_for(BLOCK_W, BLOCK_H)
}

/// The block of pixels one invocation works out, across and down.
const BLOCK_W: u32 = 2;
const BLOCK_H: u32 = 2;

fn conv_shader_for(bw: u32, bh: u32) -> String {
    // The workgroup's patch of pixels, and the patch with its one-pixel
    // rim that its taps read.
    let (pw, ph) = (8 * bw, 8 * bh);
    let (tw, th) = (pw + 2, ph + 2);
    let tile = tw * th;
    let mut code = format!(
        "var<workgroup> tile: array<vec4<T>, {tile}>;\n\
         var<workgroup> taps: array<mat4x4<T>, 36>;\n\n\
         fn near(sx: i32, sy: i32, i: u32) -> vec4<T> {{\n\
         \x20   if (sx < 0 || sy < 0 || sx >= i32(dims.width) || sy >= i32(dims.height)) {{\n\
         \x20       return vec4<T>(0.0);\n\
         \x20   }}\n\
         \x20   return src[(u32(sy) * dims.width + u32(sx)) * dims.in_groups + i];\n\
         }}\n\n\
         @compute @workgroup_size(8, 8, 1)\n\
         fn conv(@builtin(global_invocation_id) id: vec3<u32>,\n\
         \x20       @builtin(local_invocation_id) local: vec3<u32>,\n\
         \x20       @builtin(local_invocation_index) lid: u32,\n\
         \x20       @builtin(workgroup_id) group: vec3<u32>) {{\n\
         \x20   let x0 = id.x * {bw}u;\n\
         \x20   let y0 = id.y * {bh}u;\n\
         \x20   let o0 = group.z * 4u;\n\
         \x20   let px = i32(group.x * {pw}u) - 1;\n\
         \x20   let py = i32(group.y * {ph}u) - 1;\n\
         \x20   let base = local.y * {bh}u * {tw}u + local.x * {bw}u;\n"
    );
    let pixels: Vec<(u32, u32)> = (0..bh).flat_map(|dy| (0..bw).map(move |dx| (dy, dx))).collect();
    for g in 0..4 {
        code += &format!("    let b{g} = bias[o0 + {g}u];\n");
        for p in 0..pixels.len() {
            code += &format!("    var a{p}_{g} = b{g};\n");
        }
    }
    // Every invocation stays to the barriers, past the edge or not: the
    // loads are shared, and only the writes are held to the picture.
    code += &format!(
        "    for (var i = 0u; i < dims.in_groups; i++) {{\n\
         \x20       workgroupBarrier();\n\
         \x20       for (var k = lid; k < {tile}u; k += 64u) {{\n\
         \x20           tile[k] = near(px + i32(k % {tw}u), py + i32(k / {tw}u), i);\n\
         \x20       }}\n\
         \x20       if (lid < 36u) {{\n\
         \x20           taps[lid] = weights[((o0 + lid / 9u) * 9u + lid % 9u) * dims.in_groups + i];\n\
         \x20       }}\n\
         \x20       workgroupBarrier();\n"
    );
    for ny in 0..bh + 2 {
        for nx in 0..bw + 2 {
            code += &format!("        let n{ny}_{nx} = tile[base + {}u];\n", ny * tw + nx);
        }
    }
    for g in 0..4u32 {
        // A group's nine taps summed in the stored type, then carried into
        // the f32 sum: one conversion a group rather than one a product,
        // and only 36 terms ever added at sixteen bits. One tap's matrix is
        // read and spent on every pixel of the block before the next is
        // read, so a single matrix is ever held in registers: holding all
        // nine at once, with the neighbourhood and the sums, was past what
        // a thread has, and spilled.
        for p in 0..pixels.len() {
            code += &format!("        var s{p}_{g} = vec4<T>(0.0);\n");
        }
        for tap in 0..9u32 {
            code += &format!("        {{\n            let m = taps[{}u];\n", g * 9 + tap);
            for (p, (dy, dx)) in pixels.iter().enumerate() {
                code += &format!(
                    "            s{p}_{g} += m * n{}_{};\n",
                    tap / 3 + dy,
                    tap % 3 + dx
                );
            }
            code += "        }\n";
        }
        for p in 0..pixels.len() {
            code += &format!("        a{p}_{g} += vec4<f32>(s{p}_{g});\n");
        }
    }
    code += "    }\n";
    for (p, (dy, dx)) in pixels.iter().enumerate() {
        code += &format!(
            "    if (x0 + {dx}u < dims.width && y0 + {dy}u < dims.height) {{\n\
             \x20       let at = ((y0 + {dy}u) * dims.width + x0 + {dx}u) * dims.out_groups + o0;\n"
        );
        for g in 0..4 {
            code += &format!(
                "        var v{g} = a{p}_{g};\n\
                 \x20       if (dims.prelu == 1u) {{ v{g} = select(v{g}, v{g} * slope[o0 + {g}u], v{g} < vec4<f32>(0.0)); }}\n\
                 \x20       dst[at + {g}u] = vec4<T>(v{g});\n"
            );
        }
        code += "    }\n";
    }
    code += "}\n";
    code
}

/// A layer as the GPU holds it.
struct GpuLayer {
    out_groups: u32,
    bind_group: wgpu::BindGroup,
}

/// The network on a device of its own, for windows of one size.
pub struct Upscaler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    conv: wgpu::ComputePipeline,
    shuffle: wgpu::ComputePipeline,
    layers: Vec<GpuLayer>,
    shuffle_group: wgpu::BindGroup,
    picture: wgpu::Buffer,
    planes: wgpu::Buffer,
    staging: wgpu::Buffer,
    side: u32,
    scale: u32,
    /// Activations and weights are stored as f16.
    half: bool,
}

/// `values` as the shader stores them: little-endian f32, or f16.
fn stored(values: &[f32], half: bool) -> Vec<u8> {
    if half {
        values
            .iter()
            .flat_map(|v| half::f16::from_f32(*v).to_le_bytes())
            .collect()
    } else {
        values.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
}

/// Four channels to a group.
fn groups(channels: usize) -> usize {
    channels.div_ceil(4)
}

impl Upscaler {
    /// The network `layers`, enlarging `scale` times, for square windows
    /// `side` pixels across, on the first GPU that opens - `None` where
    /// there is none, and the caller runs the model the slow way.
    pub fn new(layers: &[ConvLayer], scale: u32, side: u32) -> Option<Upscaler> {
        let instance = wgpu::Instance::default();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        if adapter.get_info().device_type == wgpu::DeviceType::Cpu {
            return None;
        }
        let half = adapter.features().contains(wgpu::Features::SHADER_F16);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("concat upscaler"),
            required_features: if half {
                wgpu::Features::SHADER_F16
            } else {
                wgpu::Features::empty()
            },
            required_limits: wgpu::Limits::default()
                .or_worse_values_from(&adapter.limits())
                .using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .ok()?;
        Self::on(device, queue, layers, scale, side, half)
    }

    fn on(
        device: wgpu::Device,
        queue: wgpu::Queue,
        layers: &[ConvLayer],
        scale: u32,
        side: u32,
        half: bool,
    ) -> Option<Upscaler> {
        // The kernel works out four channel groups at a time, so a layer's
        // outputs are padded up to a multiple of sixteen with zero weights,
        // and the next layer reads the padding as inputs it weighs by zero.
        let padded_groups = |channels: usize| groups(channels).div_ceil(4) * 4;
        let widest = layers.iter().map(|l| padded_groups(l.out_channels)).max()?;
        let pixels = (side * side) as u64;
        // A vec4 of the stored type: sixteen bytes of f32, eight of f16.
        let vec4 = if half { 8u64 } else { 16u64 };
        let activation = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: pixels * widest as u64 * vec4,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let ping = activation("upscaler ping");
        let pong = activation("upscaler pong");
        let picture = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscaler picture"),
            size: pixels * vec4,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let planes_size = 3 * pixels * u64::from(scale * scale) * 4;
        let planes = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscaler planes"),
            size: planes_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upscaler staging"),
            size: planes_size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let descriptor = wgpu::ShaderModuleDescriptor {
            label: Some("upscaler"),
            source: wgpu::ShaderSource::Wgsl(shader(half).into()),
        };
        let module = device.create_shader_module(descriptor);
        let entry = |binding: u32, kind: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: kind,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let read = wgpu::BufferBindingType::Storage { read_only: true };
        let write = wgpu::BufferBindingType::Storage { read_only: false };
        let conv_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscaler conv"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, read),
                entry(2, write),
                entry(3, read),
                entry(4, read),
                entry(5, read),
            ],
        });
        let shuffle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("upscaler shuffle"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, read),
                entry(6, read),
                entry(7, write),
            ],
        });
        let pipeline = |layout: &wgpu::BindGroupLayout, entry_point: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry_point),
                layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some(entry_point),
                    bind_group_layouts: &[Some(layout)],
                    immediate_size: 0,
                })),
                module: &module,
                entry_point: Some(entry_point),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let conv = pipeline(&conv_layout, "conv");
        let shuffle = pipeline(&shuffle_layout, "shuffle");

        let dims = |in_groups: usize, out_groups: usize, prelu: bool| {
            let words = [
                side,
                side,
                in_groups as u32,
                out_groups as u32,
                u32::from(prelu),
                scale,
                0,
                0,
            ];
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("upscaler dims"),
                contents: &words.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<u8>>(),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };
        let floats = |label, values: &[f32]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>(),
                usage: wgpu::BufferUsages::STORAGE,
            })
        };

        let mut gpu_layers = Vec::new();
        for (index, layer) in layers.iter().enumerate() {
            let in_groups = if index == 0 {
                groups(layer.in_channels)
            } else {
                padded_groups(layers[index - 1].out_channels)
            };
            let out_groups = padded_groups(layer.out_channels);
            // Each mat4x4 is four input channels into four outputs, kept
            // column by column: column `j` is input `4i + j`'s weights to
            // outputs `4o..4o + 4`, so `matrix * input` is the four sums.
            let mut packed = vec![0.0f32; out_groups * 9 * in_groups * 16];
            for o in 0..layer.out_channels {
                for i in 0..layer.in_channels {
                    for tap in 0..9 {
                        let w = layer.weight[(o * layer.in_channels + i) * 9 + tap];
                        let matrix = ((o / 4) * 9 + tap) * in_groups + i / 4;
                        packed[matrix * 16 + (i % 4) * 4 + o % 4] = w;
                    }
                }
            }
            let padded = |values: &[f32]| {
                let mut out = values.to_vec();
                out.resize(out_groups * 4, 0.0);
                out
            };
            let slopes = padded(layer.prelu.as_deref().unwrap_or(&[]));
            let (src, dst) = if index % 2 == 0 {
                (if index == 0 { &picture } else { &pong }, &ping)
            } else {
                (&ping, &pong)
            };
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("upscaler layer"),
                layout: &conv_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: dims(in_groups, out_groups, layer.prelu.is_some())
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: src.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: dst.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: device
                            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                label: Some("upscaler weights"),
                                contents: &stored(&packed, half),
                                usage: wgpu::BufferUsages::STORAGE,
                            })
                            .as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: floats("upscaler bias", &padded(&layer.bias)).as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: floats("upscaler slope", &slopes).as_entire_binding(),
                    },
                ],
            });
            gpu_layers.push(GpuLayer {
                out_groups: out_groups as u32,
                bind_group,
            });
        }
        // The last layer wrote ping when there is an odd number of layers.
        let last = if layers.len() % 2 == 1 { &ping } else { &pong };
        let shuffle_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("upscaler shuffle"),
            layout: &shuffle_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: dims(padded_groups(layers.last()?.out_channels), 0, false)
                        .as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: last.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: picture.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: planes.as_entire_binding(),
                },
            ],
        });
        Some(Upscaler {
            device,
            queue,
            conv,
            shuffle,
            layers: gpu_layers,
            shuffle_group,
            picture,
            planes,
            half,
            staging,
            side,
            scale,
        })
    }

    /// Whether it works in half floats; see [`shader`].
    pub fn half(&self) -> bool {
        self.half
    }

    /// The side of the square windows this runs.
    pub fn side(&self) -> u32 {
        self.side
    }

    /// The network over one window: three planes of `side` by `side`,
    /// values `0..=1`, in; three planes `scale` times the side out - the
    /// ONNX model's input and answer, value for value.
    pub fn run(&self, planes: &[f32]) -> Result<Vec<f32>, String> {
        let side = self.side as usize;
        let area = side * side;
        if planes.len() != 3 * area {
            return Err(format!(
                "upscaler: {} values for a {side}-pixel window",
                planes.len()
            ));
        }
        let mut packed = vec![0.0f32; area * 4];
        for at in 0..area {
            for c in 0..3 {
                packed[at * 4 + c] = planes[c * area + at];
            }
        }
        self.queue.write_buffer(
            &self.picture,
            0,
            &stored(&packed, self.half),
        );
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("upscale") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("upscale"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.conv);
            let across = self.side.div_ceil(8 * BLOCK_W);
            let down = self.side.div_ceil(8 * BLOCK_H);
            for layer in &self.layers {
                pass.set_bind_group(0, &layer.bind_group, &[]);
                pass.dispatch_workgroups(across, down, layer.out_groups.div_ceil(4));
            }
            pass.set_pipeline(&self.shuffle);
            pass.set_bind_group(0, &self.shuffle_group, &[]);
            let big = (self.side * self.scale).div_ceil(8);
            pass.dispatch_workgroups(big, big, 1);
        }
        encoder.copy_buffer_to_buffer(&self.planes, 0, &self.staging, 0, self.planes.size());
        self.queue.submit([encoder.finish()]);

        let slice = self.staging.slice(..);
        let (sent, mapped) = mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sent.send(result);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("upscaler: the device did not finish: {error}"))?;
        mapped
            .recv()
            .map_err(|_| "upscaler: the device never answered".to_owned())?
            .map_err(|error| format!("upscaler: the answer would not map: {error}"))?;
        let answer = {
            let data = slice.get_mapped_range().map_err(|e| format!("upscaler: {e}"))?;
            data.chunks_exact(4)
                .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect()
        };
        self.staging.unmap();
        Ok(answer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_vision::compact::{CompactNet, Conv};

    /// A small network of made-up weights, run on the GPU and on the CPU
    /// reference: the two agree to rounding, edges and shuffle included.
    #[test]
    fn the_gpu_answers_as_the_reference_does() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % 2001) as f32 / 1000.0 - 1.0
        };
        let mut conv = |inputs: usize, outputs: usize, prelu: bool| Conv {
            in_channels: inputs,
            out_channels: outputs,
            weight: (0..outputs * inputs * 9).map(|_| next() * 0.3).collect(),
            bias: (0..outputs).map(|_| next() * 0.1).collect(),
            prelu: prelu.then(|| (0..outputs).map(|_| next() * 0.5).collect()),
        };
        let net = CompactNet {
            layers: vec![conv(3, 8, true), conv(8, 8, true), conv(8, 12, false)],
            scale: 2,
        };
        let side = 20usize;
        let mut seed = 7u64;
        let planes: Vec<f32> = (0..3 * side * side)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 40) as f32 / (1u64 << 24) as f32
            })
            .collect();
        let layers: Vec<ConvLayer> = net
            .layers
            .iter()
            .map(|l| ConvLayer {
                in_channels: l.in_channels,
                out_channels: l.out_channels,
                weight: l.weight.clone(),
                bias: l.bias.clone(),
                prelu: l.prelu.clone(),
            })
            .collect();
        let Some(gpu) = Upscaler::new(&layers, 2, side as u32) else {
            eprintln!("no GPU here; skipped");
            return;
        };
        let got = gpu.run(&planes).expect("runs");
        let want = net.run_reference(&planes, side);
        assert_eq!(got.len(), want.len());
        let worst = got
            .iter()
            .zip(&want)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        // Half floats keep about three decimal digits.
        let allowed = if gpu.half() { 2e-2 } else { 1e-4 };
        assert!(worst < allowed, "the GPU is off by {worst}");
    }

    /// The real model, when this machine has fetched it, timed over a few
    /// windows of the size Enhance cuts a frame into, and the 1080p frame
    /// that comes to. `cargo test -p concat-render --release upscale_speed
    /// -- --ignored --nocapture`.
    #[test]
    #[ignore = "a benchmark, and it needs the fetched model"]
    fn upscale_speed() {
        let Some(home) = std::env::var_os("HOME") else { return };
        let support = std::path::Path::new(&home).join("Library/Application Support");
        let Some(bytes) = ["org.standalones.concat", "app.concat.editor"]
            .iter()
            .map(|id| support.join(id).join("cutout-models/realesr-general-x4v3.onnx"))
            .find_map(|file| std::fs::read(file).ok())
        else {
            eprintln!("no Enhance model on this machine");
            return;
        };
        let net = CompactNet::read(&bytes).expect("reads");
        let layers: Vec<ConvLayer> = net
            .layers
            .iter()
            .map(|l| ConvLayer {
                in_channels: l.in_channels,
                out_channels: l.out_channels,
                weight: l.weight.clone(),
                bias: l.bias.clone(),
                prelu: l.prelu.clone(),
            })
            .collect();
        // The real model on a small window against the CPU reference, in
        // eight-bit levels, which is what the file is written in.
        let small = 40u32;
        let gpu = Upscaler::new(&layers, net.scale as u32, small).expect("a GPU");
        let mut seed = 11u64;
        let planes: Vec<f32> = (0..3 * (small * small) as usize)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                (seed >> 40) as f32 / (1u64 << 24) as f32
            })
            .collect();
        let got = gpu.run(&planes).expect("runs");
        let want = net.run_reference(&planes, small as usize);
        let worst = got
            .iter()
            .zip(&want)
            .map(|(a, b)| (a.clamp(0.0, 1.0) - b.clamp(0.0, 1.0)).abs() * 255.0)
            .fold(0.0f32, f32::max);
        eprintln!("half floats: {}, worst error {worst:.2} levels of 255", gpu.half());
        for side in [288u32, 544] {
            let gpu = Upscaler::new(&layers, net.scale as u32, side).expect("a GPU");
            let planes = vec![0.5f32; 3 * (side * side) as usize];
            gpu.run(&planes).expect("warms up");
            let runs = 5;
            let started = std::time::Instant::now();
            for _ in 0..runs {
                gpu.run(&planes).expect("runs");
            }
            let each = started.elapsed().as_secs_f64() / runs as f64;
            let core = f64::from(side - 32);
            let windows = (1920.0 / core).ceil() * (1080.0 / core).ceil();
            eprintln!(
                "window {side}: {:.1} ms each, a 1080p frame in {:.0} windows = {:.2} s",
                each * 1000.0,
                windows,
                each * windows
            );
        }
    }
}
