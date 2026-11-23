// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What the references may weigh, and a walk over every case's `golden`
//! folder that says whether they do.
//!
//! References are PNGs in the repository itself, not in LFS, so every one
//! ever committed is in every clone for good. A budget for each and for
//! all of them is what keeps a golden a small frame of a small project,
//! and not a screenshot. The same walk holds each folder to its
//! `golden.json`: a PNG the file does not name, or a name with no PNG, is
//! a reference nothing checks or a check with nothing behind it.

use std::path::{Path, PathBuf};

use super::bounds::{self, Metadata};
use super::picture::Rgba8Image;

/// The most one reference may be: 48 KB.
pub const REFERENCE_BYTES: u64 = 48 * 1024;

/// The most all the references together may be: 6 MB.
pub const TOTAL_BYTES: u64 = 6 * 1024 * 1024;

/// What the references may weigh.
#[derive(Clone, Copy, Debug)]
pub struct Budget {
    /// The most one reference may be, in bytes.
    pub reference: u64,
    /// The most all of them together may be, in bytes.
    pub total: u64,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            reference: REFERENCE_BYTES,
            total: TOTAL_BYTES,
        }
    }
}

/// What a walk over the cases' references found.
#[derive(Clone, Debug, Default)]
pub struct Audit {
    /// Every reference and its size in bytes, in order of path.
    pub references: Vec<(PathBuf, u64)>,
    /// Their sizes together.
    pub total: u64,
    /// Everything that is wrong, a line each. Empty when all is well.
    pub problems: Vec<String>,
}

/// Walks the `golden` folder of every case under `cases` and holds what
/// is there to `budget` and to each folder's `golden.json`. An error is a
/// folder that could not be listed; everything else that is wrong is in
/// [`Audit::problems`].
pub fn audit(cases: &Path, budget: Budget) -> Result<Audit, String> {
    let mut audit = Audit::default();
    for case in list(cases)? {
        let folder = case.join(bounds::FOLDER);
        if !folder.is_dir() {
            continue;
        }
        let metadata = match Metadata::load(&case) {
            Ok(metadata) => Some(metadata),
            Err(error) => {
                audit.problems.push(error);
                None
            }
        };
        let mut named: Vec<&String> = metadata
            .iter()
            .flat_map(|metadata| metadata.goldens.keys())
            .collect();
        for path in list(&folder)? {
            let file = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            // The file itself, and what a file manager leaves behind,
            // which git does not keep.
            if file == bounds::FILE || file.starts_with('.') {
                continue;
            }
            let Some(name) = file.strip_suffix(".png").filter(|_| path.is_file()) else {
                audit.problems.push(format!(
                    "{} is neither a reference nor {}",
                    path.display(),
                    bounds::FILE
                ));
                continue;
            };
            let size = std::fs::metadata(&path)
                .map_err(|error| format!("could not size {}: {error}", path.display()))?
                .len();
            audit.total += size;
            if size > budget.reference {
                audit.problems.push(format!(
                    "{} is {size} bytes, over the {} a reference may be",
                    path.display(),
                    budget.reference
                ));
            }
            if let Err(error) = Rgba8Image::read_png(&path) {
                audit.problems.push(error);
            }
            if metadata.is_some() {
                match named.iter().position(|known| known.as_str() == name) {
                    Some(at) => {
                        named.swap_remove(at);
                    }
                    None => audit.problems.push(format!(
                        "{} is a reference {} does not name",
                        path.display(),
                        Metadata::path(&case).display()
                    )),
                }
            }
            audit.references.push((path, size));
        }
        named.sort();
        for name in named {
            audit.problems.push(format!(
                "{} names {name}, which has no reference",
                Metadata::path(&case).display()
            ));
        }
    }
    if audit.total > budget.total {
        audit.problems.push(format!(
            "the references under {} are {} bytes together, over the {} they may be",
            cases.display(),
            audit.total,
            budget.total
        ));
    }
    Ok(audit)
}

/// What is in `folder`, in order of name.
fn list(folder: &Path) -> Result<Vec<PathBuf>, String> {
    let fail = |error: std::io::Error| format!("could not list {}: {error}", folder.display());
    let mut paths = Vec::new();
    for entry in std::fs::read_dir(folder).map_err(fail)? {
        paths.push(entry.map_err(fail)?.path());
    }
    paths.sort();
    Ok(paths)
}
