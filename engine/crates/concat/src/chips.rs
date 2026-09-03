// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The picture the cursor carries while something is dragged out of the
//! library or a panel out of its seat.

use crate::format::hex_of;
use crate::ui::ClipKind;

pub fn chip_glyph(kind: ClipKind) -> &'static str {
    match kind {
        // lucide/film
        ClipKind::Video => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                            M7 3v18 M3 7.5h4 M3 12h18 M3 16.5h4 M17 3v18 M17 7.5h4 M17 16.5h4"
        }
        // lucide/music
        ClipKind::Audio => {
            "M9 18V5l12-2v13 M3 18a3 3 0 1 0 6 0a3 3 0 1 0 -6 0 \
                            M15 16a3 3 0 1 0 6 0a3 3 0 1 0 -6 0"
        }
        // lucide/image
        ClipKind::Image => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                            M7 9a2 2 0 1 0 4 0a2 2 0 1 0 -4 0 M21 15l-3.086-3.086a2 2 0 0 0-2.828 0L6 21"
        }
        // lucide/type
        ClipKind::Text => "M12 4v16 M4 7V5a1 1 0 0 1 1-1h14a1 1 0 0 1 1 1v2 M9 20h6",
        // lucide/audio-lines
        ClipKind::Filter => "M2 10v3 M6 6v11 M10 3v18 M14 8v7 M18 5v13 M22 10v3",
    }
}

/// The same again for a panel being dragged out of its seat. The slugs are
/// the ones `Panes.id` hands out, and they are what the payload carries.
pub fn pane_glyph(slug: &str) -> &'static str {
    match slug {
        // lucide/image
        "preview" => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                      M7 9a2 2 0 1 0 4 0a2 2 0 1 0 -4 0 M21 15l-3.086-3.086a2 2 0 0 0-2.828 0L6 21"
        }
        // lucide/sliders-horizontal
        "inspector" => {
            "M10 5H3 M12 19H3 M14 3v4 M16 17v4 M21 12h-9 M21 19h-5 M21 5h-7 \
                        M8 10v4 M8 12H3"
        }
        // lucide/rows-3
        "timeline" => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
                       M21 9H3 M21 15H3"
        }
        // lucide/film
        _ => {
            "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1 -2 2h-14a2 2 0 0 1 -2 -2v-14a2 2 0 0 1 2 -2Z \
              M7 3v18 M3 7.5h4 M3 12h18 M3 16.5h4 M17 3v18 M17 7.5h4 M17 16.5h4"
        }
    }
}

/// `&`, `<` and `>` out of a name that is going into an SVG document.
pub fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The chip that hangs off the cursor while something is dragged out of the
/// library, as an SVG document.
///
/// SVG rather than a pixel buffer, and for two reasons. Slint rasterises one
/// through the window's own font collection — so the name on the chip is set
/// in the same Inter the cards are, from the face this binary already
/// embeds — and the drag overlay draws it at the window's scale factor rather
/// than at whatever resolution a buffer was baked at.
///
/// Sized in logical points, because that is how the overlay reads it back:
/// `render_drag_image_overlay` takes the image's own size as its size on
/// screen. `PAD` is the margin the drop shadow needs to fall into; the visible
/// chip is inset by it, which is why the offsets the DragAreas pass are that
/// much larger than the gap they want.
pub fn drag_chip_svg(
    glyph: &str,
    label: &str,
    wave: &str,
    mark: slint::Color,
    well: slint::Color,
    ground: slint::Color,
    ink: slint::Color,
) -> String {
    /// Room for the shadow to fall into, on every side.
    const PAD: f32 = 5.0;
    const CHIP_H: f32 = 32.0;
    /// The badge, and the glyph centred in it.
    const BADGE: f32 = 22.0;
    const MARK: f32 = 16.0;
    /// Inter's average advance at 12px, rounded up. A chip a few points wider
    /// than its text is a chip; one a few points narrower is a bug.
    const ADVANCE: f32 = 6.4;
    /// Long enough for a take name, short enough not to become a banner.
    const MAX_CHARS: usize = 26;

    let label: String = if label.chars().count() > MAX_CHARS {
        label.chars().take(MAX_CHARS - 1).collect::<String>() + "\u{2026}"
    } else {
        label.to_string()
    };
    let text_x = PAD + 10.0 + BADGE;
    let chip_w = (text_x - PAD + label.chars().count() as f32 * ADVANCE + 12.0).clamp(96.0, 268.0);
    let (width, height) = (chip_w + 2.0 * PAD, CHIP_H + 2.0 * PAD);

    // The badge holds the file's own envelope when there is one, and the
    // kind's mark otherwise — the same rule the bin card follows, so what is
    // in the air looks like the card it came off.
    let badge_art = if wave.is_empty() {
        format!(
            r#"<g transform="translate({x} {y}) scale({scale})" fill="none" stroke="{mark}"
stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><path d="{glyph}"/></g>"#,
            x = PAD + 5.0 + (BADGE - MARK) / 2.0,
            y = PAD + 5.0 + (BADGE - MARK) / 2.0,
            scale = MARK / 24.0,
            mark = hex_of(mark),
        )
    } else {
        format!(
            r#"<g transform="translate({x} {y}) scale({BADGE})"><path d="{wave}" fill="{mark}"/></g>"#,
            x = PAD + 5.0,
            y = PAD + 5.0,
            mark = hex_of(mark),
        )
    };

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}">
<filter id="drop" x="-30%" y="-30%" width="170%" height="170%">
<feDropShadow dx="0" dy="1.5" stdDeviation="2" flood-color="#000000" flood-opacity="0.55"/></filter>
<rect x="{cx}" y="{cy}" width="{cw}" height="{ch}" rx="8" fill="{ground}" stroke="{mark}" stroke-opacity="0.65" filter="url(#drop)"/>
<rect x="{bx}" y="{by}" width="{BADGE}" height="{BADGE}" rx="6" fill="{well}"/>
{badge_art}
<text x="{tx}" y="{ty}" font-family="Inter" font-size="12" fill="{ink}">{label}</text>
</svg>"##,
        cx = PAD + 0.5,
        cy = PAD + 0.5,
        cw = chip_w - 1.0,
        ch = CHIP_H - 1.0,
        bx = PAD + 5.0,
        by = PAD + 5.0,
        tx = text_x,
        // The baseline, not the middle: usvg honours `dominant-baseline`
        // unevenly, and a number here is one fewer thing to be surprised by.
        ty = PAD + CHIP_H / 2.0 + 4.2,
        ground = hex_of(ground),
        mark = hex_of(mark),
        well = hex_of(well),
        ink = hex_of(ink),
        label = xml_escaped(&label),
    )
}

// ── the launch screen ────────────────────────────────────────────────────────
