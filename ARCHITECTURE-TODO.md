# Architecture: left for later

What the 16 September architecture proposal ticked off with a note that
part of it waits. The proposal itself is gone; `ARCHITECTURE.md` describes
the tree as it is, and this is the list of what it does not yet do. Each
item names where the seam is, so the next person starts at the file and
not at the history.

- [ ] **GPU-texture pooling of cached frames.** The source-frame cache
  (`concat-host/src/pool.rs`) hands out CPU frames; a frame that goes to the
  wgpu compositor is uploaded again each time it is drawn. Pool the textures
  beside the frames now that both compositors read one `FramePlan`.
- [ ] **Zero-copy hardware decode.** VideoToolbox frames
  (`concat-media/src/hardware.rs`) are copied out to system memory and back
  up to the GPU. An IOSurface handed straight to wgpu skips both copies; the
  seam is the decoder's output frame and the compositor's texture upload.
- [ ] **Crop, flips and transition fades filled by the export.** `FramePlan`
  carries all three, the parity tests fill them, and the compositors honour
  them - but the export path still bakes them into the decoder's filter
  chain (`concat-export/src/resolve.rs`, `concat-media` filter graph). Fill
  the plan from the export and take them out of the chain.
- [ ] **Timeline drawing at 2 ms.** Clips are virtualised
  (`TimelinePane::published_span`), but filmstrips and waveforms are still
  one image and one path per clip; the design is one texture per track per
  zoom level. The Slint repaint itself has not been measured
  (`SLINT_DEBUG_PERFORMANCE` needs the window), so the target is not yet
  shown either way.
- [ ] **OS-native restoration behind Enhance.** Enhance runs restoration
  models through `concat-vision/src/runtime.rs` on every platform. macOS 26
  and iOS 26 carry a super-resolution scaler, frame interpolation and motion
  blur in VideoToolbox (`VTFrameProcessor`; Rust bindings in
  `objc2-video-toolbox`), and Windows App SDK carries a video
  super-resolution API. Both belong behind the same enhanced-copy job as a
  fast path chosen per platform, never as a second feature; the seam is the
  job's per-frame step.
- [ ] **Shader passes at a size of their own.** A `[[wgsl.pass]]` declares
  `size` over `WIDTH` and `HEIGHT` (`concat-effects/src/manifest.rs`), but
  `run_passes` (`concat-render/src/gpu.rs`) claims every target at the
  source's size and ignores it. Honouring it is what a GPU upscaler package
  (FSR 1.0, Anime4K, both MIT with WGSL ports) needs to write a larger
  picture than it reads.
