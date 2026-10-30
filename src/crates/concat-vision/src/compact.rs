// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The Enhance model's weights, read out of its ONNX file for a GPU of our
//! own to run.
//!
//! Real-ESRGAN's compact network (SRVGGNetCompact) is a plain stack: a 3x3
//! convolution from the picture's three channels to 64, a PReLU, thirty-two
//! more 3x3 convolutions of 64 to 64 each with its PReLU, one last 3x3 to
//! `3 * scale * scale` channels, those channels rearranged into a picture
//! `scale` times larger (PyTorch's pixel shuffle, ONNX's `DepthToSpace` in
//! CRD order), and the input, enlarged by nearest neighbour, added to it.
//! Nothing in it needs a runtime: [`CompactNet::read`] takes the layers'
//! numbers out of the file and checks the graph is that shape, so a file
//! that is anything else is refused rather than run wrong.
//!
//! ONNX is protocol buffers. The handful of fields read here are decoded by
//! hand (see [`fields`]) rather than through a protobuf crate: a model
//! file is the only one this reads, and only its graph's nodes and
//! initializers.

use std::collections::HashMap;

/// One 3x3 convolution, padded by one, stride one, and the PReLU after it
/// when there is one.
#[derive(Clone, Debug, PartialEq)]
pub struct Conv {
    /// Channels read.
    pub in_channels: usize,
    /// Channels written.
    pub out_channels: usize,
    /// `[out][in][3][3]`, row-major, as ONNX and PyTorch keep it.
    pub weight: Vec<f32>,
    /// One per output channel.
    pub bias: Vec<f32>,
    /// One slope per output channel, or `None` for the last layer, whose
    /// answer goes to the pixel shuffle as it is.
    pub prelu: Option<Vec<f32>>,
}

/// The whole network: its convolutions in order, and how much larger the
/// picture comes out.
#[derive(Clone, Debug, PartialEq)]
pub struct CompactNet {
    /// The convolutions, input first.
    pub layers: Vec<Conv>,
    /// How many times larger the picture comes out.
    pub scale: usize,
}

impl CompactNet {
    /// The network in the ONNX file `bytes`.
    pub fn read(bytes: &[u8]) -> Result<CompactNet, String> {
        let graph = fields(bytes)?
            .into_iter()
            .find_map(|(field, value)| (field == 7).then_some(value))
            .and_then(Value::bytes)
            .ok_or("enhance: the model file has no graph")?;
        let mut tensors: HashMap<String, (Vec<usize>, Vec<f32>)> = HashMap::new();
        let mut nodes = Vec::new();
        for (field, value) in fields(graph)? {
            match (field, value.bytes()) {
                (1, Some(node)) => nodes.push(Node::read(node)?),
                (5, Some(tensor)) => {
                    let (name, dims, data) = tensor_of(tensor)?;
                    tensors.insert(name, (dims, data));
                }
                _ => {}
            }
        }

        let mut layers = Vec::new();
        let mut scale = None;
        for (at, node) in nodes.iter().enumerate() {
            match node.op.as_str() {
                "Conv" => {
                    if node.ints("kernel_shape") != Some(&[3, 3][..])
                        || node.ints("pads").is_some_and(|pads| pads != [1, 1, 1, 1])
                        || node.ints("strides").is_some_and(|s| s != [1, 1])
                        || node.ints("group").is_some_and(|g| g != [1])
                    {
                        return Err(format!(
                            "enhance: layer {} is not a plain 3x3 convolution",
                            layers.len()
                        ));
                    }
                    let tensor = |index: usize| {
                        node.inputs
                            .get(index)
                            .and_then(|name| tensors.get(name))
                            .ok_or_else(|| format!("enhance: layer {} has no weights", layers.len()))
                    };
                    let (dims, weight) = tensor(1)?;
                    let (_, bias) = tensor(2)?;
                    let [out_channels, in_channels, 3, 3] = dims[..] else {
                        return Err(format!("enhance: layer {} is shaped {dims:?}", layers.len()));
                    };
                    if bias.len() != out_channels {
                        return Err(format!("enhance: layer {}'s bias is the wrong length", layers.len()));
                    }
                    // The PReLU after it, when the next node is one.
                    let prelu = match nodes.get(at + 1) {
                        Some(next) if next.op == "PRelu" => {
                            let (_, slopes) = next
                                .inputs
                                .get(1)
                                .and_then(|name| tensors.get(name))
                                .ok_or("enhance: a PReLU has no slopes")?;
                            if slopes.len() != out_channels {
                                return Err("enhance: a PReLU's slopes are the wrong length".into());
                            }
                            Some(slopes.clone())
                        }
                        _ => None,
                    };
                    layers.push(Conv {
                        in_channels,
                        out_channels,
                        weight: weight.clone(),
                        bias: bias.clone(),
                        prelu,
                    });
                }
                "DepthToSpace" => {
                    if node.string("mode").is_some_and(|mode| mode != "CRD") {
                        return Err("enhance: the pixel shuffle is not in CRD order".into());
                    }
                    scale = node.ints("blocksize").and_then(|b| b.first().copied());
                }
                "Resize" => {
                    if node.string("mode").is_some_and(|mode| mode != "nearest") {
                        return Err("enhance: the input is not enlarged by nearest neighbour".into());
                    }
                }
                "PRelu" | "Constant" | "Add" => {}
                other => return Err(format!("enhance: the model has a {other} this cannot run")),
            }
        }
        let scale = scale.ok_or("enhance: the model has no pixel shuffle")? as usize;
        // The chain must join up: three channels in, 64s through, and the
        // last layer's channels exactly the shuffle's.
        let chained = layers.first().is_some_and(|first| first.in_channels == 3)
            && layers.windows(2).all(|pair| pair[0].out_channels == pair[1].in_channels)
            && layers.last().is_some_and(|last| {
                last.out_channels == 3 * scale * scale && last.prelu.is_none()
            })
            && layers[..layers.len() - 1].iter().all(|layer| layer.prelu.is_some());
        if !chained {
            return Err("enhance: the model's layers do not chain as a compact network".into());
        }
        Ok(CompactNet { layers, scale })
    }

    /// The network run on the CPU, plainly, over `planes` - three planes of
    /// `side` by `side`, values `0..=1` - into three planes `scale` times
    /// the side: the reference the GPU's answer is held to. Far too slow
    /// for a frame; a small square is what a test gives it.
    pub fn run_reference(&self, planes: &[f32], side: usize) -> Vec<f32> {
        let area = side * side;
        let mut x = planes.to_vec();
        for layer in &self.layers {
            let mut y = vec![0.0f32; layer.out_channels * area];
            for o in 0..layer.out_channels {
                for py in 0..side {
                    for px in 0..side {
                        let mut acc = layer.bias[o];
                        for i in 0..layer.in_channels {
                            for ky in 0..3 {
                                let sy = py as isize + ky as isize - 1;
                                if sy < 0 || sy >= side as isize {
                                    continue;
                                }
                                for kx in 0..3 {
                                    let sx = px as isize + kx as isize - 1;
                                    if sx < 0 || sx >= side as isize {
                                        continue;
                                    }
                                    let w = layer.weight[((o * layer.in_channels + i) * 3 + ky) * 3 + kx];
                                    acc += w * x[i * area + sy as usize * side + sx as usize];
                                }
                            }
                        }
                        if let Some(slopes) = &layer.prelu
                            && acc < 0.0
                        {
                            acc *= slopes[o];
                        }
                        y[o * area + py * side + px] = acc;
                    }
                }
            }
            x = y;
        }
        let s = self.scale;
        let big = side * s;
        let mut out = vec![0.0f32; 3 * big * big];
        for c in 0..3 {
            for oy in 0..big {
                for ox in 0..big {
                    let (y, dy, xx, dx) = (oy / s, oy % s, ox / s, ox % s);
                    let shuffled = x[(c * s * s + dy * s + dx) * area + y * side + xx];
                    out[c * big * big + oy * big + ox] = shuffled + planes[c * area + y * side + xx];
                }
            }
        }
        out
    }
}

/// One node of the graph: what it does, what it reads, and its attributes.
struct Node {
    op: String,
    inputs: Vec<String>,
    attributes: HashMap<String, Attribute>,
}

enum Attribute {
    Ints(Vec<i64>),
    Text(String),
}

impl Node {
    fn read(bytes: &[u8]) -> Result<Node, String> {
        let mut node = Node {
            op: String::new(),
            inputs: Vec::new(),
            attributes: HashMap::new(),
        };
        for (field, value) in fields(bytes)? {
            match (field, value) {
                (1, Value::Bytes(name)) => node.inputs.push(text(name)),
                (4, Value::Bytes(op)) => node.op = text(op),
                (5, Value::Bytes(attribute)) => {
                    let mut name = String::new();
                    let mut ints = Vec::new();
                    let mut string = None;
                    for (field, value) in fields(attribute)? {
                        match (field, value) {
                            (1, Value::Bytes(n)) => name = text(n),
                            // `i`, and `ints` written one at a time.
                            (3 | 8, Value::Varint(v)) => ints.push(v as i64),
                            // `ints` packed.
                            (8, Value::Bytes(packed)) => {
                                let mut at = 0;
                                while at < packed.len() {
                                    let (v, next) = varint(packed, at)?;
                                    ints.push(v as i64);
                                    at = next;
                                }
                            }
                            (4, Value::Bytes(s)) => string = Some(text(s)),
                            _ => {}
                        }
                    }
                    let value = match string {
                        Some(s) => Attribute::Text(s),
                        None => Attribute::Ints(ints),
                    };
                    node.attributes.insert(name, value);
                }
                _ => {}
            }
        }
        Ok(node)
    }

    fn ints(&self, name: &str) -> Option<&[i64]> {
        match self.attributes.get(name)? {
            Attribute::Ints(ints) => Some(ints),
            Attribute::Text(_) => None,
        }
    }

    fn string(&self, name: &str) -> Option<&str> {
        match self.attributes.get(name)? {
            Attribute::Text(s) => Some(s),
            Attribute::Ints(_) => None,
        }
    }
}

/// A float tensor's name, shape and numbers, from a `TensorProto`: kept as
/// `raw_data` little-endian, or as packed `float_data`.
fn tensor_of(bytes: &[u8]) -> Result<(String, Vec<usize>, Vec<f32>), String> {
    let mut name = String::new();
    let mut dims = Vec::new();
    let mut data = Vec::new();
    let mut kind = 1;
    for (field, value) in fields(bytes)? {
        match (field, value) {
            (1, Value::Varint(d)) => dims.push(d as usize),
            (1, Value::Bytes(packed)) => {
                let mut at = 0;
                while at < packed.len() {
                    let (d, next) = varint(packed, at)?;
                    dims.push(d as usize);
                    at = next;
                }
            }
            (2, Value::Varint(k)) => kind = k,
            (8, Value::Bytes(n)) => name = text(n),
            (9, Value::Bytes(raw)) | (4, Value::Bytes(raw)) => data.extend(
                raw.chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            ),
            _ => {}
        }
    }
    if kind != 1 && !data.is_empty() {
        return Err(format!("enhance: {name} is not 32-bit floats"));
    }
    let count: usize = dims.iter().product();
    if !data.is_empty() && data.len() != count.max(1) {
        return Err(format!("enhance: {name} holds {} numbers, not {count}", data.len()));
    }
    Ok((name, dims, data))
}

/// A protobuf field's value: the two wire types a model file's fields use,
/// and the fixed-width ones skipped over.
enum Value<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed,
}

impl<'a> Value<'a> {
    fn bytes(self) -> Option<&'a [u8]> {
        match self {
            Value::Bytes(bytes) => Some(bytes),
            _ => None,
        }
    }
}

/// A protobuf message's fields, in order, as `(number, value)`.
fn fields(bytes: &[u8]) -> Result<Vec<(u64, Value<'_>)>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let (key, next) = varint(bytes, at)?;
        at = next;
        let value = match key & 7 {
            0 => {
                let (v, next) = varint(bytes, at)?;
                at = next;
                Value::Varint(v)
            }
            1 => {
                at += 8;
                Value::Fixed
            }
            2 => {
                let (length, next) = varint(bytes, at)?;
                let end = next
                    .checked_add(length as usize)
                    .filter(|end| *end <= bytes.len())
                    .ok_or("enhance: the model file is cut short")?;
                at = end;
                Value::Bytes(&bytes[next..end])
            }
            5 => {
                at += 4;
                Value::Fixed
            }
            other => return Err(format!("enhance: the model file has wire type {other}")),
        };
        if at > bytes.len() {
            return Err("enhance: the model file is cut short".into());
        }
        out.push((key >> 3, value));
    }
    Ok(out)
}

fn varint(bytes: &[u8], mut at: usize) -> Result<(u64, usize), String> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *bytes.get(at).ok_or("enhance: the model file is cut short")?;
        at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Ok((value, at));
        }
    }
    Err("enhance: the model file has a number too long to be one".into())
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// What the tests of this crate share.
#[cfg(test)]
pub(crate) mod tests_support {
    pub(crate) use super::tests::fetched_model;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real model, when this machine has fetched it: read whole, the
    /// shape Real-ESRGAN's compact network has.
    #[test]
    fn the_fetched_model_reads_as_a_compact_network() {
        let Some(file) = fetched_model() else {
            eprintln!("no Enhance model on this machine; skipped");
            return;
        };
        let net = CompactNet::read(&std::fs::read(file).expect("reads")).expect("parses");
        assert_eq!(net.scale, 4);
        assert_eq!(net.layers.len(), 34);
        assert_eq!(net.layers[0].in_channels, 3);
        assert_eq!(net.layers[33].out_channels, 48);
        let params: usize = net
            .layers
            .iter()
            .map(|l| l.weight.len() + l.bias.len() + l.prelu.as_ref().map_or(0, Vec::len))
            .sum();
        assert_eq!(params, 1_213_296);
    }

    /// Anything but the shape it knows is refused, not run wrong.
    #[test]
    fn a_file_that_is_not_a_model_is_refused() {
        assert!(CompactNet::read(b"not a model").is_err());
        assert!(CompactNet::read(&[]).is_err());
    }

    /// Where the app keeps the model on this machine, if it has it: under
    /// the app's identifier, or the one before it on a machine whose app
    /// has not started since the rename.
    pub(crate) fn fetched_model() -> Option<std::path::PathBuf> {
        let bases: Vec<std::path::PathBuf> = if cfg!(target_os = "macos") {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join("Library/Application Support"))
                .into_iter()
                .collect()
        } else if cfg!(windows) {
            std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).into_iter().collect()
        } else {
            std::env::var_os("HOME")
                .map(|home| std::path::PathBuf::from(home).join(".local/share"))
                .into_iter()
                .collect()
        };
        bases
            .iter()
            .flat_map(|base| {
                ["org.standalones.concat", "app.concat.editor"]
                    .map(|id| base.join(id).join("cutout-models").join("realesr-general-x4v3.onnx"))
            })
            .find(|file| file.is_file())
    }
}
