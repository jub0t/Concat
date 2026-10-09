# Patched dependencies

Crates copied from crates.io with one change each, applied through the
workspace's `[patch.crates-io]`. Each is the published version the
`Cargo.lock` names, so an upgrade is: copy the new version in, re-apply
the change, and read it again.

## i-slint-renderer-skia 1.18.1

`itemrenderer.rs`, `draw_glyph_run`: glyphs are positioned on whole
pixels when the window's scale factor is under 1.5. Slint always asks
Skia for subpixel positioning, which on a 1x Windows screen lands stems
between pixels and the text reads blurred (jub0t/concat#278). On a
high-density screen the subpixel positions stay, where they are worth
their spacing. Nothing else differs from the published crate.
