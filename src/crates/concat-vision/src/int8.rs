// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A quantised graph's matrix products asked to run in eight-bit integers.
//!
//! ONNX Runtime's `MatMulNBits` keeps its weights in four or eight bits,
//! and its `accuracy_level` says what the product is worked out in. A
//! graph that leaves it unset - Chatterbox's export does, on all of its
//! 963 - is worked in `f32`: the weights are widened to floats on every
//! run. A prompt of two hundred tokens spreads that over all of them; a
//! language model writing one token a step pays it whole each step. Level
//! 4 works the product in `int8`, on the integer kernels the CPU has
//! (dot-product instructions on ARM, VNNI and AVX2 on x86). Measured on an
//! M-series Mac, CPU only: a step of Chatterbox's language model went from
//! 100 ms to 8 ms, and a read from 3.1 s a second of speech to 0.6.
//!
//! The level is an attribute on each node, and the runtime has no session
//! option over it, so the graph itself is rewritten: once, beside the one
//! it comes from, under a name of its own. The original stays as it was
//! downloaded, so its checksum still holds, and the weights - kept in a
//! file of their own beside the graph, by a relative name - are shared,
//! not copied: only the graph, a few hundred kilobytes, is written.
//!
//! The rewrite is done on the protobuf's bytes, not through a model
//! library: only the model's graph and the graph's nodes are opened, and
//! every other field goes through byte for byte.

use std::path::{Path, PathBuf};

/// `accuracy_level` 4: the product in `int8`.
const INT8: u64 = 4;
/// `AttributeProto.AttributeType.INT`.
const ATTRIBUTE_INT: u64 = 2;

/// The graph at `path` with every `MatMulNBits` worked in `int8`: the path
/// of the rewritten graph, written beside it if it is not there already or
/// is older than the graph it comes from. `path` itself when the graph has
/// no such node, or when the rewrite cannot be made or written - the graph
/// as downloaded still runs, only slower, and the log says why.
pub fn int8_graph(path: &Path) -> PathBuf {
    let derived = derived_path(path);
    if is_current(path, &derived) {
        return derived;
    }
    let written = std::fs::read(path)
        .map_err(|error| format!("could not read it: {error}"))
        .and_then(|bytes| rewrite(&bytes))
        .and_then(|rewritten| match rewritten {
            None => Ok(false),
            Some((bytes, nodes)) => write_whole(&derived, &bytes).map(|()| {
                log::info!(
                    "onnx: {} has its {nodes} matrix products worked in int8 from now on",
                    name(path)
                );
                true
            }),
        });
    match written {
        Ok(true) => derived,
        Ok(false) => path.to_owned(),
        Err(error) => {
            log::warn!(
                "onnx: {} is run as downloaded, in f32 and slower: {error}",
                name(path)
            );
            path.to_owned()
        }
    }
}

fn name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// `model.onnx` -> `model.int8.onnx`, in the same folder, so the weights'
/// relative name resolves the same from both.
fn derived_path(path: &Path) -> PathBuf {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy();
    path.with_file_name(format!("{stem}.int8.onnx"))
}

/// Written since the graph it comes from was: a graph downloaded again is
/// rewritten again.
fn is_current(source: &Path, derived: &Path) -> bool {
    let modified = |path: &Path| std::fs::metadata(path).and_then(|meta| meta.modified());
    match (std::fs::metadata(derived), modified(source)) {
        (Ok(meta), Ok(source_time)) => {
            meta.len() > 0 && meta.modified().is_ok_and(|time| time >= source_time)
        }
        _ => false,
    }
}

/// Written beside, then renamed over: a torn write is never taken for a
/// graph.
fn write_whole(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let partial = path.with_extension("onnx.part");
    std::fs::write(&partial, bytes)
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            format!("could not write {}: {error}", name(path))
        })
}

/// The model's bytes with every `MatMulNBits` node's `accuracy_level` set
/// to [`INT8`], and how many nodes that was; `None` when there are none.
fn rewrite(model: &[u8]) -> Result<Option<(Vec<u8>, usize)>, String> {
    let mut nodes = 0;
    // ModelProto.graph = 7; GraphProto.node = 1.
    let out = map_field(model, 7, &mut |graph| {
        map_field(graph, 1, &mut |node| {
            let (node, changed) = node_int8(node)?;
            nodes += usize::from(changed);
            Ok(node)
        })
    })?;
    Ok((nodes > 0).then_some((out, nodes)))
}

/// One node: unchanged unless it is a `MatMulNBits`, which loses any
/// `accuracy_level` it had and gains [`INT8`].
fn node_int8(node: &[u8]) -> Result<(Vec<u8>, bool), String> {
    let fields = fields(node)?;
    // NodeProto.op_type = 4, .attribute = 5; AttributeProto.name = 1.
    let is_target = fields
        .iter()
        .any(|field| field.number == 4 && field.payload == b"MatMulNBits");
    if !is_target {
        return Ok((node.to_vec(), false));
    }
    let mut out = Vec::with_capacity(node.len() + 32);
    for field in &fields {
        let is_level = field.number == 5
            && fields_of(field.payload).is_some_and(|inner| {
                inner
                    .iter()
                    .any(|f| f.number == 1 && f.payload == b"accuracy_level")
            });
        if !is_level {
            out.extend_from_slice(field.whole);
        }
    }
    let mut attribute = Vec::new();
    put_bytes(&mut attribute, 1, b"accuracy_level");
    put_varint_field(&mut attribute, 3, INT8);
    put_varint_field(&mut attribute, 20, ATTRIBUTE_INT);
    put_bytes(&mut out, 5, &attribute);
    Ok((out, true))
}

/// What [`map_field`] makes of one field's payload.
type MapPayload<'a> = dyn FnMut(&[u8]) -> Result<Vec<u8>, String> + 'a;

/// `message` with every length-delimited field `number` replaced by what
/// `map` makes of its payload; every other field as it was.
fn map_field(message: &[u8], number: u64, map: &mut MapPayload) -> Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(message.len() + 4096);
    for field in fields(message)? {
        if field.number == number && field.wire == LEN {
            let payload = map(field.payload)?;
            put_bytes(&mut out, number, &payload);
        } else {
            out.extend_from_slice(field.whole);
        }
    }
    Ok(out)
}

const VARINT: u8 = 0;
const FIXED64: u8 = 1;
const LEN: u8 = 2;
const FIXED32: u8 = 5;

/// One field as it lies in the message.
struct Field<'a> {
    number: u64,
    wire: u8,
    /// The value: a length-delimited field's bytes, else the raw value.
    payload: &'a [u8],
    /// The field whole, tag included, to be written back unchanged.
    whole: &'a [u8],
}

fn fields(message: &[u8]) -> Result<Vec<Field<'_>>, String> {
    fields_of(message).ok_or_else(|| "the graph is not a protobuf this can read".to_owned())
}

/// The fields of `message`, or `None` when it does not parse as one.
fn fields_of(message: &[u8]) -> Option<Vec<Field<'_>>> {
    let mut at = 0;
    let mut out = Vec::new();
    while at < message.len() {
        let start = at;
        let tag = varint(message, &mut at)?;
        let (number, wire) = (tag >> 3, (tag & 7) as u8);
        let payload = match wire {
            VARINT => {
                let from = at;
                varint(message, &mut at)?;
                &message[from..at]
            }
            FIXED64 => take(message, &mut at, 8)?,
            FIXED32 => take(message, &mut at, 4)?,
            LEN => {
                let len = usize::try_from(varint(message, &mut at)?).ok()?;
                take(message, &mut at, len)?
            }
            // Groups (3, 4) are long gone from protobuf and ONNX never used them.
            _ => return None,
        };
        if number == 0 {
            return None;
        }
        out.push(Field {
            number,
            wire,
            payload,
            whole: &message[start..at],
        });
    }
    Some(out)
}

fn take<'a>(message: &'a [u8], at: &mut usize, len: usize) -> Option<&'a [u8]> {
    let end = at.checked_add(len).filter(|&end| end <= message.len())?;
    let slice = &message[*at..end];
    *at = end;
    Some(slice)
}

fn varint(message: &[u8], at: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *message.get(*at)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn put_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn put_varint_field(out: &mut Vec<u8>, number: u64, value: u64) {
    put_varint(out, number << 3 | u64::from(VARINT));
    put_varint(out, value);
}

fn put_bytes(out: &mut Vec<u8>, number: u64, bytes: &[u8]) {
    put_varint(out, number << 3 | u64::from(LEN));
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribute(name: &str, value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        put_bytes(&mut out, 1, name.as_bytes());
        put_varint_field(&mut out, 3, value);
        put_varint_field(&mut out, 20, ATTRIBUTE_INT);
        out
    }

    fn node(op: &str, attributes: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        put_bytes(&mut out, 1, b"x");
        put_bytes(&mut out, 2, b"y");
        put_bytes(&mut out, 4, op.as_bytes());
        for attribute in attributes {
            put_bytes(&mut out, 5, attribute);
        }
        out
    }

    /// A model: an ir_version, a graph of `nodes` with a name, and an
    /// opset after the graph, so fields on both sides of it are kept.
    fn model(nodes: &[Vec<u8>]) -> Vec<u8> {
        let mut graph = Vec::new();
        for node in nodes {
            put_bytes(&mut graph, 1, node);
        }
        put_bytes(&mut graph, 2, b"main");
        let mut out = Vec::new();
        put_varint_field(&mut out, 1, 8);
        put_bytes(&mut out, 7, &graph);
        put_bytes(&mut out, 8, b"\x12\x00\x10\x15");
        out
    }

    /// Each node of a model's graph as (op_type, [(attribute name, value)]).
    fn read(model: &[u8]) -> Vec<(String, Vec<(String, u64)>)> {
        let graph = fields(model)
            .unwrap()
            .into_iter()
            .find(|f| f.number == 7)
            .unwrap();
        fields(graph.payload)
            .unwrap()
            .into_iter()
            .filter(|f| f.number == 1)
            .map(|node| {
                let fields = fields(node.payload).unwrap();
                let op = fields.iter().find(|f| f.number == 4).unwrap().payload;
                let attributes = fields
                    .iter()
                    .filter(|f| f.number == 5)
                    .map(|a| {
                        let inner = fields_of(a.payload).unwrap();
                        let name = inner.iter().find(|f| f.number == 1).unwrap().payload;
                        let mut at = 0;
                        let value = varint(
                            inner.iter().find(|f| f.number == 3).unwrap().payload,
                            &mut at,
                        )
                        .unwrap();
                        (String::from_utf8(name.to_vec()).unwrap(), value)
                    })
                    .collect();
                (String::from_utf8(op.to_vec()).unwrap(), attributes)
            })
            .collect()
    }

    #[test]
    fn every_quantised_product_is_worked_in_int8_and_nothing_else_changes() {
        let before = model(&[
            node(
                "MatMulNBits",
                &[attribute("bits", 8), attribute("block_size", 32)],
            ),
            node("Add", &[]),
            node(
                "MatMulNBits",
                &[attribute("accuracy_level", 0), attribute("bits", 4)],
            ),
        ]);
        let (after, nodes) = rewrite(&before).unwrap().expect("two nodes to change");
        assert_eq!(nodes, 2);
        let read = read(&after);
        assert_eq!(read[0].0, "MatMulNBits");
        assert_eq!(
            read[0].1,
            [
                ("bits".into(), 8),
                ("block_size".into(), 32),
                ("accuracy_level".into(), 4)
            ]
        );
        assert_eq!(read[1], ("Add".into(), vec![]));
        // An accuracy_level already there is replaced, not doubled.
        assert_eq!(
            read[2].1,
            [("bits".into(), 4), ("accuracy_level".into(), 4)]
        );
        // The fields around the graph come through byte for byte.
        let (before_fields, after_fields) = (fields(&before).unwrap(), fields(&after).unwrap());
        assert_eq!(before_fields[0].whole, after_fields[0].whole);
        assert_eq!(before_fields[2].whole, after_fields[2].whole);
        // Done twice, it is the same graph.
        let (again, _) = rewrite(&after).unwrap().unwrap();
        assert_eq!(again, after);
    }

    #[test]
    fn a_graph_with_no_quantised_product_is_left_alone() {
        assert!(
            rewrite(&model(&[node("Conv", &[]), node("Relu", &[])]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn bytes_that_are_not_a_model_are_refused_not_mangled() {
        assert!(rewrite(&[0x0a, 0xff, 0xff]).is_err());
        assert!(rewrite(&[0x0b]).is_err());
    }

    #[test]
    fn the_rewrite_is_written_once_beside_the_graph_and_again_when_the_graph_is() {
        let dir = std::env::temp_dir().join(format!("concat-int8-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("model.onnx");
        std::fs::write(&source, model(&[node("MatMulNBits", &[])])).unwrap();
        let derived = int8_graph(&source);
        assert_eq!(derived, dir.join("model.int8.onnx"));
        assert_eq!(
            read(&std::fs::read(&derived).unwrap())[0].1,
            [("accuracy_level".into(), 4)]
        );
        assert!(!dir.join("model.int8.onnx.part").exists());
        // A graph with nothing to change is run as it is.
        let plain = dir.join("plain.onnx");
        std::fs::write(&plain, model(&[node("Conv", &[])])).unwrap();
        assert_eq!(int8_graph(&plain), plain);
        // A graph that cannot be read is run as it is, too.
        let torn = dir.join("torn.onnx");
        std::fs::write(&torn, [0x0a, 0xff]).unwrap();
        assert_eq!(int8_graph(&torn), torn);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
