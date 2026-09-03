// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Effect packages.
//!
//! An effect is a folder: `effect.toml` declares its id, its parameters and
//! one backend, and `fixtures.toml` pins what it produces. The built-in
//! packages under `packages/` are compiled into the binary; user packages
//! load from a directory at run time. The window shows the parameters, the
//! engine runs the backend, and nothing in Rust names an individual effect.
//!
//! Two backends exist. `[ffmpeg]` is a filter-chain template with
//! `{expression}` slots ([`template`], [`expr`]), run inside the decoder's
//! filtergraph where the chains have always run. `[wgsl]` is a shader,
//! declared here and run by the compositor.
//!
//! The document is untouched by any of this: a clip still stores
//! `{ id, params, enabled }`, and an id the catalogue does not know is
//! skipped at render time.

pub mod catalogue;
pub mod expr;
pub mod manifest;
pub mod shader;
pub mod template;

mod builtins {
    include!(concat!(env!("OUT_DIR"), "/builtins.rs"));
}

pub use catalogue::{At, Catalogue, Fixture, Package};
pub use manifest::{Kind, Manifest, Param, ParamType};
pub use shader::Shader;

/// Why a package could not be loaded.
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// The manifest, a template or a fixture is wrong.
    #[error("{id}: {message}")]
    Invalid {
        /// The package's id, or `?` when the manifest did not parse far
        /// enough to have one.
        id: String,
        /// What is wrong.
        message: String,
    },
    /// A package file could not be read.
    #[error("{path}: {message}")]
    Io {
        /// The file.
        path: std::path::PathBuf,
        /// The system's reason.
        message: String,
    },
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use concat_project::model::AppliedFilter;

    use super::*;

    fn applied(id: &str, params: &[(&str, f64)]) -> AppliedFilter {
        AppliedFilter {
            id: id.to_owned(),
            params: params
                .iter()
                .map(|(key, value)| ((*key).to_owned(), *value))
                .collect(),
            enabled: true,
        }
    }

    /// A filter below full intensity is split, looked at on one copy and
    /// blended back; at full intensity it is the bare fragment. An effect
    /// never mixes: intensity is a filter's word.
    #[test]
    fn a_filter_mixes_by_its_intensity_and_an_effect_does_not() {
        let catalogue = Catalogue::builtin();
        let full = catalogue.video_chain(&[applied("concat.warm", &[])]);
        assert_eq!(full, "colortemperature=temperature=4600");
        let half = catalogue.video_chain(&[
            applied("concat.sepia", &[]),
            applied("concat.warm", &[("intensity", 50.0)]),
        ]);
        assert!(half.starts_with("colorchannelmixer"), "{half}");
        assert!(
            half.ends_with(
                "split[m1a][m1b];[m1b]colortemperature=temperature=4600[m1c];\
                 [m1a][m1c]blend=all_mode=normal:all_opacity=0.500"
            ),
            "{half}"
        );
        let effect = catalogue.video_chain(&[applied("concat.sepia", &[("intensity", 50.0)])]);
        assert!(!effect.contains("blend"), "{effect}");
    }

    #[test]
    fn every_built_in_package_loads_and_its_fixtures_pass() {
        let catalogue = Catalogue::builtin();
        assert!(catalogue.packages().count() >= 28);
        let failures: Vec<String> = catalogue
            .packages()
            .flat_map(|package| package.check_fixtures())
            .collect();
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn every_ffmpeg_package_pins_its_default_and_every_slider_bound() {
        // A package without fixtures is a package nobody has looked at.
        let mut gaps = Vec::new();
        for package in Catalogue::builtin().packages() {
            if package.manifest.ffmpeg.is_none() {
                continue;
            }
            let has = |at: At| {
                package
                    .fixtures
                    .iter()
                    .any(|case| case.at == at && case.params.is_empty())
            };
            if !has(At::Default) {
                gaps.push(format!("{}: no default case", package.id()));
            }
            if !package.manifest.params.is_empty() {
                if !has(At::Min) {
                    gaps.push(format!("{}: no min case", package.id()));
                }
                if !has(At::Max) {
                    gaps.push(format!("{}: no max case", package.id()));
                }
            }
        }
        assert!(gaps.is_empty(), "\n{}", gaps.join("\n"));
    }

    #[test]
    fn built_in_ids_are_namespaced_and_bare_aliases_still_resolve() {
        let catalogue = Catalogue::builtin();
        for package in catalogue.packages() {
            assert!(package.id().starts_with("concat."), "{}", package.id());
        }
        assert_eq!(catalogue.get("glow").map(Package::id), Some("concat.glow"));
        assert_eq!(
            catalogue.get("concat.glow").map(Package::id),
            Some("concat.glow")
        );
        assert!(catalogue.get("from-the-future").is_none());
    }

    #[test]
    fn stacked_effects_join_with_commas_in_applied_order() {
        let catalogue = Catalogue::builtin();
        assert_eq!(
            catalogue.video_chain(&[applied("gaussian-blur", &[]), applied("black-white", &[])]),
            "gblur=sigma=10.0,hue=s=0"
        );
        assert_eq!(
            catalogue.video_chain(&[applied("black-white", &[]), applied("gaussian-blur", &[])]),
            "hue=s=0,gblur=sigma=10.0"
        );
    }

    #[test]
    fn a_bypassed_entry_contributes_nothing_and_consumes_no_index() {
        let catalogue = Catalogue::builtin();
        let mut sepia = applied("sepia", &[]);
        sepia.enabled = false;
        assert_eq!(
            catalogue.video_chain(&[
                applied("invert", &[]),
                sepia.clone(),
                applied("black-white", &[])
            ]),
            "negate,hue=s=0"
        );
        let skipped = catalogue.video_chain(&[sepia, applied("mirror", &[])]);
        assert!(skipped.contains("[mirl0]"), "was: {skipped}");
    }

    #[test]
    fn stacking_one_labelled_effect_twice_keeps_its_graph_labels_distinct() {
        let chain = Catalogue::builtin().video_chain(&[applied("glow", &[]), applied("glow", &[])]);
        assert!(chain.contains("[glowa0]"), "was: {chain}");
        assert!(chain.contains("[glowa1]"), "was: {chain}");
    }

    #[test]
    fn unknown_ids_and_wrong_kinds_are_skipped() {
        let catalogue = Catalogue::builtin();
        assert_eq!(
            catalogue.video_chain(&[applied("from-the-future", &[]), applied("invert", &[])]),
            "negate"
        );
        // An audio filter in the video list is not a video effect.
        assert_eq!(
            catalogue.video_chain(&[applied("bass", &[]), applied("invert", &[])]),
            "negate"
        );
        assert_eq!(
            catalogue.audio_chain(&[applied("invert", &[]), applied("echo", &[])]),
            "aecho=0.8:0.85:250:0.40"
        );
        assert_eq!(catalogue.video_chain(&[]), "");
    }

    #[test]
    fn stray_parameter_keys_are_dropped_and_missing_ones_default() {
        let catalogue = Catalogue::builtin();
        assert_eq!(
            catalogue.video_chain(&[applied("sharpen", &[("amount", 2.0), ("bogus", 99.0)])]),
            "unsharp=5:5:2.00:5:5:0"
        );
        assert_eq!(
            catalogue.video_chain(&[applied("shake", &[("amount", 20.0)])]),
            "crop=iw-40:ih-40:20+20*sin(t*13):20+20*cos(t*17)"
        );
        assert_eq!(
            catalogue.audio_chain(&[applied("echo", &[("delay", 0.5)])]),
            "aecho=0.8:0.85:500:0.40"
        );
    }

    #[test]
    fn a_user_package_loads_from_a_directory_and_a_broken_one_is_reported() {
        let dir = std::env::temp_dir().join(format!("concat-effects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let good = dir.join("alice.tint");
        std::fs::create_dir_all(&good).expect("mkdir");
        std::fs::write(
            good.join("effect.toml"),
            r#"
            [effect]
            id = "alice.tint"
            name = "Tint"
            kind = "effect"
            [[param]]
            key = "hue"
            label = "Hue"
            max = 360
            default = 90
            [ffmpeg]
            chain = "hue=h={round(hue)}"
            "#,
        )
        .expect("write");
        let bad = dir.join("bob.broken");
        std::fs::create_dir_all(&bad).expect("mkdir");
        std::fs::write(bad.join("effect.toml"), "[effect]\nid = \"bob.broken\"\n").expect("write");

        let mut catalogue = Catalogue::new();
        let errors = catalogue.load_dir(&dir);
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].to_string().contains("bob.broken") || errors[0].to_string().contains("?")
        );
        assert_eq!(
            catalogue.video_chain(&[applied("alice.tint", &[("hue", 45.0)])]),
            "hue=h=45"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_duplicate_id_is_refused() {
        let mut catalogue = Catalogue::new();
        let package = || {
            Package::from_sources(
                "[effect]\nid = \"a.b\"\nname = \"B\"\nkind = \"effect\"\n[ffmpeg]\nchain = \"negate\"\n",
                None,
                None,
            )
            .expect("loads")
        };
        catalogue.add(package()).expect("first");
        assert!(catalogue.add(package()).is_err());
        assert_eq!(catalogue.video_chain(&[applied("a.b", &[])]), "negate");
        let _ = BTreeMap::<String, f64>::new();
    }
}
