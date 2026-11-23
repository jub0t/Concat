// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A run's goldens as one page: what needs looking at first, each with
//! its four pictures side by side and its numbers against their bounds.
//!
//! The page is one file. Its pictures are inside it, its style is inside
//! it, and it runs no script, so it can be taken out of a CI run's
//! artefacts, mailed, or opened from a folder that has since been
//! emptied, and still shows what the run saw. It follows the reader's
//! light or dark setting.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use base64::Engine as _;

use super::GoldenResult;

/// Where a result stands, in the order the page shows them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Standing {
    Failed,
    Unexpected,
    Blessed,
    Passed,
}

impl Standing {
    fn of(result: &GoldenResult) -> Standing {
        if !result.passed {
            Standing::Failed
        } else if result.unexpected_pass {
            Standing::Unexpected
        } else if result.blessed {
            Standing::Blessed
        } else {
            Standing::Passed
        }
    }

    fn word(self) -> &'static str {
        match self {
            Standing::Failed => "Failed",
            Standing::Unexpected => "Passed unexpectedly",
            Standing::Blessed => "Blessed",
            Standing::Passed => "Passed",
        }
    }

    fn class(self) -> &'static str {
        match self {
            Standing::Failed => "failed",
            Standing::Unexpected => "unexpected",
            Standing::Blessed => "blessed",
            Standing::Passed => "passed",
        }
    }
}

/// Text made safe to put in the page, in an element or an attribute.
fn escape(text: &str) -> String {
    let mut safe = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => safe.push_str("&amp;"),
            '<' => safe.push_str("&lt;"),
            '>' => safe.push_str("&gt;"),
            '"' => safe.push_str("&quot;"),
            '\'' => safe.push_str("&#39;"),
            c => safe.push(c),
        }
    }
    safe
}

const STYLE: &str = r#"
:root {
  color-scheme: light dark;
  --page: #f6f6f4; --card: #ffffff; --ink: #1c1c1a; --quiet: #62625c;
  --line: #d9d9d3; --well: #ecece8;
  --failed: #b3261e; --unexpected: #8a5a00; --blessed: #1f5fa8; --passed: #2f6f3a;
}
@media (prefers-color-scheme: dark) {
  :root {
    --page: #141413; --card: #1e1e1c; --ink: #ecece6; --quiet: #a3a39b;
    --line: #36362f; --well: #282825;
    --failed: #ff8a80; --unexpected: #f0b849; --blessed: #8ab8f5; --passed: #86cf93;
  }
}
* { box-sizing: border-box; }
body {
  margin: 0; padding: 24px 16px 64px; background: var(--page); color: var(--ink);
  font: 15px/1.5 system-ui, -apple-system, "Segoe UI", Roboto, sans-serif;
}
header, main { max-width: 1400px; margin: 0 auto; }
h1 { font-size: 24px; margin: 0 0 4px; }
h2 { font-size: 17px; margin: 0 0 8px; display: flex; flex-wrap: wrap; gap: 6px 12px; align-items: baseline; }
.tally { margin: 0 0 24px; color: var(--quiet); }
.result, .passes {
  background: var(--card); border: 1px solid var(--line); border-radius: 8px;
  padding: 16px; margin: 0 0 16px;
}
.verdict {
  font-size: 12px; font-weight: 700; letter-spacing: 0.04em; text-transform: uppercase;
  border: 1px solid currentColor; border-radius: 4px; padding: 1px 6px;
}
.failed .verdict { color: var(--failed); }
.unexpected .verdict { color: var(--unexpected); }
.blessed .verdict { color: var(--blessed); }
.passed .verdict { color: var(--passed); }
.family { font-size: 13px; font-weight: 400; color: var(--quiet); }
.detail { margin: 0 0 12px; overflow-wrap: anywhere; }
.source { margin: 8px 0 12px; color: var(--quiet); font-size: 13px; overflow-wrap: anywhere; }
table { border-collapse: collapse; font-variant-numeric: tabular-nums; }
th, td { text-align: left; padding: 3px 20px 3px 0; border-bottom: 1px solid var(--line); font-weight: 400; }
thead th { color: var(--quiet); font-size: 13px; }
td.number { text-align: right; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; font-size: 13px; }
tr.over td, tr.over th { color: var(--failed); font-weight: 600; }
.pictures { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 12px; }
@media (max-width: 900px) { .pictures { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
figure { margin: 0; }
figcaption { font-size: 13px; color: var(--quiet); margin: 0 0 4px; }
figure img, figure .none {
  display: block; width: 100%; height: auto; background: var(--well);
  border: 1px solid var(--line); border-radius: 4px; image-rendering: pixelated;
}
figure .none {
  aspect-ratio: 16 / 9; display: flex; align-items: center; justify-content: center;
  color: var(--quiet); font-size: 13px;
}
.passes table { width: 100%; }
.passes td { overflow-wrap: anywhere; }
"#;

/// A picture as an element: the file's bytes inside the page, or a box
/// that says there is none.
fn figure(page: &mut String, caption: &str, path: Option<&PathBuf>, absent: &str) {
    let picture = path.and_then(|path| std::fs::read(path).ok());
    let _ = write!(page, "<figure><figcaption>{}</figcaption>", escape(caption));
    match picture {
        Some(bytes) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let _ = write!(
                page,
                "<img alt=\"{}\" src=\"data:image/png;base64,{encoded}\">",
                escape(caption)
            );
        }
        None => {
            let _ = write!(page, "<div class=\"none\">{}</div>", escape(absent));
        }
    }
    page.push_str("</figure>\n");
}

/// One result that needs looking at: its verdict, what was found, its
/// numbers and its pictures.
fn section(page: &mut String, result: &GoldenResult, standing: Standing) {
    let _ = write!(
        page,
        "<section class=\"result {}\">\n<h2><span class=\"verdict\">{}</span><span>{} / {}</span><span class=\"family\">drawn on {}</span></h2>\n<p class=\"detail\">{}</p>\n",
        standing.class(),
        standing.word(),
        escape(&result.case),
        escape(&result.name),
        result.family,
        escape(&result.detail)
    );
    if let Some(measured) = &result.measured {
        page.push_str(
            "<table>\n<thead><tr><th>Measure</th><th>Found</th><th>Bound</th><th></th></tr></thead>\n<tbody>\n",
        );
        let within = measured.within(&result.bounds);
        let values = measured.values();
        for (at, (what, bound)) in result.bounds.named().into_iter().enumerate() {
            let (class, word) = if within[at] {
                ("", "within")
            } else {
                (" class=\"over\"", "over")
            };
            let _ = writeln!(
                page,
                "<tr{class}><th>{}</th><td class=\"number\">{:.4}</td><td class=\"number\">{bound}</td><td>{word}</td></tr>",
                escape(what),
                values[at]
            );
        }
        let _ = write!(
            page,
            "</tbody>\n</table>\n<p class=\"source\">Held to {}.</p>\n",
            escape(&result.bounds_from.to_string())
        );
    }
    let artefacts = &result.artefacts;
    let difference = match artefacts.difference_gain {
        Some(gain) => format!("Difference, times {gain}"),
        None => "Difference".to_owned(),
    };
    // A result that never got as far as a picture has none to show.
    if *artefacts == super::Artefacts::default() {
        page.push_str("</section>\n");
        return;
    }
    page.push_str("<div class=\"pictures\">\n");
    figure(page, "Drawn", artefacts.got.as_ref(), "Not written");
    figure(
        page,
        "Reference",
        artefacts.reference.as_ref(),
        "No reference",
    );
    figure(
        page,
        &difference,
        artefacts.difference.as_ref(),
        "Nothing to compare",
    );
    figure(
        page,
        "FLIP, black the same",
        artefacts.flip.as_ref(),
        "Nothing to compare",
    );
    page.push_str("</div>\n</section>\n");
}

/// The page for `results`: the failures first, then the unexpected
/// passes, then what was blessed, each with its pictures; the plain
/// passes last, in a table.
pub fn failure_page(results: &[GoldenResult]) -> String {
    let mut ordered: Vec<(Standing, &GoldenResult)> = results
        .iter()
        .map(|result| (Standing::of(result), result))
        .collect();
    ordered.sort_by(|a, b| (a.0, &a.1.case, &a.1.name).cmp(&(b.0, &b.1.case, &b.1.name)));
    let count = |standing| ordered.iter().filter(|(one, _)| *one == standing).count();
    let tally = format!(
        "{} failed, {} passed unexpectedly, {} blessed, {} passed",
        count(Standing::Failed),
        count(Standing::Unexpected),
        count(Standing::Blessed),
        count(Standing::Passed)
    );

    let mut page = String::new();
    let _ = write!(
        page,
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Goldens: {tally}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<header>\n<h1>Goldens</h1>\n<p class=\"tally\">{tally}</p>\n</header>\n<main>\n"
    );
    for (standing, result) in &ordered {
        if *standing != Standing::Passed {
            section(&mut page, result, *standing);
        }
    }
    if count(Standing::Passed) > 0 {
        page.push_str(
            "<section class=\"passes passed\">\n<h2><span class=\"verdict\">Passed</span></h2>\n<table>\n<thead><tr><th>Golden</th><th>Drawn on</th><th>Found</th></tr></thead>\n<tbody>\n",
        );
        for (_, result) in ordered.iter().filter(|(one, _)| *one == Standing::Passed) {
            let _ = writeln!(
                page,
                "<tr><th>{} / {}</th><td>{}</td><td>{}</td></tr>",
                escape(&result.case),
                escape(&result.name),
                result.family,
                escape(&result.detail)
            );
        }
        page.push_str("</tbody>\n</table>\n</section>\n");
    }
    page.push_str("</main>\n</body>\n</html>\n");
    page
}

/// Writes [`failure_page`] as `index.html` in `dir`, making the folder.
/// Where it was written.
pub fn write_failure_page(dir: &Path, results: &[GoldenResult]) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir)
        .map_err(|error| format!("could not make {}: {error}", dir.display()))?;
    let path = dir.join("index.html");
    std::fs::write(&path, failure_page(results))
        .map_err(|error| format!("could not write {}: {error}", path.display()))?;
    Ok(path)
}
