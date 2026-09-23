// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The GPU compositor against the CPU reference: the same plans, and the
//! pictures must agree. Every test skips on a machine without an adapter,
//! and says so.

use std::collections::BTreeMap;
use std::sync::Arc;

use concat_core::frame::Frame;
use concat_core::shader::{Lut, RevealMap, ShaderPass};
use concat_core::timeline::{Blend, Transform};

use super::*;
use crate::CpuCompositor;
use crate::metrics::ssim;
use crate::plan::{Crop, Transition, detached_clip};

fn gpu() -> Option<WgpuCompositor> {
    let compositor = WgpuCompositor::new();
    if compositor.is_none() {
        eprintln!("no usable GPU adapter; skipping");
    }
    compositor
}

fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Frame {
    let mut frame = Frame::transparent(width, height);
    frame.fill(rgba);
    frame
}

/// A picture with something in it everywhere: a diagonal gradient and a
/// dark bar, so a misplaced or mirrored picture is caught.
fn gradient(width: u32, height: u32) -> Frame {
    let mut frame = Frame::transparent(width, height);
    for y in 0..height {
        for x in 0..width {
            let r = (x * 255 / width.max(1)) as u8;
            let g = (y * 255 / height.max(1)) as u8;
            let bar = x > width / 3 && x < width / 2 && y > height / 4;
            let b = if bar { 20 } else { 200 };
            frame.set_pixel(x, y, [r, g, b, 255]);
        }
    }
    frame
}

fn layer(frame: Frame) -> PlannedLayer {
    PlannedLayer::picture(detached_clip(), Arc::new(frame))
}

fn plan(width: u32, height: u32, layers: Vec<PlannedLayer>) -> FramePlan {
    FramePlan {
        layers,
        ..FramePlan::empty(width, height)
    }
}

/// A real package's pass: `id` with `body` as its shader and `params` as
/// the manifest's parameter table, resolved to `values`.
fn package(
    id: &str,
    body: &str,
    params: &str,
    values: &[(&str, f64)],
    intensity: f32,
) -> ShaderPass {
    let manifest = concat_effects::Manifest::parse(&format!(
        "[effect]\nid = \"{id}\"\nname = \"Test\"\nkind = \"effect\"\n{params}\n[wgsl]\nentry = \"effect.wgsl\"\n"
    ))
    .expect("a manifest");
    let shader = concat_effects::Shader::compile(&manifest, body).expect("compiles");
    let values: BTreeMap<String, f64> = values
        .iter()
        .map(|(key, value)| ((*key).to_owned(), *value))
        .collect();
    shader.pass(&values, &manifest.params, intensity, None, None)
}

const INVERT: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(vec3<f32>(1.0) - c.rgb, c.a); }";

/// Both compositors draw `plan`; the pictures must agree to `least` by
/// SSIM, and every pixel's alpha must be opaque.
fn assert_parity(name: &str, plan: &FramePlan, least: f64) {
    let Some(mut gpu) = gpu() else { return };
    let expected = CpuCompositor.render(plan);
    let actual = gpu.render(plan);
    let score = ssim(&expected, &actual);
    eprintln!("parity {name}: ssim {score:.4}");
    assert!(
        score >= least,
        "{name}: cpu and gpu differ, ssim {score:.4} < {least}"
    );
    assert!(
        actual.pixels().chunks_exact(4).all(|px| px[3] == 255),
        "{name}: opaque"
    );
}

#[test]
fn empty_output_is_opaque_black() {
    let Some(mut gpu) = gpu() else { return };
    let frame = gpu.render(&FramePlan::empty(4, 4));
    assert_eq!(frame.pixel(0, 0), Some([0, 0, 0, 255]));
    assert_eq!(frame.pixel(3, 3), Some([0, 0, 0, 255]));
}

#[test]
fn plain_layers_match_the_cpu_reference_to_the_pixel() {
    let Some(mut gpu) = gpu() else { return };
    let mut top = layer(solid(3, 3, [0, 0, 255, 160]));
    top.transform = Transform {
        offset_x: 2.0 / 8.0,
        offset_y: 1.0 / 8.0,
        ..Transform::default()
    };
    let plan = plan(8, 8, vec![layer(solid(8, 8, [255, 0, 0, 255])), top]);
    let expected = CpuCompositor.render(&plan);
    let actual = gpu.render(&plan);
    for (index, (want, got)) in expected
        .pixels()
        .chunks_exact(4)
        .zip(actual.pixels().chunks_exact(4))
        .enumerate()
    {
        for channel in 0..4 {
            let difference = (i32::from(want[channel]) - i32::from(got[channel])).abs();
            assert!(
                difference <= 1,
                "pixel {index} channel {channel}: cpu {want:?} vs gpu {got:?}"
            );
        }
    }
}

/// A pass runs over the layer before it is placed: an invert shader
/// over a red frame composites cyan, and at half intensity the mix.
#[test]
fn a_pass_treats_the_layer_before_it_is_placed() {
    let Some(mut gpu) = gpu() else { return };
    let red = solid(4, 4, [255, 0, 0, 255]);
    let mut full = layer(red.clone());
    full.effects = vec![package("test.invert", INVERT, "", &[], 1.0)];
    let out = gpu.render(&plan(4, 4, vec![full]));
    assert_eq!(&out.pixels()[..3], &[0, 255, 255]);
    let mut half = layer(red);
    half.effects = vec![package("test.invert", INVERT, "", &[], 0.5)];
    let out = gpu.render(&plan(4, 4, vec![half]));
    let p = &out.pixels()[..3];
    assert!(
        p[0] > 120 && p[0] < 136 && p[1] > 120 && p[1] < 136,
        "{p:?}"
    );
}

/// A pass reads its table through `lut()`: a table that answers green
/// to every colour turns a red frame green, and a pass without one is
/// handed the identity and changes nothing.
#[test]
fn a_pass_samples_its_table_and_the_identity_without_one() {
    let Some(mut gpu) = gpu() else { return };
    let body = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(lut(c.rgb), c.a); }";
    let green = Lut::from_rgb(2, &[0.0, 1.0, 0.0].repeat(8)).expect("a table");
    let mut tabled = layer(solid(4, 4, [255, 0, 0, 255]));
    let mut pass = package("test.table", body, "", &[], 1.0);
    pass.lut = Some(Arc::new(green));
    tabled.effects = vec![pass];
    let out = gpu.render(&plan(4, 4, vec![tabled]));
    assert_eq!(&out.pixels()[..3], &[0, 255, 0]);
    let mut plain = layer(solid(4, 4, [255, 0, 0, 255]));
    plain.effects = vec![package("test.table", body, "", &[], 1.0)];
    let out = gpu.render(&plan(4, 4, vec![plain]));
    assert_eq!(&out.pixels()[..3], &[255, 0, 0]);
}

/// A treatment runs its passes over the stack beneath its track and
/// nothing above it, blended back by its strength, without the frame
/// leaving the GPU: an invert over a red ground under a blue quarter
/// turns the ground cyan and leaves the blue alone.
#[test]
fn a_treatment_treats_the_stack_beneath_its_track_only() {
    let Some(mut gpu) = gpu() else { return };
    let ground = layer(solid(8, 8, [255, 0, 0, 255]));
    let mut blue = layer(solid(4, 4, [0, 0, 255, 255]));
    blue.track = 2;
    blue.transform = Transform {
        offset_x: -2.0 / 8.0,
        offset_y: -2.0 / 8.0,
        ..Transform::default()
    };
    let treated = |strength: f32| FramePlan {
        treatments: vec![PlannedTreatment {
            track: 1,
            effects: vec![package("concat.invert", INVERT, "", &[], 1.0)],
            strength,
        }],
        ..plan(8, 8, vec![ground.clone(), blue.clone()])
    };
    let out = gpu.render(&treated(1.0));
    // Top-left is under the blue quarter; bottom-right is treated ground.
    assert_eq!(&out.pixels()[..3], &[0, 0, 255]);
    let last = out.pixels().len() - 4;
    assert_eq!(&out.pixels()[last..last + 3], &[0, 255, 255]);
    let out = gpu.render(&treated(0.5));
    let p = &out.pixels()[last..last + 3];
    assert!(
        p[0] > 120 && p[0] < 136 && p[1] > 120 && p[1] < 136,
        "{p:?}"
    );
    // The CPU reference agrees on the whole picture.
    assert_parity("treatment", &treated(0.5), 0.99);
}

/// The parity suite: one plan per thing a frame can ask for, drawn by
/// both compositors, alike by SSIM.
#[test]
fn every_kind_of_layer_matches_the_cpu_reference() {
    let sepia = include_str!("../../../concat-effects/packages/concat.sepia/effect.wgsl");
    let blur = include_str!("../../../concat-effects/packages/concat.box-blur/effect.wgsl");
    let radius =
        "[[param]]\nkey = \"radius\"\nlabel = \"Radius\"\nmin = 0\nmax = 20\ndefault = 4\n";

    assert_parity("plain", &plan(64, 48, vec![layer(gradient(64, 48))]), 0.999);

    // Fitted: a wide picture inside a square, letterboxed.
    assert_parity("fitted", &plan(64, 64, vec![layer(gradient(48, 16))]), 0.99);

    let mut placed = layer(gradient(32, 24));
    placed.transform = Transform {
        scale: 1.4,
        rotation: 30.0,
        offset_x: 0.1,
        offset_y: -0.05,
        stretch_x: 1.2,
        ..Transform::default()
    };
    assert_parity(
        "placed",
        &plan(
            64,
            64,
            vec![layer(solid(64, 64, [40, 40, 40, 255])), placed],
        ),
        0.98,
    );

    let mut cropped = layer(gradient(64, 48));
    cropped.crop = Crop::of([0.25, 0.1, 0.2, 0.3]);
    assert_parity("cropped", &plan(64, 48, vec![cropped]), 0.99);

    let mut flipped = layer(gradient(64, 48));
    flipped.flip_h = true;
    flipped.flip_v = true;
    assert_parity("flipped", &plan(64, 48, vec![flipped]), 0.999);

    let mut blended = layer(gradient(64, 48));
    blended.opacity = 0.5;
    blended.blend = Blend::Multiply;
    assert_parity(
        "blended",
        &plan(64, 48, vec![layer(gradient(64, 48)), blended]),
        0.99,
    );

    let mut masked = layer(gradient(64, 64));
    let mut mask = Frame::transparent(32, 32);
    for y in 0..32u32 {
        for x in 0..32u32 {
            let d = ((x as f32 - 16.0).powi(2) + (y as f32 - 16.0).powi(2)).sqrt();
            let a = ((16.0 - d) / 8.0).clamp(0.0, 1.0);
            mask.set_pixel(x, y, [255, 255, 255, (a * 255.0) as u8]);
        }
    }
    masked.mask = Some(Arc::new(mask));
    assert_parity(
        "masked",
        &plan(64, 64, vec![layer(solid(64, 64, [0, 60, 0, 255])), masked]),
        0.99,
    );

    let mut cut = layer(gradient(64, 48));
    cut.transitions = vec![
        Transition::FadeTo {
            colour: [0.0, 0.0, 0.0],
            amount: 0.5,
        },
        Transition::Wipe {
            uncovered: 0.6,
            from_right: false,
        },
    ];
    assert_parity("faded and wiped", &plan(64, 48, vec![cut]), 0.99);

    let mut toned = layer(gradient(64, 48));
    toned.effects = vec![package("concat.sepia", sepia, "", &[], 1.0)];
    assert_parity("sepia kernel", &plan(64, 48, vec![toned]), 0.99);

    let mut keyed = layer(gradient(64, 48));
    keyed.effects = vec![package("concat.sepia", sepia, "", &[], 0.4)];
    assert_parity(
        "sepia at a keyed intensity",
        &plan(64, 48, vec![keyed]),
        0.99,
    );

    // A blur over a cropped, flipped picture: the effects have to see it
    // made first, on both sides.
    let mut prepared = layer(gradient(64, 48));
    prepared.crop = Crop::of([0.1, 0.0, 0.1, 0.2]);
    prepared.flip_h = true;
    prepared.effects = vec![package(
        "concat.box-blur",
        blur,
        radius,
        &[("radius", 3.0)],
        1.0,
    )];
    assert_parity(
        "blur over a made picture",
        &plan(80, 60, vec![prepared]),
        0.98,
    );

    let mut under = layer(gradient(64, 48));
    under.track = 0;
    let mut over = layer(solid(16, 16, [0, 0, 255, 255]));
    over.track = 2;
    let treated = FramePlan {
        treatments: vec![PlannedTreatment {
            track: 1,
            effects: vec![package("concat.sepia", sepia, "", &[], 1.0)],
            strength: 0.6,
        }],
        ..plan(64, 48, vec![under, over])
    };
    assert_parity("treated stack", &treated, 0.99);
}

/// What cannot be drawn is not drawn, the same way on both sides: a
/// layer with a NaN opacity, one far outside the frame, one with no
/// picture, and an opacity past one is one.
#[test]
fn what_cannot_be_drawn_is_skipped_alike() {
    let Some(mut gpu) = gpu() else { return };
    let mut nan = layer(solid(8, 8, [255, 0, 0, 255]));
    nan.opacity = f32::NAN;
    let mut away = layer(solid(8, 8, [255, 0, 0, 255]));
    away.transform = Transform {
        offset_x: 40.0,
        ..Transform::default()
    };
    let mut none = layer(solid(8, 8, [255, 0, 0, 255]));
    none.source = None;
    let plan = plan(8, 8, vec![nan, away, none]);
    assert_eq!(gpu.render(&plan).pixel(4, 4), Some([0, 0, 0, 255]));
    assert_eq!(
        CpuCompositor.render(&plan).pixel(4, 4),
        Some([0, 0, 0, 255])
    );
    let mut over = layer(solid(8, 8, [0, 200, 0, 255]));
    over.opacity = 7.0;
    let plan = FramePlan {
        layers: vec![over],
        ..FramePlan::empty(8, 8)
    };
    assert_eq!(gpu.render(&plan).pixel(1, 1), Some([0, 200, 0, 255]));
}

/// A package that runs within the budget passes its trial; a device that
/// answers in time is kept.
#[test]
fn a_benign_package_survives_its_trial() {
    let Some(mut gpu) = gpu() else { return };
    let pass = package("test.trial", INVERT, "", &[], 1.0);
    gpu.trial(&pass, std::time::Duration::from_secs(5))
        .expect("an invert is quick");
    assert!(!gpu.is_dead());
    // The compositor is still good for a frame afterwards.
    let frame = gpu.render(&plan(4, 4, vec![layer(solid(4, 4, [0, 0, 255, 255]))]));
    assert_eq!(&frame.pixels()[..3], &[0, 0, 255]);
}

#[test]
fn output_size_changes_are_handled() {
    let Some(mut gpu) = gpu() else { return };
    let small = gpu.render(&plan(4, 4, vec![layer(solid(4, 4, [255, 255, 255, 255]))]));
    assert_eq!((small.width(), small.height()), (4, 4));
    let large = gpu.render(&plan(
        16,
        8,
        vec![layer(solid(16, 8, [255, 255, 255, 255]))],
    ));
    assert_eq!((large.width(), large.height()), (16, 8));
    assert_eq!(large.pixel(15, 7), Some([255, 255, 255, 255]));
}

/// A pass reads its title's reveal map through `reveal_order()`: the
/// first word's half of the canvas reads 0, the second word's half
/// reads 1, and a pass without one - the common case, any pass over
/// anything that is not a title - reads the identity, 0 everywhere.
#[test]
fn a_pass_reads_its_reveal_map_and_the_identity_everywhere() {
    let Some(mut gpu) = gpu() else { return };
    let body = "fn effect(uv: vec2<f32>) -> vec4<f32> { let r = reveal_order(uv); return vec4<f32>(r, r, r, 1.0); }";
    let map = RevealMap::from_rects(4, 4, &[(0, 0, 2, 4), (2, 0, 2, 4)]);
    let mut revealed = layer(solid(4, 4, [0, 0, 0, 255]));
    let mut pass = package("test.reveal", body, "", &[], 1.0);
    pass.reveal_map = Some(Arc::new(map));
    revealed.effects = vec![pass];
    let out = gpu.render(&plan(4, 4, vec![revealed]));
    let pixels = out.pixels();
    assert_eq!(pixels[0], 0, "{:?}", &pixels[..4]);
    assert_eq!(pixels[2 * 4], 255, "{:?}", &pixels[8..12]);

    let mut plain = layer(solid(4, 4, [0, 0, 0, 255]));
    plain.effects = vec![package("test.reveal", body, "", &[], 1.0)];
    let out = gpu.render(&plan(4, 4, vec![plain]));
    assert_eq!(&out.pixels()[..4], &[0, 0, 0, 255]);
}

/// A transition combines its two inputs through its shader: a trivial
/// dissolve over two solid colours must equal `from` at progress 0, `to`
/// at progress 1, and the exact half-and-half mix at progress 0.5. The
/// golden every packaged transition's own shader is measured against.
#[test]
fn a_transition_combines_its_two_inputs_by_progress() {
    let Some(mut gpu) = gpu() else { return };
    let manifest = concat_effects::Manifest::parse(
        "[effect]\nid = \"test.dissolve\"\nname = \"Dissolve\"\nkind = \"transition\"\n[transition]\nentry = \"effect.wgsl\"\n",
    )
    .expect("a manifest");
    let shader = concat_effects::TransitionShader::compile(
        &manifest,
        "fn transition(uv: vec2<f32>, progress: f32) -> vec4<f32> { return mix(from_at(uv), to_at(uv), progress); }",
    )
    .expect("compiles");
    let red = solid(4, 4, [255, 0, 0, 255]);
    let blue = solid(4, 4, [0, 0, 255, 255]);

    let mut at = |progress: f32| {
        let pass = shader.pass(&Default::default(), &[], progress, None);
        gpu.combine(4, 4, 0.0, &red, &blue, &pass)
            .expect("a GPU combine")
    };
    assert_eq!(&at(0.0).pixels()[..3], &[255, 0, 0], "all outgoing at 0");
    assert_eq!(&at(1.0).pixels()[..3], &[0, 0, 255], "all incoming at 1");
    assert_eq!(&at(0.5).pixels()[..3], &[128, 0, 128], "the exact half mix");
}

/// Every packaged effect and filter with a shader actually renders on
/// the GPU at its default settings - naga's validation at load catches
/// a broken shader's syntax and types, but only a real pipeline creation
/// and draw catches a binding or layout mistake.
#[test]
fn every_shader_package_renders_at_its_defaults() {
    let Some(mut gpu) = gpu() else { return };
    let source = solid(4, 4, [200, 120, 60, 255]);
    let catalogue = concat_effects::Catalogue::builtin();
    for package in catalogue.packages() {
        let Some(shader) = package.shader() else {
            continue;
        };
        let values = package.resolve(&Default::default());
        let pass = shader.pass(
            &values,
            &package.manifest.params,
            1.0,
            package.lut().cloned(),
            None,
        );
        let mut treated = layer(source.clone());
        treated.effects = vec![pass];
        let out = gpu.render(&plan(4, 4, vec![treated]));
        assert_eq!(out.pixels().len(), 4 * 4 * 4, "{}", package.id());
    }
}

/// A pass reads how long its own clip has been on screen through
/// `frame.clip_time` - the gap between the frame's own time and where the
/// clip begins on the timeline, not the timeline's absolute clock. A
/// clip starting at 2s, five seconds into the timeline, has been on
/// screen for exactly three.
#[test]
fn a_pass_reads_its_layers_clip_relative_time() {
    let Some(mut gpu) = gpu() else { return };
    let body = "fn effect(uv: vec2<f32>) -> vec4<f32> { if (abs(frame.clip_time - 3.0) < 0.001) { return vec4<f32>(0.0, 1.0, 0.0, 1.0); } return vec4<f32>(1.0, 0.0, 0.0, 1.0); }";
    let mut timed = layer(solid(4, 4, [0, 0, 0, 255]));
    timed.clip_start = concat_core::time::Rational::approximate(2.0).expect("a rational");
    timed.effects = vec![package("test.cliptime", body, "", &[], 1.0)];
    let mut p = plan(4, 4, vec![timed]);
    p.time = concat_core::time::Rational::approximate(5.0).expect("a rational");
    let out = gpu.render(&p);
    assert_eq!(
        &out.pixels()[..3],
        &[0, 255, 0],
        "clip_time should read 3.0"
    );
}

/// Every packaged transition's pipeline actually creates and runs on the
/// GPU, across its whole progress range - naga's validation at load
/// catches a broken shader's syntax and types, but only a real pipeline
/// creation catches a binding or layout mistake.
#[test]
fn every_packaged_transition_combines_across_its_progress_range() {
    let Some(mut gpu) = gpu() else { return };
    let red = solid(4, 4, [255, 0, 0, 255]);
    let blue = solid(4, 4, [0, 0, 255, 255]);
    let catalogue = concat_effects::Catalogue::builtin();
    for package in catalogue.packages() {
        if package.kind() != concat_effects::Kind::Transition {
            continue;
        }
        for progress in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let pass = catalogue
                .transition_pass(package.id(), &Default::default(), progress)
                .unwrap_or_else(|| panic!("{} has no transition pass", package.id()));
            gpu.combine(4, 4, 0.0, &red, &blue, &pass)
                .unwrap_or_else(|| {
                    panic!("{} failed to combine at progress {progress}", package.id())
                });
        }
    }
}
