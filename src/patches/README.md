# Patched dependencies

Crates copied from crates.io with the changes documented below, applied through the
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

## i-slint-backend-android-activity 1.18.1

A mouse wheel on an Android phone or tablet - a USB or Bluetooth mouse, a
desktop mode - closed the app on its first turn (jub0t/concat#800): the
backend's motion events end in `MotionAction::Scroll => todo!()`, and the
panic took the activity with it. One change, marked `Concat:` in
`androidwindowadapter.rs`:

- the scroll is dispatched as `WindowEvent::PointerScrolled` at the
  pointer, its `AXIS_VSCROLL` and `AXIS_HSCROLL` scaled as the winit
  backend scales a desktop wheel's line (60 logical pixels a notch, the
  same signs), so the timeline and the lists scroll with a wheel as they
  do on a desk.

Slint 1.19.0 and its main branch still have the `todo!()`; drop the copy
once a release handles the scroll. `Cargo.lock` and the crate's registry
metadata files are left out.

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
metadata files are left out too.

The bundled whisper.cpp 1.8.3 also carries the upstream timestamp fix
[c33c5618b72bb345df029b730b36bc0e369845a3](https://github.com/ggml-org/whisper.cpp/commit/c33c5618b72bb345df029b730b36bc0e369845a3)
(whisper.cpp #2279, MIT licence), backported without changes to
`whisper.cpp/src/whisper.cpp`. The upstream commit linked above contains
the complete original diff.

The old segment parser skipped consecutive timestamp tokens after a
segment and reused its end as the next segment's start. Captions after a
pause could therefore begin at the end of the preceding caption, covering
the silence. The corrected parser uses the last consecutive timestamp as
the next start. The upstream patch also preserves these tokens when
printing special tokens and flushes realtime output consistently.

This changes the common segment parser for every Whisper model and
language; no model download, decoding option or application setting changes.
It does not guarantee exact alignment, remove leading silence from the
first caption, or add word timestamps or word-by-word animation. The
existing shared-library build and CPU backend selection are unchanged.

Apart from the changes documented here, the copy matches the published
crate. When refreshing it, check whether the new whisper.cpp already
includes this fix before reapplying it.

## wgpu-hal 30.0.1

wgpu's memory gates refuse a resource once the driver's budget is 95%
spent and lose the device at 99% (Concat opens every device with them;
see `MEMORY_BUDGET` in concat-render's gpu.rs). On Vulkan the published
crate checks the heap of the memory type gpu-allocator *prefers*. For a
buffer the CPU writes (`MemoryLocation::CpuToGpu`) that is a type both
device-local and host-visible, which on a card without resizable BAR is
the BAR: a 256 MB heap every program on the machine shares, whose budget
of 88-120 MB is likely true. gpu-allocator puts such a buffer in plain
host memory when the BAR is full; the gate refused it first. A 2 GB NVIDIA
card with 1.5 GB free refused a 208-byte buffer, its next device would not
open, and the window's device was lost at the next submit over the same
heap (jub0t/concat#746). Two changes, marked `Concat:` in
`src/vulkan/device.rs`:

- `error_if_would_oom_on_resource_allocation`: where the preferred heap is
  past the gate, a `CpuToGpu` request is gated on the heap of the first
  host-visible, host-coherent type it may use, the one gpu-allocator falls
  back to, and allowed when that heap has room.
- `check_if_oom`: the BAR (`MemoryProperties::is_bar`: a device-local heap
  only host-visible types live on, beside a larger device-local heap) is
  left out of the device-loss gate. Only buffers the CPU writes land there,
  and they move to system memory once it is full, so the card's own memory,
  which the gate protects, is no fuller for it.

Textures, buffers the GPU alone uses, readback buffers and Direct3D 12 are
gated exactly as published. Nothing else differs from the published crate;
`Cargo.lock` and the registry metadata files are left out. Drop the copy
once wgpu's own gate follows the allocator's fallback.
