// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A golden from end to end, on made-up pictures and in folders of its
//! own: what passes, what fails and by which gate, what is written and
//! when. No GPU is asked for anything.

use std::path::{Path, PathBuf};

use super::synth::{self, Draw, FIELD, PATCH, Quantise, scene, turn, with_noise, with_rect};
use super::*;

/// The case every test here is of.
const CASE: &str = "looks";

/// A case's folder, a folder for artefacts and a bless log, all in a
/// temporary folder that goes when the test does.
struct Bench {
    root: tempfile::TempDir,
}

impl Bench {
    fn new() -> Self {
        Self {
            root: tempfile::tempdir().unwrap(),
        }
    }

    fn cases(&self) -> PathBuf {
        self.root.path().join("cases")
    }

    fn case_dir(&self) -> PathBuf {
        self.cases().join(CASE)
    }

    fn artefacts(&self) -> PathBuf {
        self.root.path().join("out").join(CASE).join("golden")
    }

    fn log(&self) -> PathBuf {
        self.root.path().join("out").join("bless.md")
    }

    /// A run that only compares.
    fn checking(&self, family: Family) -> Golden {
        Golden::new(&self.case_dir(), family, &self.artefacts(), Mode::check())
    }

    /// A run told to bless this case.
    fn blessing(&self, family: Family, force: bool) -> Golden {
        let mode =
            Mode::from_settings(CASE, &self.log(), Some(CASE), force.then_some("1"), None).unwrap();
        Golden::new(&self.case_dir(), family, &self.artefacts(), mode)
    }

    /// Blesses `picture` as the golden `name`, drawn on lavapipe.
    fn bless(&self, name: &str, picture: &Rgba8Image) {
        let result = self.blessing(Family::Lavapipe, false).check(name, picture);
        assert!(result.blessed && result.passed, "{}", result.detail);
    }

    /// A bench whose golden `title` is the scene.
    fn with_scene() -> (Self, Rgba8Image) {
        let bench = Self::new();
        let reference = scene(Draw::default());
        bench.bless("title", &reference);
        (bench, reference)
    }

    /// Gives `title` an override for WARP that loosens the percentile to
    /// 0.18, for edges measured at 0.14.
    fn override_for_warp(&self) {
        let mut metadata = Metadata::load(&self.case_dir()).unwrap();
        let over = bounds::Override {
            bounds: bounds::Changed {
                flip_p99: Some(0.18),
                ..bounds::Changed::default()
            },
            measured: Some(Measured {
                flip_mean: 0.0053,
                flip_p99: 0.14,
                delta_e: 0.61,
            }),
            why: Some("WARP covers the edges in quarters of a pixel".to_owned()),
        };
        metadata
            .goldens
            .entry("title".to_owned())
            .or_default()
            .overrides
            .insert(Family::Warp, over);
        metadata.save(&self.case_dir()).unwrap();
        // What was written is a file the rules accept.
        Metadata::load(&self.case_dir()).unwrap();
    }
}

/// Every file under `folder`, relative to it, in order.
fn files(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(next) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                folders.push(path);
            } else {
                let relative = path.strip_prefix(folder).unwrap();
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    found.sort();
    found
}

/// Which of the three gates `result` is within: the mean, the
/// percentile, the colour.
fn within(result: &GoldenResult) -> [bool; 3] {
    result.measured.unwrap().within(&result.bounds)
}

#[test]
fn the_same_picture_passes_with_every_number_nought_and_writes_nothing() {
    let (bench, reference) = Bench::with_scene();
    let before = (files(&bench.cases()), files(&bench.artefacts()));
    let result = bench.checking(Family::Lavapipe).check("title", &reference);
    assert!(result.ok(), "{}", result.detail);
    assert!(!result.blessed && !result.unexpected_pass);
    assert_eq!(
        result.measured,
        Some(Measured {
            flip_mean: 0.0,
            flip_p99: 0.0,
            delta_e: 0.0
        })
    );
    assert_eq!(result.bounds, Bounds::DEFAULT);
    assert_eq!(result.bounds_from, Source::Default);
    assert_eq!(result.artefacts, Artefacts::default());
    assert_eq!((files(&bench.cases()), files(&bench.artefacts())), before);
    assert_eq!(
        result.detail,
        "FLIP's mean 0.0000 of 0.06, its 99th percentile 0.0000 of 0.08, the worst block's ΔE 0.00 of 2.5 at (0, 0); the default bounds"
    );
}

#[test]
fn renders_that_differ_by_rounding_noise_or_dither_pass_the_default_bounds() {
    let (bench, reference) = Bench::with_scene();
    let golden = bench.checking(Family::Metal);
    let draw = |quantise| {
        scene(Draw {
            quantise,
            ..Draw::default()
        })
    };
    for (what, got) in [
        (
            "one level of noise in every pixel",
            with_noise(&reference, 1, 1, 7),
        ),
        (
            "two levels of noise in every pixel",
            with_noise(&reference, 2, 1, 7),
        ),
        ("truncated", draw(Quantise::Truncate)),
        ("dithered", draw(Quantise::Dither)),
        (
            "moved 1/64 of a pixel",
            scene(Draw {
                offset: (1.0 / 64.0, 1.0 / 64.0),
                ..Draw::default()
            }),
        ),
    ] {
        assert_ne!(got, reference, "{what}");
        let result = golden.check("title", &got);
        assert!(result.ok(), "{what}: {}", result.detail);
        assert!(result.measured.unwrap().flip_mean > 0.0, "{what}");
    }
}

#[test]
fn a_field_whose_hue_turns_at_the_same_lightness_fails_on_colour() {
    let (bench, reference) = Bench::with_scene();
    let (from, to) = (synth::FIELD_COLOUR, [153, 118, 101]);
    // The same lightness to a fiftieth of an L*, and the same luma to a
    // tenth of a level: a measure of brightness alone sees no change.
    let lightness = |rgb| colour::srgb_to_lab(rgb)[0];
    assert!((lightness(from) - lightness(to)).abs() < 0.02);
    let luma = |rgb: [u8; 3]| {
        0.2126 * f64::from(rgb[0]) + 0.7152 * f64::from(rgb[1]) + 0.0722 * f64::from(rgb[2])
    };
    assert!((luma(from) - luma(to)).abs() < 0.1);

    let result = bench
        .checking(Family::Lavapipe)
        .check("title", &with_rect(&reference, FIELD, turn(to)));
    assert!(!result.passed);
    let measured = result.measured.unwrap();
    assert!(measured.delta_e > 6.0, "{}", measured.delta_e);
    assert!(!within(&result)[2]);
    assert!(
        result.detail.contains("the worst block's ΔE is 6.4"),
        "{}",
        result.detail
    );
    // The mean, pooled over the frame, does not see it at all.
    assert!(within(&result)[0]);
}

#[test]
fn a_patch_off_its_hue_under_a_frame_of_noise_is_lost_to_both_of_flips_gates_and_caught_by_colour()
{
    let (bench, reference) = Bench::with_scene();
    let golden = bench.checking(Family::Metal);
    // The noise by itself passes: it is what another adapter's rounding
    // looks like.
    let noisy = with_noise(&reference, 2, 1, 7);
    assert!(golden.check("title", &noisy).ok());
    // One 8 × 8 block of a flat field goes from tan to mauve. Its pixels
    // are each as wrong as FLIP would fail, but they are 64 of 57,600 and
    // hold under a hundredth of the frame's error, so the percentile
    // never reaches them.
    let got = with_rect(&noisy, PATCH, |c| [c[0] + 3, c[1] - 2, c[2] + 11]);
    let result = golden.check("title", &got);
    assert_eq!(within(&result), [true, true, false], "{}", result.detail);
    assert!(!result.passed);
    assert_eq!(result.worst_block, Some([PATCH.0, PATCH.1]));
    assert!(result.measured.unwrap().delta_e > 6.0);
}

#[test]
fn a_percentile_loosened_for_one_adapters_edges_leaves_a_field_off_its_hue_still_failing() {
    let (bench, reference) = Bench::with_scene();
    bench.override_for_warp();
    // Edges covered in quarters of a pixel where the reference's are in
    // sixteenths: past the default percentile, inside WARP's.
    let coarse = scene(Draw {
        samples: 2,
        ..Draw::default()
    });
    let elsewhere = bench.checking(Family::Gpu).check("title", &coarse);
    assert_eq!(
        within(&elsewhere),
        [true, false, true],
        "{}",
        elsewhere.detail
    );
    assert_eq!(elsewhere.bounds_from, Source::Default);
    let on_warp = bench.checking(Family::Warp).check("title", &coarse);
    assert!(on_warp.ok(), "{}", on_warp.detail);
    assert_eq!(on_warp.bounds.flip_p99, 0.18);
    assert!(
        on_warp.detail.ends_with(
            "the bounds of the override for warp: WARP covers the edges in quarters of a pixel"
        ),
        "{}",
        on_warp.detail
    );
    // The same picture with a field's hue turned a little: the loosened
    // percentile lets FLIP pass it, and the colour gate does not.
    let turned = with_rect(&coarse, FIELD, turn([151, 119, 98]));
    let result = bench.checking(Family::Warp).check("title", &turned);
    assert_eq!(within(&result), [true, true, false], "{}", result.detail);
    assert!(!result.passed);
    // And the reference itself was not touched by any of it.
    assert_eq!(
        bench
            .checking(Family::Lavapipe)
            .check("title", &reference)
            .measured,
        Some(Measured {
            flip_mean: 0.0,
            flip_p99: 0.0,
            delta_e: 0.0
        })
    );
}

#[test]
fn a_picture_moved_two_pixels_fails_on_flip() {
    let (bench, _) = Bench::with_scene();
    let moved = scene(Draw {
        offset: (2.0, 0.0),
        ..Draw::default()
    });
    let result = bench.checking(Family::Lavapipe).check("title", &moved);
    assert!(!result.passed);
    assert_eq!(within(&result)[..2], [false, false], "{}", result.detail);
    assert!(
        result.detail.starts_with("FLIP's mean is 0.0"),
        "{}",
        result.detail
    );
    assert!(
        result
            .detail
            .contains(", over 0.06, and FLIP's 99th percentile is 0.9"),
        "{}",
        result.detail
    );
    // Most of the way to as different as two pictures can be, where a
    // person looks: at the edges.
    assert!(result.measured.unwrap().flip_p99 > 0.8);
}

#[test]
fn one_white_pixel_fails_the_percentile_and_neither_the_mean_nor_the_colour() {
    let (bench, reference) = Bench::with_scene();
    let speck = with_rect(&reference, (250, 90, 1, 1), |_| [255, 255, 255]);
    let result = bench.checking(Family::Lavapipe).check("title", &speck);
    assert_eq!(within(&result), [true, false, true], "{}", result.detail);
    assert!(!result.passed);
    let measured = result.measured.unwrap();
    // A six-hundredth of the mean's bound, and five times the
    // percentile's.
    assert!(measured.flip_mean < 0.0002, "{}", measured.flip_mean);
    assert!(measured.flip_p99 > 0.4, "{}", measured.flip_p99);
}

#[test]
fn a_picture_of_another_size_fails_saying_both_sizes() {
    let (bench, _) = Bench::with_scene();
    let small = synth::flat(160, 90, [10, 20, 30]);
    let result = bench.checking(Family::Lavapipe).check("title", &small);
    assert!(!result.passed);
    assert_eq!(
        result.detail,
        "the picture is 160 × 90 and its reference 320 × 180"
    );
    assert_eq!(result.measured, None);
    // What was drawn and the reference are there to look at; there is
    // nothing to take a difference of.
    assert!(result.artefacts.got.is_some() && result.artefacts.reference.is_some());
    assert!(result.artefacts.difference.is_none() && result.artefacts.flip.is_none());
}

#[test]
fn a_picture_whose_alpha_is_not_its_references_fails_saying_where() {
    let bench = Bench::new();
    let mut pixels = scene(Draw::default()).pixels().to_vec();
    // A clear window in the reference.
    for y in 40..60 {
        for x in 100..140 {
            pixels[(y * 320 + x) * 4 + 3] = 100;
        }
    }
    let reference = Rgba8Image::new(320, 180, pixels.clone()).unwrap();
    bench.bless("window", &reference);
    let golden = bench.checking(Family::Lavapipe);
    assert!(golden.check("window", &reference).ok());
    // A level of rounding in the alpha is let through.
    pixels[(40 * 320 + 100) * 4 + 3] = 101;
    let rounded = Rgba8Image::new(320, 180, pixels.clone()).unwrap();
    assert!(golden.check("window", &rounded).ok());
    // Two pixels fully opaque are not.
    pixels[(50 * 320 + 120) * 4 + 3] = 255;
    pixels[(50 * 320 + 121) * 4 + 3] = 255;
    let result = golden.check("window", &Rgba8Image::new(320, 180, pixels).unwrap());
    assert!(!result.passed);
    assert!(
        result.detail.starts_with("its alpha is not its reference's: 2 pixels are more than 1 level apart, the furthest 155 levels at (120, 50)"),
        "{}",
        result.detail
    );
}

#[test]
fn a_reference_that_is_not_a_picture_fails_saying_so() {
    let bench = Bench::new();
    let path = Golden::reference(&bench.case_dir(), "title");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"<<<<<<< a merge went wrong here").unwrap();
    let result = bench
        .checking(Family::Lavapipe)
        .check("title", &scene(Draw::default()));
    assert!(!result.passed);
    assert!(
        result.detail.contains("does not read: not a PNG"),
        "{}",
        result.detail
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        b"<<<<<<< a merge went wrong here"
    );
}

#[test]
fn a_missing_reference_fails_and_writes_nothing_to_the_case() {
    let bench = Bench::new();
    std::fs::create_dir_all(bench.case_dir()).unwrap();
    std::fs::write(bench.case_dir().join("case.json"), "{}").unwrap();
    let result = bench
        .checking(Family::Lavapipe)
        .check("title", &scene(Draw::default()));
    assert!(!result.passed && !result.blessed);
    assert_eq!(
        result.detail,
        "no reference for title; bless it with CONCAT_CORPUS_BLESS=looks"
    );
    // The case's folder is as it was: no reference, no golden.json, no
    // golden folder. And nothing was blessed, so there is no log.
    assert_eq!(files(&bench.cases()), ["looks/case.json"]);
    assert!(!bench.log().exists());
    // What was drawn is with the artefacts, to be looked at before it is
    // blessed.
    assert_eq!(files(&bench.artefacts()), ["title.got.png"]);
    assert_eq!(result.artefacts.reference, None);
}

#[test]
fn a_run_told_to_bless_another_case_writes_nothing_either() {
    let bench = Bench::new();
    let mode = Mode::from_settings(CASE, &bench.log(), Some("cut,title"), None, None).unwrap();
    assert!(!mode.blesses());
    let golden = Golden::new(&bench.case_dir(), Family::Metal, &bench.artefacts(), mode);
    let result = golden.check("title", &scene(Draw::default()));
    assert!(!result.passed);
    assert!(files(&bench.cases()).is_empty());
}

#[test]
fn blessing_writes_the_reference_and_the_run_after_it_passes() {
    let bench = Bench::new();
    let picture = scene(Draw::default());
    let result = bench
        .blessing(Family::Metal, false)
        .check("title", &picture);
    assert!(result.passed && result.blessed && result.ok());
    assert!(
        result.detail.contains("because no reference for title"),
        "{}",
        result.detail
    );
    assert_eq!(
        files(&bench.cases()),
        ["looks/golden/golden.json", "looks/golden/title.png"]
    );
    assert_eq!(Rgba8Image::read_png(&result.reference).unwrap(), picture);
    // The file says what the reference was drawn on.
    let metadata = Metadata::load(&bench.case_dir()).unwrap();
    assert_eq!(metadata.entry("title").blessed_on, Some(Family::Metal));
    let log = std::fs::read_to_string(bench.log()).unwrap();
    assert!(log.starts_with("# References blessed\n"), "{log}");
    assert!(log.contains("## looks/title, drawn on metal"), "{log}");
    assert!(log.contains("- old: none"), "{log}");

    let after = bench.checking(Family::Metal).check("title", &picture);
    assert!(after.ok() && !after.blessed, "{}", after.detail);
}

#[test]
fn blessing_leaves_a_reference_that_passes_alone_unless_it_is_forced() {
    let (bench, reference) = Bench::with_scene();
    let log = std::fs::read_to_string(bench.log()).unwrap();
    let path = Golden::reference(&bench.case_dir(), "title");
    let bytes = std::fs::read(&path).unwrap();
    // Within the bounds, though not the same picture.
    let noisy = with_noise(&reference, 1, 1, 7);

    let result = bench.blessing(Family::Metal, false).check("title", &noisy);
    assert!(result.passed && !result.blessed, "{}", result.detail);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_to_string(bench.log()).unwrap(), log);
    assert_eq!(
        Metadata::load(&bench.case_dir())
            .unwrap()
            .entry("title")
            .blessed_on,
        Some(Family::Lavapipe)
    );

    let result = bench.blessing(Family::Metal, true).check("title", &noisy);
    assert!(result.passed && result.blessed, "{}", result.detail);
    assert_eq!(Rgba8Image::read_png(&path).unwrap(), noisy);
    let log = std::fs::read_to_string(bench.log()).unwrap();
    assert!(
        log.contains("- because: CONCAT_CORPUS_BLESS_FORCE is set"),
        "{log}"
    );
    assert_eq!(
        Metadata::load(&bench.case_dir())
            .unwrap()
            .entry("title")
            .blessed_on,
        Some(Family::Metal)
    );
}

#[test]
fn blessing_a_reference_that_fails_rewrites_it_and_lists_the_old_the_new_and_the_difference() {
    let (bench, reference) = Bench::with_scene();
    bench.override_for_warp();
    let moved = scene(Draw {
        offset: (2.0, 0.0),
        ..Draw::default()
    });
    let result = bench
        .blessing(Family::Lavapipe, false)
        .check("title", &moved);
    assert!(result.passed && result.blessed, "{}", result.detail);
    assert_eq!(Rgba8Image::read_png(&result.reference).unwrap(), moved);
    // The old reference is kept beside the difference, for the log to
    // point at.
    let old = result.artefacts.reference.clone().unwrap();
    assert_eq!(Rgba8Image::read_png(&old).unwrap(), reference);
    let log = std::fs::read_to_string(bench.log()).unwrap();
    let entry = log.rsplit("\n## ").next().unwrap();
    assert!(
        entry.starts_with("looks/title, drawn on lavapipe"),
        "{entry}"
    );
    assert!(entry.contains("- because: FLIP's mean is 0.0"), "{entry}");
    for (label, path) in [
        ("new", Some(result.reference.clone())),
        ("old", Some(old)),
        ("difference", result.artefacts.difference.clone()),
        ("FLIP", result.artefacts.flip.clone()),
    ] {
        let path = path.unwrap();
        assert!(path.is_file(), "{label}");
        assert!(
            entry.contains(&format!("- {label}: {}", path.display())),
            "{label}: {entry}"
        );
    }
    // The log has both blessings, the first one's entry untouched.
    assert_eq!(log.matches("\n## ").count(), 2);
    // The override a person wrote is still in the file.
    let metadata = Metadata::load(&bench.case_dir()).unwrap();
    assert!(
        metadata
            .entry("title")
            .overrides
            .contains_key(&Family::Warp)
    );
    assert!(bench.checking(Family::Lavapipe).check("title", &moved).ok());
}

#[test]
fn blessing_is_refused_outright_where_ci_is_set() {
    let log = Path::new("bless.md");
    for bless in ["1", "all", CASE, "some-other-case"] {
        for ci in ["true", "1"] {
            let error = Mode::from_settings(CASE, log, Some(bless), None, Some(ci)).unwrap_err();
            assert_eq!(
                error,
                "CONCAT_CORPUS_BLESS is set and so is CI: references are blessed on a person's machine and looked at, never in CI"
            );
        }
    }
    // CI by itself is an ordinary run, and so is a variable set to
    // nothing.
    for (bless, ci) in [
        (None, Some("true")),
        (Some(""), Some("true")),
        (Some("0"), Some("true")),
        (None, None),
    ] {
        let mode = Mode::from_settings(CASE, log, bless, Some("1"), ci).unwrap();
        assert!(!mode.blesses());
    }
    assert!(!Mode::check().blesses());
}

#[test]
fn blessing_is_only_of_the_cases_the_variable_names() {
    let log = Path::new("bless.md");
    let blesses = |bless: &str| {
        Mode::from_settings(CASE, log, Some(bless), None, Some("false"))
            .unwrap()
            .blesses()
    };
    assert!(blesses("1"));
    assert!(blesses("all"));
    assert!(blesses("looks"));
    assert!(blesses("cut, looks ,title"));
    assert!(!blesses("cut,title"));
    assert!(!blesses("look"));
    assert!(!blesses("looks-again"));
}

#[test]
fn a_pass_far_inside_what_its_override_measured_is_flagged_as_unexpected() {
    let (bench, reference) = Bench::with_scene();
    bench.override_for_warp();
    // WARP now draws the reference to the byte: whatever the override
    // was for has been fixed.
    let result = bench.checking(Family::Warp).check("title", &reference);
    assert!(result.passed && result.unexpected_pass);
    assert!(!result.ok());
    assert_eq!(
        result.detail,
        "it passes, unexpectedly: FLIP's 99th percentile is 0.0000, inside the default bound of 0.08 without the override for warp (bound 0.18, measured 0.1400): remove the override"
    );
    let check = result.to_check();
    assert!(!check.passed);
    assert_eq!(check.name, "title looks as its golden does");
    assert_eq!(check.detail, result.detail);
    // It is shown as a failure is, so there are pictures to look at.
    assert!(result.artefacts.got.is_some() && result.artefacts.flip.is_some());
    // The other families have no override to be stale.
    assert!(
        bench
            .checking(Family::Metal)
            .check("title", &reference)
            .ok()
    );
}

#[test]
fn a_failure_leaves_four_pictures_and_says_where() {
    let (bench, reference) = Bench::with_scene();
    let speck = with_rect(&reference, (250, 90, 3, 3), |_| [255, 255, 255]);
    let result = bench.checking(Family::Lavapipe).check("title", &speck);
    assert!(!result.passed);
    assert_eq!(
        files(&bench.artefacts()),
        [
            "title.difference.png",
            "title.flip.png",
            "title.got.png",
            "title.reference.png"
        ]
    );
    let read = |path: &Option<PathBuf>| Rgba8Image::read_png(path.as_ref().unwrap()).unwrap();
    assert_eq!(read(&result.artefacts.got), speck);
    assert_eq!(read(&result.artefacts.reference), reference);
    // The difference is black but for the speck, which is at full scale.
    let difference = read(&result.artefacts.difference);
    let lit: Vec<usize> = difference
        .pixels()
        .chunks_exact(4)
        .enumerate()
        .filter(|(_, pixel)| pixel[..3] != [0, 0, 0])
        .map(|(at, _)| at)
        .collect();
    assert_eq!(lit.len(), 9);
    assert_eq!(lit[0], 90 * 320 + 250);
    assert!(result.artefacts.difference_gain.unwrap() >= 1);
    assert!(difference.pixels().contains(&255));
    // FLIP's map is dark away from the speck and bright on it.
    let flip = read(&result.artefacts.flip);
    let pixel = |x: usize, y: usize| &flip.pixels()[(y * 320 + x) * 4..][..3];
    assert!(pixel(20, 170).iter().all(|&level| level < 8));
    assert!(pixel(251, 91).iter().any(|&level| level > 200));
}

#[test]
fn a_name_that_is_not_a_files_or_a_golden_json_that_breaks_its_rules_is_a_failure_blessing_or_not()
{
    let (bench, reference) = Bench::with_scene();
    let before = files(&bench.cases());
    for golden in [
        bench.checking(Family::Metal),
        bench.blessing(Family::Metal, true),
    ] {
        let result = golden.check("../title", &reference);
        assert!(!result.passed && !result.blessed);
        assert!(
            result.detail.contains("is not a golden's name"),
            "{}",
            result.detail
        );
    }
    assert_eq!(files(&bench.root.path().join("cases")), before);

    let path = Metadata::path(&bench.case_dir());
    let text = r#"{ "goldens": { "title": { "overrides": { "warp": { "bounds": { "flipP99": 0.5 } } } } } }"#;
    std::fs::write(&path, text).unwrap();
    for golden in [
        bench.checking(Family::Metal),
        bench.blessing(Family::Metal, true),
    ] {
        let result = golden.check("title", &reference);
        assert!(!result.passed && !result.blessed);
        assert!(
            result.detail.ends_with("golden.json: title's override for warp has no \"why\": an override says why its family draws the golden differently"),
            "{}",
            result.detail
        );
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

#[test]
fn references_over_their_budget_or_out_of_step_with_golden_json_are_named() {
    let (bench, reference) = Bench::with_scene();
    bench.bless("mask", &with_noise(&reference, 40, 1, 3));
    let size = |name: &str| {
        std::fs::metadata(Golden::reference(&bench.case_dir(), name))
            .unwrap()
            .len()
    };
    let (title, mask) = (size("title"), size("mask"));
    // The scene is a few kilobytes; forty levels of noise in every pixel
    // does not compress, and is over the budget as the blessing said.
    assert!(title < 8 * 1024, "{title}");
    assert!(mask > budget::REFERENCE_BYTES, "{mask}");

    let found = audit(&bench.cases(), Budget::default()).unwrap();
    assert_eq!(found.references.len(), 2);
    assert_eq!(found.total, title + mask);
    assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
    assert!(
        found.problems[0].contains("mask.png"),
        "{:?}",
        found.problems
    );
    assert!(found.problems[0].contains("over the 49152 a reference may be"));

    // Room for each, and not for both.
    let roomy = Budget {
        reference: mask,
        total: title + mask,
    };
    assert!(audit(&bench.cases(), roomy).unwrap().problems.is_empty());
    let tight = Budget {
        reference: mask,
        total: title + mask - 1,
    };
    let found = audit(&bench.cases(), tight).unwrap();
    assert_eq!(found.problems.len(), 1, "{:?}", found.problems);
    assert!(found.problems[0].contains("bytes together, over the"));

    // A reference nothing names, a name with no reference, a file that is
    // neither, and a reference that is not a picture.
    let folder = bench.case_dir().join("golden");
    std::fs::copy(folder.join("title.png"), folder.join("titel.png")).unwrap();
    std::fs::remove_file(folder.join("mask.png")).unwrap();
    std::fs::write(folder.join("notes.txt"), "to do").unwrap();
    std::fs::write(folder.join("title.png"), "not a picture").unwrap();
    let problems = audit(&bench.cases(), roomy).unwrap().problems;
    assert_eq!(problems.len(), 4, "{problems:?}");
    let has = |part: &str, rest: &str| {
        problems
            .iter()
            .any(|problem| problem.contains(part) && problem.contains(rest))
    };
    assert!(has("titel.png", "is a reference"), "{problems:?}");
    assert!(
        has("names mask, which has no reference", ""),
        "{problems:?}"
    );
    assert!(
        has("notes.txt", "is neither a reference nor golden.json"),
        "{problems:?}"
    );
    assert!(has("title.png", "not a PNG"), "{problems:?}");
}

#[test]
fn the_references_in_the_tree_are_within_their_budget_and_in_step_with_their_files() {
    let found = audit(&crate::cases_dir(), Budget::default()).unwrap();
    assert!(
        found.problems.is_empty(),
        "{} references, {} bytes:\n{}",
        found.references.len(),
        found.total,
        found.problems.join("\n")
    );
}

/// One result of every kind, in the order a run might make them: a pass,
/// an unexpected pass, a failure with its pictures, a name that is not
/// one, a golden with no reference, and a blessing.
fn one_of_each(bench: &Bench, reference: &Rgba8Image) -> Vec<GoldenResult> {
    bench.bless("mask", reference);
    bench.bless("glow", reference);
    bench.override_for_warp();
    let golden = bench.checking(Family::Warp);
    let speck = with_rect(reference, (250, 90, 3, 3), |_| [255, 255, 255]);
    vec![
        golden.check("mask", reference),
        golden.check("title", reference),
        golden.check("glow", &speck),
        golden.check("wipe <b>", reference),
        golden.check("none", reference),
        bench
            .blessing(Family::Warp, false)
            .check("fresh", reference),
    ]
}

#[test]
#[ignore = "writes a page to look at, and checks nothing"]
fn a_sample_page() {
    let (bench, reference) = Bench::with_scene();
    let mut results = one_of_each(&bench, &reference);
    let moved = scene(Draw {
        offset: (2.0, 0.0),
        ..Draw::default()
    });
    results.push(bench.checking(Family::Metal).check("mask", &moved));
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/corpus/golden-sample");
    let path = write_failure_page(&out, &results).unwrap();
    println!("{}", path.display());
}

#[test]
fn the_page_has_every_result_the_failures_first_and_nothing_from_outside_itself() {
    let (bench, reference) = Bench::with_scene();
    let results = one_of_each(&bench, &reference);
    let standing: Vec<(bool, bool, bool)> = results
        .iter()
        .map(|result| (result.passed, result.unexpected_pass, result.blessed))
        .collect();
    assert_eq!(
        standing,
        [
            (true, false, false),
            (true, true, false),
            (false, false, false),
            (false, false, false),
            (false, false, false),
            (true, false, true)
        ]
    );

    let path = write_failure_page(&bench.root.path().join("out"), &results).unwrap();
    assert_eq!(path.file_name().unwrap(), "index.html");
    let page = std::fs::read_to_string(&path).unwrap();
    assert_eq!(page, failure_page(&results));

    // Every result is on it, in order of how much it needs looking at.
    let at = |text: &str| {
        page.find(text)
            .unwrap_or_else(|| panic!("{text} is not on the page"))
    };
    let order = [
        at("<span>looks / glow</span>"),
        at("<span>looks / none</span>"),
        at("<span>looks / wipe &lt;b&gt;</span>"),
        at("<span>looks / title</span>"),
        at("<span>looks / fresh</span>"),
        at("<th>looks / mask</th>"),
    ];
    assert!(order.is_sorted(), "{order:?}");
    assert!(page.contains("3 failed, 1 passed unexpectedly, 1 blessed, 1 passed"));
    assert_eq!(
        page.matches("<span class=\"verdict\">Failed</span>")
            .count(),
        3
    );
    // The failure that was compared has its four pictures in the page
    // itself, and its numbers with the one that is over marked.
    let glow = &page[order[0]..order[1]];
    assert_eq!(glow.matches("<img alt=").count(), 4);
    assert_eq!(glow.matches("src=\"data:image/png;base64,").count(), 4);
    assert_eq!(glow.matches("<tr class=\"over\">").count(), 2);
    assert!(
        glow.contains("Difference, times 1</figcaption>"),
        "no gain in the caption"
    );
    // The one with no reference says so where the reference would be.
    assert!(page[order[1]..order[2]].contains("<div class=\"none\">No reference</div>"));
    // What a result says is text, never markup.
    assert!(!page.contains("<b>"));

    // Nothing is fetched and nothing runs: no address, no script, no
    // link, and every picture is a data URI.
    assert_eq!(
        page.matches("src=\"").count(),
        page.matches("src=\"data:image/png;base64,").count()
    );
    // (The pictures' own bytes aside, which are base64 and may spell
    // anything.)
    let mut text = String::new();
    for (at, part) in page.split("src=\"data:image/png;base64,").enumerate() {
        text.push_str(match at {
            0 => part,
            _ => part.split_once('"').unwrap().1,
        });
    }
    for outside in [
        "http:", "https:", "//", "<script", "<link", "@import", "url(", "<iframe", "href",
    ] {
        assert!(!text.contains(outside), "the page has {outside} in it");
    }
    // Light and dark, by the reader's own setting.
    assert!(page.contains("@media (prefers-color-scheme: dark)"));
    assert!(page.contains("color-scheme: light dark"));
}

#[test]
fn renders_of_one_frame_are_said_to_be_identical_or_how_far_apart_they_are() {
    let first = scene(Draw::default());
    let same = spread(&[first.clone(), first.clone(), first.clone()]).unwrap();
    assert!(same.identical);
    assert_eq!(
        (same.differing_pairs, same.differing_pixels, same.levels),
        (0, 0, 0)
    );
    assert_eq!(same.worst.values(), [0.0, 0.0, 0.0]);
    assert_eq!(same.summary(), "3 renders, byte for byte the same");

    // Three renders, one of them a level of noise off the other two: two
    // of the three pairs differ.
    let noisy = with_noise(&first, 1, 10, 7);
    let apart = spread(&[first.clone(), noisy.clone(), first.clone()]).unwrap();
    assert!(!apart.identical);
    assert_eq!(
        (apart.renders, apart.differing_pairs, apart.levels),
        (3, 2, 1)
    );
    assert!(apart.differing_pixels > 3000 && apart.differing_pixels < 8000);
    // The worst of the pairs is that one pair's, whichever way round it
    // was taken.
    let alone = judge(&first, &noisy).measured;
    for (worst, alone) in apart.worst.values().into_iter().zip(alone.values()) {
        assert!(
            worst >= alone && worst - alone < 1e-3,
            "{worst} against {alone}"
        );
    }
    assert!(alone.flip_mean > 0.0 && alone.flip_mean < Bounds::DEFAULT.flip_mean);
    assert!(
        apart
            .summary()
            .starts_with("3 renders, 2 pairs of them different: up to ")
    );

    assert!(spread(&[]).is_err());
    assert!(spread(std::slice::from_ref(&first)).is_err());
    let error = spread(&[first, synth::flat(8, 8, [0, 0, 0])]).unwrap_err();
    assert_eq!(error, "render 1 is 8 × 8 and the first 320 × 180");
}

#[test]
fn a_result_is_written_into_a_report_in_the_case_files_style() {
    let (bench, reference) = Bench::with_scene();
    let result = bench.checking(Family::Metal).check("title", &reference);
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["case"], "looks");
    assert_eq!(json["family"], "metal");
    assert_eq!(json["unexpectedPass"], false);
    assert_eq!(json["measured"]["flipP99"], 0.0);
    assert_eq!(json["bounds"]["deltaE"], 2.5);
    assert_eq!(json["boundsFrom"]["from"], "default");
    assert_eq!(json["worstBlock"], serde_json::json!([0, 0]));
    assert!(json["artefacts"]["got"].is_null());
}
