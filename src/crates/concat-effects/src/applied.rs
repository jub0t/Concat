// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A link to a package as the renderer runs it: the package's id, its
//! knobs' values, their keys by a share of the clip, and whether it is on.
//!
//! The document keeps its own effect (`concat_doc::model::Effect`), with
//! ids and keys at ticks; whoever hands a clip to the renderer makes one
//! of these from it, and nothing here knows of documents.

use std::collections::BTreeMap;

use concat_core::animate::{Ease, Key, Track};

/// One key of one knob.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ParamKey {
    /// Where in the clip, `0..=1`.
    pub at: f64,
    /// The knob's value there.
    pub value: f64,
    /// How this key is approached from the one before.
    pub ease: Ease,
}

/// A package applied to a clip.
#[derive(Clone, PartialEq, Debug)]
pub struct AppliedFilter {
    /// The package's id.
    pub id: String,
    /// Each knob's value; a knob left out is at the package's default.
    pub params: BTreeMap<String, f64>,
    /// Whether it runs; a link switched off is kept and draws nothing.
    pub enabled: bool,
    /// Each keyed knob's keys, in order of `at`.
    pub keys: BTreeMap<String, Vec<ParamKey>>,
}

impl AppliedFilter {
    /// A link to `id`, on, every knob at its default.
    pub fn new(id: impl Into<String>) -> AppliedFilter {
        AppliedFilter {
            id: id.into(),
            params: BTreeMap::new(),
            enabled: true,
            keys: BTreeMap::new(),
        }
    }

    /// Every knob's value at `at`, a share of the clip: a keyed knob at its
    /// keys' value there, the rest as set.
    pub fn params_at(&self, at: f64) -> BTreeMap<String, f64> {
        let mut params = self.params.clone();
        for (name, keys) in &self.keys {
            if keys.is_empty() {
                continue;
            }
            let track = Track::new(
                keys.iter()
                    .map(|key| Key {
                        at: key.at,
                        value: key.value,
                        ease: key.ease,
                    })
                    .collect(),
            );
            let rest = params.get(name).copied().unwrap_or(keys[0].value);
            params.insert(name.clone(), track.value_at(at, rest));
        }
        params
    }
}
