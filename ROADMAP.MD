# Roadmap

> Concat is still in early development. This roadmap represents the direction of the project rather than guaranteed features or release dates.
>
> 🌟 = high priority / especially wanted

### 🧱 Editor & Core Architecture

* [ ] 🌟 **Concat API** — A stable API shared by the Concat UI, plugins, scripts, automation, and MCP integrations.
* [ ] 🌟 **Extensible Effects Runtime** — A scalable, data-driven system capable of supporting hundreds of effects, filters, transitions, and generators, including custom WGSL shaders.
* [ ] **Hardware Analysis** — Detect device capabilities and performance tier so Concat can automatically choose appropriate playback, rendering, proxy, and processing settings.
* [ ] **Customizable Workspace** — Allow panels to be moved, resized, docked, and rearranged similar to Blender and other professional creative tools.

### 🎨 Effects, Transitions & Mixers

* [ ] 🌟 **Effect Mixer** — Create custom effects by combining reusable processing nodes, with support for custom WGSL shader nodes.
* [ ] 🌟 **Transition Mixer** — Build and share custom transitions from reusable nodes or custom shaders.
* [ ] 🌟 **Voice Mixer** — Create custom voice effects by combining audio processing stages such as pitch shift, formant shift, EQ, compression, reverb, delay, chorus, distortion, noise gate, limiter, stereo width, filters, de-essing, excitation, and more.
* [ ] **Generators** — Procedural visual sources such as gradients, noise, shapes, animated backgrounds, particles, grids, audio visualizers, and other generated media.
* [ ] **Reusable Creative Assets** — Expand support for color presets, caption styles, title animations, motion presets, effect presets, and other reusable creative components.

### 📦 Templates & Community Ecosystem

* [ ] 🌟 **Concat Hub / Registry** — A centralized community registry for installing and sharing templates, effects, transitions, caption styles, presets, generators, Mixer creations, and other extensions — somewhat like npm for creative assets.
* [ ] **Improved Templates** — Expand the template system and make templates easier to create, customize, package, and share.
* [ ] **Stock Media Library** — Integrate accessible stock video, images, music, and sound effects.

### 🎬 Timeline & Animation

* [ ] 🌟 **Keyframe Graph Editor** — Fine-grained control over animation curves, easing, interpolation, and effect parameters.
* [ ] **Speed Curves / Time Remapping** — Custom speed ramps, reusable velocity presets, and improved slow/fast-motion workflows.
* [ ] **Adjustment Layers** — Apply effects, filters, and color adjustments across multiple clips.
* [ ] **Scene Detection** — Detect cuts in imported footage and optionally split clips automatically.
* [ ] **Beat Detection & Beat Markers** — Analyze music and create timeline markers for rhythm-based editing.
* [ ] **Auto Cut / Beat Sync** — Automatically create edits synchronized to music and detected beats.

### 📝 Captions & Transcript Editing

* [ ] 🌟 **Advanced Caption Styling** — Reusable caption styles, animations, per-word effects, and other creator-focused caption tools.
* [ ] **Word Highlighting** — Use word-level transcription timestamps to highlight spoken words dynamically.
* [ ] **Transcript-Based Editing** — Edit spoken content through its transcript, with timeline edits automatically following text changes.
* [ ] **Filler-Word Removal** — Detect and remove filler words such as “um”, “uh”, and similar speech disfluencies.
* [ ] **Profanity Detection** — Detect profanity from transcribed audio and provide options to censor, mute, or modify affected sections.

### 🎙️ Audio

* [ ] 🌟 **Noise Removal / Voice Cleanup** — Local audio denoising and speech enhancement.
* [ ] **Silence Detection & Removal** — Automatically detect silent regions and optionally remove them from the timeline.
* [ ] **Audio Normalization** — Quickly normalize dialogue and other audio to useful loudness targets.
* [ ] **Beat Analysis** — Reusable audio analysis infrastructure for beat detection, synchronization, and music-driven effects.

### 👁️ Tracking, Masks & Computer Vision

* [ ] 🌟 **Object & Face Tracking** — Track people, faces, and selected objects through footage and attach effects, text, masks, or other elements to tracked targets.
* [ ] 🌟 **Masks & Tracked Masks** — Rectangle, ellipse, polygon/Bézier, feathering, expansion, inversion, keyframing, and tracking.
* [ ] **Automatic Face Detection & Blurring** — Detect and optionally track faces for automatic privacy blurring.
* [ ] **Background Removal** — Local subject segmentation and background removal without requiring a green screen.
* [ ] **Auto Reframe** — Automatically reframe footage for different aspect ratios while keeping important subjects in view.
* [ ] **Chroma Key** — Green/blue-screen removal with adjustable tolerance, spill suppression, feathering, and masking.

### 🔎 Media Intelligence & Automation

* [ ] **Smart Media Search** — Search local project media using speech, filenames, metadata, and eventually visual content.
* [ ] **Long Video → Clips** — Help identify and extract useful short-form segments from longer recordings.
* [ ] **Local Media Analysis** — Build reusable background analysis for transcripts, scenes, faces, audio, and other metadata that multiple Concat features can share.

### 🔌 Plugins & Extensibility

* [ ] 🌟 **Plugin System** — Allow developers to extend Concat through the Concat API without modifying the core application.
* [ ] **Custom Mixer Nodes** — Let developers create reusable nodes for Mixers, including GPU shader nodes and sandboxed processing nodes.
* [ ] **Extension Permissions & Sandboxing** — Provide a safe capability/permission model for third-party extensions.
* [ ] **Extension Versioning & Dependencies** — Stable API versions, dependency resolution, compatibility checks, and updates for community extensions.

---

### Longer-Term Ideas

These are areas worth exploring after the core editing experience is mature:

* [ ] Optical-flow frame interpolation
* [ ] Video stabilization
* [ ] Video denoising and restoration
* [ ] Local video upscaling
* [ ] Motion Mixer / reusable animation graphs
* [ ] Caption Mixer
* [ ] Advanced color grading and LUT workflows
* [ ] Local semantic video search
* [ ] Community-created automation tools
* [ ] Plugin SDK and developer tooling
