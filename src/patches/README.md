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
