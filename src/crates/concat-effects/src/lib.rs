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
//! From package format 2 a package that draws the picture is a shader
//! alone, sampling the compositor's linear working space as it is (see
//! [`shader`]), and what its `fixtures.toml` pins is colours - `[[probe]]`,
//! a colour in and the colour out, checked on a GPU - where a chain's pins
//! strings (`[[case]]`). Sound stays FFmpeg's.
//!
//! The document is untouched by any of this: a clip still stores
//! `{ id, params, enabled }`, and an id the catalogue does not know is
//! skipped at render time.

pub mod catalogue;
pub mod cube;
pub mod expr;
pub mod filters;
pub mod looks;
pub mod manifest;
pub mod shader;
pub mod template;

mod builtins {
    include!(concat!(env!("OUT_DIR"), "/builtins.rs"));
}

pub use catalogue::{At, Catalogue, Fixture, Package, Probe, package_folders, package_stamp};
pub use manifest::{CardSettings, FORMAT, Kind, Manifest, Param, ParamType, Space};
pub use shader::{Contract, Shader, TransitionShader};

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
            keys: Default::default(),
        }
    }

    /// A filter below full intensity is split, looked at on one copy and
    /// blended back; at full intensity it is the bare fragment. An effect
    /// never mixes: intensity is a filter's word.
    #[test]
    fn a_filter_mixes_by_its_intensity_and_an_effect_does_not() {
        // A format 1 look of someone's own: the built-in looks are shaders
        // alone.
        let mut catalogue = Catalogue::builtin().clone();
        catalogue
            .add(
                Package::from_sources(
                    "[effect]\nid = \"alice.warm\"\nname = \"Warm\"\nkind = \"filter\"\n\
                     [ffmpeg]\nchain = \"colortemperature=temperature=4600:pl=1\"\n",
                    None,
                    None,
                )
                .expect("loads"),
            )
            .expect("a new id");
        let full = catalogue.video_chain(&[applied("alice.warm", &[])]);
        assert_eq!(full, "colortemperature=temperature=4600:pl=1");
        let half = catalogue.video_chain(&[
            applied("concat.sepia", &[]),
            applied("alice.warm", &[("intensity", 50.0)]),
        ]);
        assert!(half.starts_with("colorchannelmixer"), "{half}");
        assert!(
            half.ends_with(
                "split[m1a][m1b];[m1b]colortemperature=temperature=4600:pl=1[m1c];\
                 [m1a][m1c]blend=all_mode=normal:all_opacity=0.500"
            ),
            "{half}"
        );
        let effect = catalogue.video_chain(&[applied("concat.sepia", &[("intensity", 50.0)])]);
        assert!(!effect.contains("blend"), "{effect}");
    }

    #[test]
    fn a_folder_package_with_a_table_loads_and_names_its_file() {
        let dir = std::env::temp_dir().join(format!("concat-lut-{}", std::process::id()));
        let folder = dir.join("test.table");
        std::fs::create_dir_all(&folder).expect("temp dir");
        std::fs::write(
            folder.join("effect.toml"),
            "[effect]\nid = \"test.table\"\nname = \"Table\"\nkind = \"filter\"\n\n[lut]\nfile = \"look.cube\"\n\n[ffmpeg]\nchain = \"lut3d=file={lut}\"\n\n[wgsl]\nentry = \"effect.wgsl\"\n",
        )
        .expect("manifest");
        std::fs::write(
            folder.join("effect.wgsl"),
            "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(lut(c.rgb), c.a); }",
        )
        .expect("shader");
        let mut cube = String::from("LUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    cube.push_str(&format!("{r} {g} {b}\n"));
                }
            }
        }
        std::fs::write(folder.join("look.cube"), cube).expect("cube");

        let mut catalogue = Catalogue::new();
        let errors = catalogue.load_dir(&dir);
        assert!(errors.is_empty(), "{errors:?}");
        let package = catalogue.get("test.table").expect("loaded");
        assert_eq!(package.lut().map(|lut| lut.size), Some(2));
        let chain = package
            .ffmpeg_fragment(&BTreeMap::new(), 0)
            .expect("renders")
            .expect("has a chain");
        assert!(
            chain.starts_with("lut3d=file='") && chain.ends_with("look.cube'"),
            "{chain}"
        );
        assert_eq!(
            catalogue.shader_passes(&[AppliedFilter::new("test.table")], None)[0]
                .lut
                .as_ref()
                .map(|l| l.size),
            Some(2)
        );
        let _ = std::fs::remove_dir_all(&dir);
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

    /// The same rule for a package drawn in light, whose pinned outputs
    /// are colours rather than chains.
    #[test]
    fn every_scene_linear_package_probes_its_default_and_every_slider_bound() {
        let mut gaps = Vec::new();
        let mut linear = 0;
        for package in Catalogue::builtin().packages() {
            let drawn = package.shader().is_some() || package.transition().is_some();
            if !package.manifest.scene_linear() || !drawn {
                continue;
            }
            linear += 1;
            let has = |at: At| {
                package
                    .probes
                    .iter()
                    .any(|probe| probe.at == at && probe.params.is_empty())
            };
            if !has(At::Default) {
                gaps.push(format!("{}: no default probe", package.id()));
            }
            if !package.manifest.params.is_empty() {
                if !has(At::Min) {
                    gaps.push(format!("{}: no min probe", package.id()));
                }
                if !has(At::Max) {
                    gaps.push(format!("{}: no max probe", package.id()));
                }
            }
        }
        assert!(linear >= 1, "no package is drawn in light yet");
        assert!(gaps.is_empty(), "\n{}", gaps.join("\n"));
    }

    /// A compound knob's dotted keys reach its shader: a wheel's puck and a
    /// curve's points go through `resolve` with the knobs, and keys the
    /// package does not own stay behind.
    #[test]
    fn a_compound_knob_is_resolved_whole() {
        let package = Catalogue::builtin()
            .get("concat.color-wheels")
            .expect("a built-in");
        let set: BTreeMap<String, f64> = [
            ("lift.x", 0.5),
            ("lift.m", -0.25),
            ("lift.q", 9.0),
            ("stray", 1.0),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        let values = package.resolve(&set);
        assert_eq!(values.get("lift.x"), Some(&0.5));
        assert_eq!(values.get("lift.m"), Some(&-0.25));
        assert!(!values.contains_key("lift.q") && !values.contains_key("stray"));
        let bounds = package.params_at(At::Max);
        assert_eq!(
            bounds.get("gain.m"),
            Some(&1.0),
            "a wheel's master at its bound"
        );
        assert_eq!(bounds.get("gain.x"), Some(&0.0), "its puck in the middle");
    }

    const EXPOSED: &str = "format = 2\n[effect]\nid = \"a.lift\"\nname = \"Lift\"\nkind = \"effect\"\n\
        [[param]]\nkey = \"stops\"\nlabel = \"Stops\"\nmin = -2\nmax = 2\n\
        [wgsl]\nentry = \"effect.wgsl\"\n";
    const EXPOSE: &str = "struct Params { stops: f32 }\nfn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(exposure(c.rgb, params.stops), c.a); }";

    /// A probe pins what a shader draws, so it needs a shader, and may set
    /// only what the package declares.
    #[test]
    fn a_probe_sets_only_declared_knobs_of_a_shader() {
        let probe = |params: &str| {
            format!(
                "[[probe]]\nname = \"up\"\nparams = {{ {params} }}\ninput = [0.25, 0.25, 0.25, 1]\nexpect = [0.5, 0.5, 0.5, 1]\n"
            )
        };
        let package = Package::from_sources(EXPOSED, Some(&probe("stops = 1")), Some(EXPOSE))
            .expect("a probe on a shader loads");
        let pass = package
            .probe_pass(&package.probes[0])
            .expect("a shader's pass");
        assert_eq!(pass.values.get("stops"), Some(&1.0));
        assert_eq!(pass.intensity, 1.0);

        let error = Package::from_sources(EXPOSED, Some(&probe("gain = 1")), Some(EXPOSE))
            .expect_err("an undeclared knob")
            .to_string();
        assert!(error.contains("`gain`"), "{error}");
        let error = Package::from_sources(EXPOSED, Some(&probe("intensity = 50")), Some(EXPOSE))
            .expect_err("an effect has no intensity")
            .to_string();
        assert!(error.contains("`intensity`"), "{error}");
        let filter = EXPOSED.replace("kind = \"effect\"", "kind = \"filter\"");
        let package = Package::from_sources(&filter, Some(&probe("intensity = 50")), Some(EXPOSE))
            .expect("a filter's intensity is its mix");
        assert_eq!(
            package.probe_pass(&package.probes[0]).map(|p| p.intensity),
            Some(0.5)
        );

        let error = Package::from_sources(TINT, Some(&probe("hue = 1")), None)
            .expect_err("a chain has no shader to probe")
            .to_string();
        assert!(
            error.contains("no shader") || error.contains("has none"),
            "{error}"
        );
        let error = Package::from_sources(
            EXPOSED,
            Some(&probe("stops = 1").replace("[[probe]]", "[[probe]]\nprogress = 0.5")),
            Some(EXPOSE),
        )
        .expect_err("an effect has no cut")
        .to_string();
        assert!(error.contains("only a transition"), "{error}");
    }

    /// A transition's probe is a cut between two colours at one point of
    /// it, so it names both and the point.
    #[test]
    fn a_transition_probe_names_its_cut() {
        const CUT: &str = "format = 2\n[effect]\nid = \"a.cut\"\nname = \"Cut\"\nkind = \"transition\"\n[transition]\nentry = \"effect.wgsl\"\n";
        const MIX: &str = "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> { return mix(from_at(uv), to_at(uv), progress); }";
        let probe = "[[probe]]\ninput = [1, 1, 1, 1]\nto = [0, 0, 0, 1]\nprogress = 0.25\nexpect = [0.75, 0.75, 0.75, 1]\n";
        let package = Package::from_sources(CUT, Some(probe), Some(MIX)).expect("loads");
        let pass = package
            .probe_transition(&package.probes[0])
            .expect("a transition's combine");
        assert_eq!(pass.progress, 0.25);
        assert!(package.probe_pass(&package.probes[0]).is_none());
        let error = Package::from_sources(
            CUT,
            Some(&probe.replace("progress = 0.25\n", "")),
            Some(MIX),
        )
        .expect_err("no progress")
        .to_string();
        assert!(error.contains("`to` and `progress`"), "{error}");
    }

    /// A probe allows a fraction of the expected value, with a floor near
    /// zero, and nothing that is not a number.
    #[test]
    fn a_probe_holds_to_its_tolerance() {
        let probe = Probe {
            name: String::new(),
            at: At::Default,
            params: BTreeMap::new(),
            input: [0.0; 4],
            to: None,
            progress: None,
            expect: [2.0, 0.5, 0.0, 1.0],
            tolerance: 0.01,
        };
        assert!(probe.check([2.019, 0.4951, 0.0000999, 1.0]).is_ok());
        assert!(probe.check([2.03, 0.5, 0.0, 1.0]).is_err(), "1.5 % off");
        assert!(probe.check([2.0, 0.5, 0.0002, 1.0]).is_err(), "off zero");
        let error = probe
            .check([2.0, f32::NAN, 0.0, 1.0])
            .expect_err("not a number");
        assert!(error.contains("NaN"), "{error}");
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
            catalogue.video_chain(&[applied("box-blur", &[]), applied("black-white", &[])]),
            "boxblur=6:1,hue=s=0"
        );
        assert_eq!(
            catalogue.video_chain(&[applied("black-white", &[]), applied("box-blur", &[])]),
            "hue=s=0,boxblur=6:1"
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
        let chain = Catalogue::builtin()
            .video_chain(&[applied("concat.bokeh", &[]), applied("concat.bokeh", &[])]);
        assert!(chain.contains("[bka0]"), "was: {chain}");
        assert!(chain.contains("[bka1]"), "was: {chain}");
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
    fn a_chain_may_only_name_filters_that_touch_the_frame() {
        let with = |chain: &str| {
            Package::from_sources(
                &format!(
                    "[effect]\nid = \"a.b\"\nname = \"B\"\nkind = \"effect\"\n[[param]]\nkey = \"hue\"\nlabel = \"Hue\"\nmax = 360\n[ffmpeg]\nchain = {chain:?}\n"
                ),
                None,
                None,
            )
        };
        with("hue=h={round(hue)},curves=all='0/0,1/1',negate").expect("plain filters load");
        with("split[a][b];[a]gblur=sigma=2[c];[b][c]blend=all_mode=screen").expect("graphs load");
        for (chain, needle) in [
            ("movie=/etc/passwd[m];[m]hue=h={hue}", "`movie`"),
            ("hue=h={hue},drawtext=textfile=/etc/passwd", "`drawtext`"),
            ("frei0r=filter_name=/tmp/evil.so,hue=h={hue}", "`frei0r`"),
            ("hue=h={hue},sendcmd=f=/tmp/cmd", "`sendcmd`"),
            (
                "hue=h={hue},vidstabdetect=result=/tmp/out",
                "`vidstabdetect`",
            ),
            ("{hue}=1", "spelt out"),
            ("hue=h={hue},", "no name"),
        ] {
            let error = with(chain).expect_err(chain).to_string();
            assert!(error.contains(needle), "{chain}: {error}");
        }
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

    /// A package folder `id` under a fresh temp dir, holding `files`.
    fn scratch(id: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("concat-check-{}-{id}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let folder = dir.join(id);
        std::fs::create_dir_all(&folder).expect("mkdir");
        for (name, text) in files {
            std::fs::write(folder.join(name), text).expect("write");
        }
        folder
    }

    const TINT: &str = "[effect]\nid = \"alice.tint\"\nname = \"Tint\"\nkind = \"effect\"\n\
        [[param]]\nkey = \"hue\"\nlabel = \"Hue\"\nmax = 360\ndefault = 90\n\
        [ffmpeg]\nchain = \"hue=h={round(hue)}\"\n";

    #[test]
    fn a_sound_package_checks_clean() {
        let folder = scratch(
            "alice.tint",
            &[
                ("effect.toml", TINT),
                (
                    "fixtures.toml",
                    "[[case]]\nname = \"default\"\nchain = \"hue=h=90\"\n[[case]]\nat = \"max\"\nchain = \"hue=h=360\"\n",
                ),
            ],
        );
        let problems = Package::check_folder(&folder, Catalogue::builtin());
        assert!(problems.is_empty(), "{problems:?}");
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    #[test]
    fn check_folder_names_every_fault() {
        // A manifest that does not parse.
        let folder = scratch(
            "bob.broken",
            &[("effect.toml", "[effect]\nid = \"bob.broken\"\n")],
        );
        let problems = Package::check_folder(&folder, Catalogue::builtin());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("bob.broken") || problems[0].contains('?'),
            "{problems:?}"
        );
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());

        // A fixture that pins a chain the template does not render.
        let folder = scratch(
            "carol.pinned",
            &[
                ("effect.toml", &TINT.replace("alice.tint", "carol.pinned")),
                (
                    "fixtures.toml",
                    "[[case]]\nname = \"wrong\"\nchain = \"hue=h=0\"\n",
                ),
            ],
        );
        let problems = Package::check_folder(&folder, Catalogue::builtin());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(
            problems[0].contains("wrong") && problems[0].contains("hue=h=90"),
            "{problems:?}"
        );
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());

        // The built-ins' author, as an id and as an alias.
        let folder = scratch(
            "concat.sepia",
            &[("effect.toml", &TINT.replace("alice.tint", "concat.sepia"))],
        );
        let problems = Package::check_folder(&folder, Catalogue::builtin());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("author.name"), "{problems:?}");
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
        let folder = scratch(
            "frank.alias",
            &[(
                "effect.toml",
                &TINT.replace("alice.tint", "frank.alias").replace(
                    "kind = \"effect\"\n",
                    "kind = \"effect\"\naliases = [\"concat.sepia\"]\n",
                ),
            )],
        );
        let problems = Package::check_folder(&folder, Catalogue::builtin());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("concat.sepia"), "{problems:?}");
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());

        // An id the catalogue it would join already answers to.
        let folder = scratch(
            "grace.twice",
            &[("effect.toml", &TINT.replace("alice.tint", "grace.twice"))],
        );
        let mut taken = Catalogue::new();
        assert!(taken.load_dir(folder.parent().unwrap()).is_empty());
        let problems = Package::check_folder(&folder, &taken);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("already taken"), "{problems:?}");
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());

        // A shader that does not compile, found at load and not on the GPU.
        let folder = scratch(
            "dave.shady",
            &[
                (
                    "effect.toml",
                    "[effect]\nid = \"dave.shady\"\nname = \"Shady\"\nkind = \"effect\"\n[wgsl]\nentry = \"effect.wgsl\"\n",
                ),
                (
                    "effect.wgsl",
                    "fn effect(uv: vec2<f32>) -> vec4<f32> { return sample(uv) + ; }",
                ),
            ],
        );
        let problems = Package::check_folder(&folder, Catalogue::builtin());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("shader"), "{problems:?}");

        // A folder that is not there at all.
        let missing = folder.parent().unwrap().join("nobody.home");
        let problems = Package::check_folder(&missing, Catalogue::builtin());
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("effect.toml"), "{problems:?}");
        let _ = std::fs::remove_dir_all(folder.parent().unwrap());
    }

    #[test]
    fn the_stamp_moves_with_the_folder_and_only_then() {
        let folder = scratch(
            "hank.still",
            &[("effect.toml", &TINT.replace("alice.tint", "hank.still"))],
        );
        let dir = folder.parent().unwrap().to_path_buf();
        assert_eq!(package_stamp(&dir.join("nowhere")), 0);
        let first = package_stamp(&dir);
        assert_ne!(first, 0);
        assert_eq!(package_stamp(&dir), first, "nothing changed");
        // A file in a package: its size changes even when the clock has
        // not ticked over.
        std::fs::write(
            folder.join("fixtures.toml"),
            "[[case]]\nchain = \"hue=h=90\"\n",
        )
        .expect("write");
        let second = package_stamp(&dir);
        assert_ne!(second, first, "a file was added");
        std::fs::write(
            folder.join("fixtures.toml"),
            "[[case]]\nchain = \"hue=h=90\"\n\n",
        )
        .expect("write");
        let third = package_stamp(&dir);
        assert_ne!(third, second, "a file grew");
        // Clutter beside the packages is not a package.
        std::fs::write(dir.join("notes.txt"), "x").expect("write");
        assert_eq!(package_stamp(&dir), third, "a file beside the packages");
        // A second package.
        std::fs::create_dir_all(dir.join("hank.other")).expect("mkdir");
        std::fs::write(dir.join("hank.other").join("effect.toml"), TINT).expect("write");
        assert_ne!(package_stamp(&dir), third, "a folder was added");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn package_folders_lists_only_folders_with_a_manifest() {
        let folder = scratch(
            "erin.one",
            &[("effect.toml", &TINT.replace("alice.tint", "erin.one"))],
        );
        let dir = folder.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(dir.join("notes")).expect("mkdir");
        std::fs::write(dir.join("README.txt"), "not a package").expect("write");
        let folders = package_folders(&dir).expect("lists");
        assert_eq!(folders, vec![folder.clone()]);
        assert!(package_folders(&dir.join("nowhere")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
