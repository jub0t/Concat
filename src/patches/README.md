# Patched dependencies

Crates copied from crates.io with one change each, applied through the
workspace's `[patch.crates-io]`. Each is the published version the
`Cargo.lock` names, so an upgrade is: copy the new version in, re-apply
the change, and read it again.

## i-slint-renderer-skia 1.18.1

Text on a 1x Windows screen read soft and grey (jub0t/concat#278). Three
changes, all marked `Concat:` in the source:

- `itemrenderer.rs`, `draw_glyph_run`: glyphs are positioned on whole
  pixels when the window's scale factor is under 1.5. Slint always asks
  Skia for subpixel positioning, which on a 1x screen lands stems between
  pixels and the text reads blurred. On a high-density screen the subpixel
  positions stay, where they are worth their spacing. The edging is LCD
  where the canvas's top layer names a subpixel order and greyscale
  everywhere else. Not LCD everywhere: on a surface with no order Skia
  derives greyscale from LCD rendering, which on macOS turns on CoreText's
  font smoothing and draws text heavier.
- `wgpu_30_surface/dx12.rs`, `system_text_props`, and its caller
  `make_swapchain_surface`: the Windows window's surface carries the
  desk's ClearType settings (on or off, RGB or BGR), so its text is drawn
  with ClearType as the system's own is. A texture rendered to stays
  greyscale.
- `itemrenderer.rs`, `create_layer_target`: a layer's surface is created with
  default props, so text in a transparent, cached layer is greyscale
  rather than inheriting the window's subpixel order.

And one for Android, marked the same way:

- `lib.rs`, `default_wgpu_30` and `android_wgpu_30_or_opengl_surface_factory`:
  Slint's Android backend asks for the wgpu surface by name once a wgpu
  feature is on, and wgpu on Android is Vulkan alone, so a phone with no
  Vulkan driver wgpu can use got no window and the app closed as it
  opened. On Android the wgpu surface is tried first and OpenGL ES after
  it, the way `create_default_surface` falls back on every other platform.

Nothing else differs from the published crate.

## whisper-rs-sys 0.15.0

whisper.cpp's Rust build. The published build script builds whisper and
ggml as static libraries only, so ggml's CPU kernels are compiled for one
instruction set: AVX2 crashed Sandy and Ivy Bridge machines at launch
(jub0t/concat#193), and AVX alone made transcription some eight times
slower everywhere else (#388). One change, marked `Concat:` in `build.rs`:

- `GGML_BACKEND_DL=ON` in the environment builds whisper, ggml and
  ggml-base as shared libraries and every backend as a module ggml loads
  at run time, links the three libraries rather than the static archives,
  and copies the runtime files into the profile directory for `cargo run`.
  With `GGML_CPU_ALL_VARIANTS=ON` the CPU backend is one module per x86_64
  generation, and ggml loads the best one the CPU can run; concat-speech's
  `cpu_backend` tells it where to look. build-app.yml turns both on for the
  x86_64 Windows and Linux bundles. Unset, the build is the published one.

And the copy is smaller than the published crate: ggml's GPU backends that
no build of Concat compiles (CANN, CUDA, Hexagon, HIP, MUSA, OpenCL, RPC,
SYCL, Vulkan, WebGPU, zDNN, ZenDNN) are left out of
`whisper.cpp/ggml/src`, so the crate's `cuda`, `hipblas`, `vulkan` and
`intel-sycl` features no longer build. CMake only enters a backend's folder
when that backend is turned on. `Cargo.lock` and the crate's registry
metadata files are left out too. Nothing else differs from the published
crate.
