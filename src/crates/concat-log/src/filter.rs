// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Which lines are written: a level for Concat's own crates, one for the
//! crates underneath, and any number of exceptions by module.
//!
//! The split is the point. At Debug, wgpu, naga, Slint and fontdb bury a
//! run in lines nobody asked for, so "show me more" has to mean more of
//! what Concat says about itself; the libraries stay at warnings unless
//! they are named. `CONCAT_LOG` spells all of it:
//!
//! ```text
//! CONCAT_LOG=debug                       Concat's crates at debug
//! CONCAT_LOG=debug,concat_media=trace    and one of them at trace
//! CONCAT_LOG=info,wgpu_core=debug        a library, by name
//! CONCAT_LOG=all=debug                   everything, libraries included
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};

use log::LevelFilter;

/// What a run writes down; see the module.
#[derive(Debug)]
pub struct Filter {
    /// Concat's own crates. The one level that changes while a run goes
    /// on - Settings has a picker for it - hence the atomic.
    own: AtomicUsize,
    /// Every other crate.
    others: LevelFilter,
    /// Exceptions, longest module path first, so the most exact one that
    /// matches is the one that decides.
    rules: Vec<(String, LevelFilter)>,
    /// Whether a level for Concat's crates was named outright, which is
    /// what makes a run's environment outrank the remembered setting.
    named: bool,
}

impl Default for Filter {
    fn default() -> Self {
        Filter {
            own: AtomicUsize::new(LevelFilter::Info as usize),
            others: LevelFilter::Warn,
            rules: Vec::new(),
            named: false,
        }
    }
}

impl Filter {
    /// The filter a spec asks for; see the module for the spelling. A word
    /// that is neither a level nor `module=level` is skipped, and returned
    /// so the caller can say so once the log is open.
    pub fn parse(spec: &str) -> (Filter, Vec<String>) {
        let mut filter = Filter::default();
        let mut skipped = Vec::new();
        for word in spec
            .split(',')
            .map(str::trim)
            .filter(|word| !word.is_empty())
        {
            match word.split_once('=') {
                None => match level_named(word) {
                    Some(level) => {
                        filter.own = AtomicUsize::new(level as usize);
                        filter.named = true;
                    }
                    None => skipped.push(word.to_owned()),
                },
                Some((module, level)) => match (module.trim(), level_named(level)) {
                    ("all" | "*", Some(level)) => {
                        filter.own = AtomicUsize::new(level as usize);
                        filter.others = level;
                        filter.named = true;
                    }
                    (module, Some(level)) if !module.is_empty() => {
                        filter.rules.push((module.replace('-', "_"), level));
                    }
                    _ => skipped.push(word.to_owned()),
                },
            }
        }
        filter
            .rules
            .sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        (filter, skipped)
    }

    /// Whether the spec named a level for Concat's own crates.
    pub fn named(&self) -> bool {
        self.named
    }

    /// The level Concat's own crates are written at.
    pub fn own(&self) -> LevelFilter {
        from_usize(self.own.load(Ordering::Relaxed))
    }

    /// Sets the level Concat's own crates are written at.
    pub fn set_own(&self, level: LevelFilter) {
        self.own.store(level as usize, Ordering::Relaxed);
    }

    /// The most any module may say: what the facade is told, so a line
    /// nothing would write is dropped before it is formatted.
    pub fn ceiling(&self) -> LevelFilter {
        self.rules
            .iter()
            .map(|(_, level)| *level)
            .chain([self.own(), self.others])
            .max()
            .unwrap_or(LevelFilter::Off)
    }

    /// Whether a line from `target` at `level` is written.
    pub fn allows(&self, target: &str, level: log::Level) -> bool {
        for (module, allowed) in &self.rules {
            if within(target, module) {
                return level <= *allowed;
            }
        }
        if own(target) {
            level <= self.own()
        } else {
            level <= self.others
        }
    }
}

/// Whether a target is one of Concat's own crates.
fn own(target: &str) -> bool {
    target.starts_with("concat")
}

/// Whether `target` is `module` or a module inside it: `wgpu` covers
/// `wgpu::backend` and not `wgpu_core`.
fn within(target: &str, module: &str) -> bool {
    target
        .strip_prefix(module)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with("::"))
}

fn from_usize(level: usize) -> LevelFilter {
    LevelFilter::iter()
        .find(|filter| *filter as usize == level)
        .unwrap_or(LevelFilter::Info)
}

/// A level by the name the environment and the preferences use: `error`,
/// `warn`, `info`, `debug`, `trace` or `off`, in any case.
pub fn level_named(name: &str) -> Option<LevelFilter> {
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "error" => LevelFilter::Error,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        "off" => LevelFilter::Off,
        _ => return None,
    })
}

/// The name [`level_named`] reads back.
pub fn level_name(level: LevelFilter) -> &'static str {
    match level {
        LevelFilter::Error => "error",
        LevelFilter::Warn => "warn",
        LevelFilter::Info => "info",
        LevelFilter::Debug => "debug",
        LevelFilter::Trace => "trace",
        LevelFilter::Off => "off",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Level;

    #[test]
    fn with_nothing_said_concat_is_at_info_and_the_libraries_at_warnings() {
        let (filter, skipped) = Filter::parse("");
        assert!(skipped.is_empty());
        assert!(!filter.named());
        assert!(filter.allows("concat_media::decode", Level::Info));
        assert!(!filter.allows("concat_media::decode", Level::Debug));
        assert!(filter.allows("wgpu_core::device", Level::Warn));
        assert!(!filter.allows("wgpu_core::device", Level::Info));
        assert_eq!(filter.ceiling(), LevelFilter::Info);
    }

    #[test]
    fn a_bare_level_is_concats_and_leaves_the_libraries_quiet() {
        let (filter, _) = Filter::parse("debug");
        assert!(filter.named());
        assert!(filter.allows("concat::studio", Level::Debug));
        assert!(!filter.allows("naga::front", Level::Debug));
        assert_eq!(filter.ceiling(), LevelFilter::Debug);
    }

    #[test]
    fn a_module_is_named_and_the_most_exact_name_decides() {
        let (filter, skipped) =
            Filter::parse("info, concat_media=trace, concat_media::pool=warn, wgpu=debug, loud");
        assert_eq!(skipped, ["loud"]);
        assert!(filter.allows("concat_media::decode", Level::Trace));
        assert!(!filter.allows("concat_media::pool", Level::Info));
        assert!(filter.allows("wgpu::backend", Level::Debug));
        // `wgpu` is not `wgpu_core`.
        assert!(!filter.allows("wgpu_core::device", Level::Debug));
        assert_eq!(filter.ceiling(), LevelFilter::Trace);
    }

    #[test]
    fn all_raises_the_libraries_too() {
        let (filter, _) = Filter::parse("all=debug");
        assert!(filter.allows("naga::front", Level::Debug));
        assert!(filter.allows("concat", Level::Debug));
    }

    #[test]
    fn the_picker_moves_concats_level_alone() {
        let (filter, _) = Filter::parse("");
        filter.set_own(LevelFilter::Trace);
        assert_eq!(filter.own(), LevelFilter::Trace);
        assert!(filter.allows("concat_export", Level::Trace));
        assert!(!filter.allows("fontdb", Level::Info));
    }

    #[test]
    fn level_names_round_trip() {
        for level in LevelFilter::iter() {
            assert_eq!(level_named(level_name(level)), Some(level));
        }
        assert_eq!(level_named(" DEBUG "), Some(LevelFilter::Debug));
        assert_eq!(level_named("loud"), None);
        assert_eq!(level_named(""), None);
    }
}
