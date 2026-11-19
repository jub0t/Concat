// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What a run found, written to `result.json` beside the frames that
//! failed, so a red run on a CI machine can be read without running it
//! again.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One thing a case asserts, and whether it held.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// What was checked, as a sentence.
    pub name: String,
    /// Whether it held.
    pub passed: bool,
    /// What was found, said either way.
    pub detail: String,
}

/// A whole run of one case.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The case.
    pub case: String,
    /// Every check held, and nothing stopped the run.
    pub passed: bool,
    /// What stopped the run before its checks were done, if anything.
    pub error: Option<String>,
    /// The checks, in the order they were made.
    pub checks: Vec<Check>,
    /// The furthest any code block of any frame read was from its level,
    /// in 8-bit levels, out of the [`crate::code::MARGIN`] a read allows:
    /// how close the export came to being unreadable.
    pub code_worst: f64,
    /// The exported file, while it is kept.
    pub export: Option<String>,
    /// The frames written out for a person to look at.
    pub frames: Vec<String>,
    /// How long the run took, in seconds.
    pub seconds: f64,
}

impl Report {
    /// Records a check.
    pub fn check(&mut self, name: impl Into<String>, passed: bool, detail: impl Into<String>) {
        self.checks.push(Check {
            name: name.into(),
            passed,
            detail: detail.into(),
        });
    }

    /// Sets [`Report::passed`] from the checks and the error.
    pub fn settle(&mut self) {
        self.passed = self.error.is_none() && self.checks.iter().all(|check| check.passed);
    }

    /// Writes the report to `result.json` in `dir`.
    pub fn write(&self, dir: &Path) -> Result<PathBuf, String> {
        let path = dir.join("result.json");
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("could not encode the report: {error}"))?;
        std::fs::write(&path, text)
            .map_err(|error| format!("could not write {}: {error}", path.display()))?;
        Ok(path)
    }

    /// The failures, a line each, for a test's panic message.
    pub fn summary(&self) -> String {
        let mut lines = Vec::new();
        if let Some(error) = &self.error {
            lines.push(format!("stopped: {error}"));
        }
        for check in self.checks.iter().filter(|check| !check.passed) {
            lines.push(format!("failed: {}: {}", check.name, check.detail));
        }
        lines.join("\n")
    }
}
