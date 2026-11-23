// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The corpus: small projects built through the API, exported through the
//! app's own export, read back, and checked without a stored picture.
//!
//! Each case lives in `src/corpus/cases/<name>/case.json` (see [`case`]).
//! What makes it checkable on every OS, GPU and encoder alike is that its
//! media says what it is: every generated frame carries a [`code`] naming
//! its media and its frame, so an exported frame read back names the
//! source frame it shows, and the case says which one that should be. A
//! cut that drops a frame, holds one or shows black is then a wrong code
//! at a known frame, not a picture that looks a little off.
//!
//! Beside that, two renders that must be equal are held to each other:
//! the document before a save and after the reopen, and the previews of a
//! case and of its twin, the same edit made another way (a clip split in
//! two against the clip whole). The previews are the compositor's own
//! pixels, before any encoder, so they are compared byte for byte.
//!
//! [`run_case`] runs one case and writes `result.json`, and any frame that
//! failed as a PNG, under `target/corpus/<case>/`; `tests/corpus.rs` runs
//! each case as a test of its own.

pub mod case;
pub mod code;
pub mod golden;
pub mod media;
pub mod report;
mod run;
pub mod sound;

use std::path::{Path, PathBuf};

pub use report::{Check, Report};
pub use run::run_case;

/// Where the cases are: `src/corpus/cases`, beside the crates.
pub fn cases_dir() -> PathBuf {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let src = crate_dir
        .parent()
        .and_then(Path::parent)
        .unwrap_or(crate_dir);
    src.join("corpus").join("cases")
}
