// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! One ONNX model, loaded and ready to run.
//!
//! A thin coat over ONNX Runtime: a session with the platform's
//! accelerator registered ahead of the CPU - CoreML on Apple, DirectML on
//! Windows, NNAPI on Android - and one call that takes named `f32`
//! tensors and gives named `f32` tensors back. Every model this crate
//! runs is a picture in and a picture out, so that is all the surface it
//! needs; what each model's tensors mean lives beside the model.

use ort::session::Session;
use ort::value::Tensor;

/// A tensor by name: its dimensions and its values, row-major.
pub struct Input<'a> {
    /// The graph's input name.
    pub name: &'a str,
    /// The tensor's shape.
    pub dims: Vec<usize>,
    /// The values, `dims` product of them.
    pub data: Data,
}

/// A tensor's values. Pictures and probabilities are `f32`; the brush
/// model's point labels are `i64`.
pub enum Data {
    /// Floating point values.
    F32(Vec<f32>),
    /// Integer values.
    I64(Vec<i64>),
}

impl From<Vec<f32>> for Data {
    fn from(values: Vec<f32>) -> Data {
        Data::F32(values)
    }
}

impl From<Vec<i64>> for Data {
    fn from(values: Vec<i64>) -> Data {
        Data::I64(values)
    }
}

/// An `f32` tensor a model answered with.
#[derive(Clone, Debug)]
pub struct Output {
    /// The tensor's shape.
    pub dims: Vec<usize>,
    /// The values, row-major.
    pub data: Vec<f32>,
}

/// Makes ONNX Runtime ready for a first session, or says why it cannot
/// run here. Every session goes through here first, Chatterbox's too, and
/// so does every model download: a gigabyte fetched for a runtime that
/// cannot run it is a gigabyte wasted.
///
/// Only an Intel Mac has anything to do. Its runtime is loaded now rather
/// than at launch (see [`crate::onnx`]), and not at all on a macOS older
/// than the one it is built for, where loading it is what closed the app.
/// The answer is kept: neither changes while the process runs.
pub fn ready() -> Result<(), String> {
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        static LOADED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        LOADED
            .get_or_init(|| {
                if let Some(why) = crate::onnx::unavailable() {
                    return Err(why.to_string());
                }
                let library = crate::onnx::library();
                // `commit` is false when an environment is already set,
                // which is nothing to report: the library is loaded.
                let _ = ort::init_from(&library)
                    .map_err(|error| format!("could not load ONNX Runtime: {error}"))?
                    .commit();
                Ok(())
            })
            .clone()
    }
    #[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
    Ok(())
}

/// A loaded model.
pub struct Model {
    session: Session,
}

impl Model {
    /// Loads a model from its bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Model, String> {
        Model::build(bytes, &[], accelerators())
    }

    /// Loads a model whose input is always one size: its free dimensions
    /// fixed by name, and on Apple's machines CoreML asked for Apple's
    /// newer MLProgram format with the shape known up front, which is what
    /// lets it put the whole network on the GPU. Without that it took only
    /// what its older format could, and gave the rest to the CPU: Enhance
    /// ran at 27 seconds a 1080p frame through it, and 1.8 like this.
    pub fn from_file_fixed(path: &std::path::Path, dims: &[(&str, i64)]) -> Result<Model, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        Model::build(&bytes, dims, fixed_accelerators())
    }

    fn build(
        bytes: &[u8],
        dims: &[(&str, i64)],
        providers: Vec<ort::ep::ExecutionProviderDispatch>,
    ) -> Result<Model, String> {
        ready()?;
        let mut builder = Session::builder().map_err(|error| format!("onnx runtime: {error}"))?;
        for (name, size) in dims {
            builder = builder
                .with_dimension_override(*name, *size)
                .map_err(|error| format!("onnx runtime: {error}"))?;
        }
        builder = builder
            .with_execution_providers(providers)
            .map_err(|error| format!("onnx runtime: {error}"))?;
        // Every core on one frame at a time: the models are convolutional
        // and a frame fills a machine on its own.
        let threads = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(1)
            .clamp(1, 8);
        builder = builder
            .with_intra_threads(threads)
            .map_err(|error| format!("onnx runtime: {error}"))?;
        let session = builder
            .commit_from_memory(bytes)
            .map_err(|error| format!("cutout model: {error}"))?;
        Ok(Model { session })
    }

    /// Loads a model from a file.
    pub fn from_file(path: &std::path::Path) -> Result<Model, String> {
        let bytes = std::fs::read(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        Model::from_bytes(&bytes)
    }

    /// Runs the model on `inputs` and returns the named outputs, in the
    /// order asked for.
    pub fn run(&mut self, inputs: Vec<Input<'_>>, outputs: &[&str]) -> Result<Vec<Output>, String> {
        let mut fed: Vec<(
            std::borrow::Cow<'static, str>,
            ort::session::SessionInputValue<'_>,
        )> = Vec::with_capacity(inputs.len());
        for input in inputs {
            let value: ort::session::SessionInputValue<'_> = match input.data {
                Data::F32(data) => Tensor::from_array((input.dims, data))
                    .map_err(|error| format!("cutout input {}: {error}", input.name))?
                    .into(),
                Data::I64(data) => Tensor::from_array((input.dims, data))
                    .map_err(|error| format!("cutout input {}: {error}", input.name))?
                    .into(),
            };
            fed.push((input.name.to_owned().into(), value));
        }
        let answered = self
            .session
            .run(fed)
            .map_err(|error| format!("cutout: {error}"))?;
        outputs
            .iter()
            .map(|name| {
                let value = answered
                    .get(*name)
                    .ok_or_else(|| format!("cutout: the model has no output {name:?}"))?;
                let (shape, data) = value
                    .try_extract_tensor::<f32>()
                    .map_err(|error| format!("cutout output {name}: {error}"))?;
                Ok(Output {
                    dims: shape.iter().map(|&d| d.max(0) as usize).collect(),
                    data: data.to_vec(),
                })
            })
            .collect()
    }
}

/// The platform's accelerators, ahead of the CPU. A provider the machine
/// cannot supply is skipped, not an error, so the same build runs on a
/// box without one.
fn accelerators() -> Vec<ort::ep::ExecutionProviderDispatch> {
    platform_accelerator()
        .into_iter()
        .chain(std::iter::once(ort::ep::CPU::default().build()))
        .collect()
}

#[cfg(target_vendor = "apple")]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    Some(ort::ep::CoreML::default().build())
}

/// [`accelerators`] for a model of one fixed shape; see
/// [`Model::from_file_fixed`]. The GPU rather than the Neural Engine:
/// measured on an M5, CoreML ran Enhance's network four times faster on
/// the GPU than on the Neural Engine with the CPU.
fn fixed_accelerators() -> Vec<ort::ep::ExecutionProviderDispatch> {
    #[cfg(target_vendor = "apple")]
    {
        use ort::ep::coreml::{ComputeUnits, ModelFormat, SpecializationStrategy};
        vec![
            ort::ep::CoreML::default()
                .with_model_format(ModelFormat::MLProgram)
                .with_static_input_shapes(true)
                .with_specialization_strategy(SpecializationStrategy::FastPrediction)
                .with_compute_units(ComputeUnits::CPUAndGPU)
                .build(),
            ort::ep::CPU::default().build(),
        ]
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        accelerators()
    }
}

#[cfg(target_os = "android")]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    Some(ort::ep::NNAPI::default().build())
}

// Not DirectML on Windows: the bundle ships Microsoft's CPU build of the
// runtime, which has no DirectML, and asking it for one crashed the second
// session a process opened - an object cutout after a person one (#399).
#[cfg(not(any(target_vendor = "apple", target_os = "android")))]
fn platform_accelerator() -> Option<ort::ep::ExecutionProviderDispatch> {
    None
}
