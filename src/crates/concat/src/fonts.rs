// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The base fonts, lent to the window.
//!
//! concat-text embeds them for the titles it paints; the window draws its
//! own text, and a text preset's card names the preset in the preset's
//! face. So the same bytes are handed to Slint's font collection once, at
//! start, before any window exists, rather than embedded a second time.

use std::sync::Arc;

use slint::fontique_011::fontique;

/// Registers every base font with the window's text renderer.
pub fn register() {
    let mut collection = slint::fontique_011::shared_collection();
    for face in concat_text::BASE_FONTS {
        let blob = fontique::Blob::new(Arc::new(face));
        collection.register_fonts(blob, None);
    }
}
