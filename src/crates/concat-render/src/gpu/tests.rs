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
        // CI installs a software Vulkan driver so this suite runs there; a
        // machine that says it must run and has no adapter is a broken
        // setup, not a skip.
        assert!(
            std::env::var_os("CONCAT_REQUIRE_GPU").is_none(),
            "CONCAT_REQUIRE_GPU is set and no GPU adapter is usable"
        );
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
    // At half strength the ground is halfway between red and cyan in light:
    // a half of each channel, which BT.1886's 2.4 gamma stores at 191.
    let out = gpu.render(&treated(0.5));
    let p = &out.pixels()[last..last + 3];
    assert!(
        p.iter().all(|channel| (189..=193).contains(channel)),
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

    // Lighten and Darken at partial opacity: the two blends the GPU draws
    // over a copy of the ground, held to the CPU's own line.
    for (name, blend) in [("lightened", Blend::Lighten), ("darkened", Blend::Darken)] {
        let mut over = layer(solid(64, 48, [200, 60, 140, 255]));
        over.opacity = 0.3;
        over.blend = blend;
        assert_parity(
            name,
            &plan(64, 48, vec![layer(gradient(64, 48)), over]),
            0.99,
        );
    }

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

/// A module the driver refuses - here, one that is not WGSL at all, which
/// only reaches the device because the pass was built by hand rather than
/// by the catalogue - is caught in its error scope: the layer draws
/// untreated, the device is not dead, and the trial says no.
#[test]
fn a_pass_the_driver_refuses_is_skipped_and_fails_its_trial() {
    let Some(mut gpu) = gpu() else { return };
    let mut broken = package("test.broken", INVERT, "", &[], 1.0);
    broken.key = "test.broken@1#garbage".to_owned();
    broken.source = Arc::from("this is not a shader");
    let mut over = layer(solid(4, 4, [0, 200, 0, 255]));
    over.effects = vec![broken.clone()];
    let frame = gpu.render(&plan(4, 4, vec![over]));
    assert_eq!(frame.pixel(1, 1), Some([0, 200, 0, 255]), "drawn untreated");
    assert!(!gpu.is_dead());
    assert!(
        gpu.trial_at(&broken, 64, std::time::Duration::from_secs(5))
            .is_err()
    );
    // And a good pass still runs on the same compositor.
    let mut over = layer(solid(4, 4, [0, 200, 0, 255]));
    over.effects = vec![package("test.trial", INVERT, "", &[], 1.0)];
    let frame = gpu.render(&plan(4, 4, vec![over]));
    assert_eq!(frame.pixel(1, 1), Some([255, 55, 255, 255]));
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
    // The package mixes in the gamma it was written for (the legacy
    // wrapper), so its half is the half of the stored levels, give or take
    // the level the round trip through linear light can round to.
    let half = at(0.5);
    assert!(
        half.pixels()[..3]
            .iter()
            .zip([128u8, 0, 128])
            .all(|(got, want)| got.abs_diff(want) <= 1),
        "the half mix: {:?}",
        &half.pixels()[..3]
    );
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

/// A texture read back into a frame, for comparing a picture that stayed
/// on the device with one that was read back on the way.
fn read_texture(gpu: &WgpuCompositor, texture: &wgpu::Texture) -> Frame {
    let (width, height) = (texture.width(), texture.height());
    let padded = (width as usize * 4).div_ceil(256) * 256;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: (padded * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device answers");
    let data = slice.get_mapped_range().expect("mapped");
    let mut frame = Frame::transparent(width, height);
    let row = width as usize * 4;
    for y in 0..height as usize {
        frame.pixels_mut()[y * row..(y + 1) * row]
            .copy_from_slice(&data[y * padded..y * padded + row]);
    }
    frame
}

/// A packaged transition drawn whole on the device is the picture the
/// read-back path makes: the same stacks, the same shader, only without
/// the round trips through memory.
#[test]
fn a_transition_kept_on_the_device_matches_the_read_back_one() {
    let Some(mut gpu) = gpu() else { return };
    let (width, height) = (32, 18);
    let plan = |colour: [u8; 4]| FramePlan {
        time: concat_core::time::Rational::new(1, 2),
        width,
        height,
        layers: vec![PlannedLayer::picture(
            detached_clip(),
            Arc::new(solid(width, height, colour)),
        )],
        treatments: Vec::new(),
        output: concat_core::frame::Signal::Sdr,
    };
    let (from, to) = (plan([220, 30, 30, 255]), plan([30, 30, 220, 255]));
    let catalogue = concat_effects::Catalogue::builtin();
    let package = catalogue
        .packages()
        .find(|package| package.kind() == concat_effects::Kind::Transition)
        .expect("a transition package");
    let pass = catalogue
        .transition_pass(package.id(), &Default::default(), 0.5)
        .expect("a pass");

    let from_frame = gpu.render(&from);
    let to_frame = gpu.render(&to);
    let read_back = gpu
        .combine(width, height, from.seconds(), &from_frame, &to_frame, &pass)
        .expect("combines");
    let texture = gpu
        .render_transition_texture(&from, &to, &pass)
        .expect("combines on the device");
    let kept = read_texture(&gpu, &texture);
    let worst = kept
        .pixels()
        .iter()
        .zip(read_back.pixels())
        .enumerate()
        .filter(|(index, _)| index % 4 != 3)
        .map(|(_, (a, b))| a.abs_diff(*b))
        .max()
        .unwrap_or(0);
    assert!(
        worst <= 1,
        "{}: the two paths differ by {worst} a channel",
        package.id()
    );
}

/// A frame the device has drawn is found on it again when a later
/// composite asks for it - a scrub back over ground the monitor showed -
/// rather than uploaded again: the layer pool keeps what it uploaded until
/// its budget is spent.
#[test]
fn a_frame_drawn_before_is_not_uploaded_again() {
    let Some(mut gpu) = gpu() else { return };
    let frames: Vec<Arc<Frame>> = (0..10u8)
        .map(|shade| Arc::new(solid(64, 36, [shade * 20, 40, 90, 255])))
        .collect();
    let draw = |gpu: &mut WgpuCompositor, frame: &Arc<Frame>| {
        let plan = FramePlan {
            time: concat_core::time::Rational::new(0, 1),
            width: 64,
            height: 36,
            layers: vec![PlannedLayer::picture(detached_clip(), Arc::clone(frame))],
            treatments: Vec::new(),
            output: concat_core::frame::Signal::Sdr,
        };
        gpu.render_texture(&plan).expect("draws");
    };
    for frame in &frames {
        draw(&mut gpu, frame);
    }
    let first_pass = gpu.uploads;
    assert!(first_pass >= 10, "each new frame is uploaded once");
    for frame in frames.iter().rev() {
        draw(&mut gpu, frame);
    }
    assert_eq!(gpu.uploads, first_pass, "the way back is all cache hits");
}

/// The pool keeps what it uploaded only up to its limits: a long run of
/// distinct frames - here more small ones than the texture cap - leaves it
/// holding no more than the cap and the byte budget allow.
#[test]
fn the_pool_stays_within_its_limits() {
    let Some(mut gpu) = gpu() else { return };
    for index in 0..(POOL_TEXTURES as u32 + 76) {
        let frame = Arc::new(solid(4, 4, [(index % 251) as u8, 7, 9, 255]));
        let plan = FramePlan {
            time: concat_core::time::Rational::new(0, 1),
            width: 4,
            height: 4,
            layers: vec![PlannedLayer::picture(detached_clip(), frame)],
            treatments: Vec::new(),
            output: concat_core::frame::Signal::Sdr,
        };
        gpu.render_texture(&plan).expect("draws");
    }
    assert!(
        gpu.pool_textures <= POOL_TEXTURES,
        "{} textures",
        gpu.pool_textures
    );
    assert!(gpu.pool_bytes <= POOL_BUDGET);
    let counted: usize = gpu.pool.values().map(Vec::len).sum();
    assert_eq!(counted, gpu.pool_textures, "the count is the pool's");
}

/// A stack of passes takes turns between two textures and still applies
/// every pass in order: invert, turn the channels, invert again is the
/// turn alone, and five inverts are one. The stack claims two textures of
/// the layer's size whatever its length, beside the frame it reads and
/// the canvas.
#[test]
fn a_stack_of_passes_takes_turns_between_two_textures() {
    let Some(mut gpu) = gpu() else { return };
    const TURN: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(c.g, c.b, c.r, c.a); }";
    let invert = || package("concat.invert", INVERT, "", &[], 1.0);
    let turn = || package("concat.turn", TURN, "", &[], 1.0);
    let colour = [200, 90, 30, 255];
    let stacked = |effects: Vec<ShaderPass>| {
        let mut picture = layer(solid(8, 8, colour));
        picture.effects = effects;
        plan(8, 8, vec![picture])
    };

    let out = gpu.render(&stacked(vec![invert(), turn(), invert()]));
    let got = &out.pixels()[..3];
    assert!(
        got.iter()
            .zip([90, 30, 200])
            .all(|(got, want)| got.abs_diff(want) <= 1),
        "invert, turn, invert gave {got:?}"
    );
    // The frame, the two turns, and the canvas the frame is drawn on in
    // the working format before it is resolved.
    let after_three = gpu.pool[&(8, 8)].len();
    assert!(after_three <= 4, "{after_three} textures for three passes");

    let out = gpu.render(&stacked((0..5).map(|_| invert()).collect()));
    let got = &out.pixels()[..3];
    assert!(
        got.iter()
            .zip([55, 165, 225])
            .all(|(got, want)| got.abs_diff(want) <= 1),
        "five inverts gave {got:?}"
    );
    // One more: the new frame, the first still cached; the turns are reused.
    let after_five = gpu.pool[&(8, 8)].len();
    assert!(
        after_five <= after_three + 1,
        "{after_five} textures after five passes, {after_three} after three"
    );
}

/// The LUTs kept on the device are bounded: a run of more distinct looks
/// than the cache holds leaves it at its cap, the one just drawn among
/// them.
#[test]
fn the_lut_cache_keeps_the_recent_looks_only() {
    let Some(mut gpu) = gpu() else { return };
    let body = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(lut(c.rgb), c.a); }";
    let mut last = 0;
    for index in 0..(LUT_CACHE as u32 + 8) {
        let shade = index as f32 / 64.0;
        let table = Arc::new(Lut::from_rgb(2, &[shade, 0.5, 0.25].repeat(8)).expect("a table"));
        last = table.id;
        let mut looked = layer(solid(4, 4, [255, 0, 0, 255]));
        let mut pass = package("test.table", body, "", &[], 1.0);
        pass.lut = Some(table);
        looked.effects = vec![pass];
        gpu.render(&plan(4, 4, vec![looked]));
    }
    assert!(
        gpu.luts.len() <= LUT_CACHE,
        "{} looks cached",
        gpu.luts.len()
    );
    assert!(gpu.luts.contains_key(&last), "the look just drawn is kept");
    assert_eq!(gpu.luts.len(), gpu.luts_drawn.len());
}

/// The stack is drawn in half floats, so a pass that darkens a picture to
/// a tenth and one that brings it back up leave it as it was: in eight
/// bits the tenth rounds to a whole level and the way back multiplies the
/// rounding by ten.
#[test]
fn a_stack_keeps_its_precision_between_passes() {
    let Some(mut gpu) = gpu() else { return };
    const DOWN: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(c.rgb * 0.1, c.a); }";
    const UP: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(c.rgb * 10.0, c.a); }";
    let mut picture = layer(solid(8, 8, [203, 157, 97, 255]));
    picture.effects = vec![
        package("test.down", DOWN, "", &[], 1.0),
        package("test.up", UP, "", &[], 1.0),
    ];
    let out = gpu.render(&plan(8, 8, vec![picture]));
    let got = &out.pixels()[..3];
    assert!(
        got.iter()
            .zip([203, 157, 97])
            .all(|(got, want)| got.abs_diff(want) <= 1),
        "down and up again gave {got:?}"
    );
}

/// A deep frame of `level` a channel, sixteen bits, in `signal`.
fn deep(width: u32, height: u32, level: f32, signal: concat_core::frame::Signal) -> Frame {
    let value = (level.clamp(0.0, 1.0) * 65535.0).round() as u16;
    let pixel: Vec<u8> = [value, value, value, u16::MAX]
        .iter()
        .flat_map(|channel| channel.to_le_bytes())
        .collect();
    Frame::from_rgba64(
        width,
        height,
        pixel.repeat((width * height) as usize),
        signal,
    )
    .expect("a deep frame")
}

/// An HDR clip is converted into the working space on the GPU as it
/// uploads, and conformed to the SDR timeline there: HLG's reference white
/// (75 % of the signal, 203 nits) lands a little under SDR white, where
/// BT.2390's roll-off leaves room for the highlights above it, grey still;
/// the master's peak - all of HLG's signal, or 1000 nits of PQ - is SDR
/// white; black is black.
#[test]
fn an_hdr_frame_is_conformed_to_sdr_on_the_way_in() {
    use concat_core::frame::Signal;
    let Some(mut gpu) = gpu() else { return };
    let mut drawn = |frame: Frame| {
        let out = gpu.render(&plan(4, 4, vec![layer(frame)]));
        let p = out.pixel(1, 1).expect("in the frame");
        [p[0], p[1], p[2]]
    };
    let white = drawn(deep(4, 4, 0.75, Signal::Hlg));
    assert!(
        white.iter().all(|channel| (205..=245).contains(channel))
            && white[0].abs_diff(white[1]) <= 2
            && white[1].abs_diff(white[2]) <= 2,
        "HLG reference white came out {white:?}"
    );
    let peak = drawn(deep(4, 4, 1.0, Signal::Hlg));
    assert!(
        peak.iter().all(|channel| *channel >= 252),
        "HLG peak came out {peak:?}"
    );
    let black = drawn(deep(4, 4, 0.0, Signal::Hlg));
    assert!(
        black.iter().all(|channel| *channel <= 2),
        "HLG black came out {black:?}"
    );
    // 1000 nits in PQ is 0.7518 of the signal.
    let pq_peak = drawn(deep(4, 4, 0.7518, Signal::Pq));
    assert!(
        pq_peak.iter().all(|channel| *channel >= 250),
        "PQ 1000 nits came out {pq_peak:?}"
    );
}

/// `plan` drawn into the working space and read back as it stands there,
/// before the resolve: light, 1.0 SDR white, nothing clipped.
fn working(gpu: &mut WgpuCompositor, plan: &FramePlan) -> Vec<[f32; 4]> {
    let (draws, vertices) = gpu.prepare(plan);
    gpu.write_vertices(&vertices);
    let (canvas, texture, view) = gpu.canvas(plan.width, plan.height);
    let encoder = gpu.encode(&view, &texture, &draws, wgpu::Color::BLACK);
    gpu.queue.submit([encoder.finish()]);
    let out = read_floats(gpu, (plan.width, plan.height), canvas);
    gpu.retire();
    out
}

/// On an HDR timeline an HDR clip keeps its light above white: HLG's peak
/// is 1000 nits, 4.93 times SDR white, and its reference white SDR white
/// itself - where on an SDR timeline the peak is conformed down to white.
/// The same frame drawn for both is two pictures, kept apart. For the SDR
/// screen the resolve rolls the HDR timeline off with the maths the SDR
/// timeline's upload uses, so a lone clip looks the same on either; an SDR
/// clip's white goes a little under white there, leaving room above it.
#[test]
fn an_hdr_timeline_keeps_the_light_above_white() {
    use concat_core::frame::Signal;
    let Some(mut gpu) = gpu() else { return };
    let peak = deep(4, 4, 1.0, Signal::Hlg);
    let hdr = |layers| FramePlan {
        output: Signal::Hlg,
        ..plan(4, 4, layers)
    };
    let level = |pixels: Vec<[f32; 4]>| pixels[5];
    let sdr_light = level(working(&mut gpu, &plan(4, 4, vec![layer(peak.clone())])));
    assert!(
        (sdr_light[1] - 1.0).abs() < 0.01,
        "conformed: {sdr_light:?}"
    );
    let hdr_light = level(working(&mut gpu, &hdr(vec![layer(peak.clone())])));
    assert!(
        hdr_light[..3]
            .iter()
            .all(|channel| (channel - 1000.0 / 203.0).abs() < 0.05),
        "kept: {hdr_light:?}"
    );
    let white = level(working(
        &mut gpu,
        &hdr(vec![layer(deep(4, 4, 0.75, Signal::Hlg))]),
    ));
    assert!(
        (white[1] - 1.0).abs() < 0.01,
        "HLG reference white: {white:?}"
    );

    let shown = |gpu: &mut WgpuCompositor, plan: &FramePlan| {
        let out = gpu.render(plan);
        let p = out.pixel(1, 1).expect("in the frame");
        [p[0], p[1], p[2]]
    };
    for signal_level in [0.75, 1.0, 0.5] {
        let frame = deep(4, 4, signal_level, Signal::Hlg);
        let on_sdr = shown(&mut gpu, &plan(4, 4, vec![layer(frame.clone())]));
        let on_hdr = shown(&mut gpu, &hdr(vec![layer(frame)]));
        assert!(
            on_sdr.iter().zip(on_hdr).all(|(a, b)| a.abs_diff(b) <= 1),
            "HLG {signal_level}: {on_sdr:?} on SDR, {on_hdr:?} on HDR"
        );
    }
    let sdr_white = shown(
        &mut gpu,
        &hdr(vec![layer(solid(4, 4, [255, 255, 255, 255]))]),
    );
    assert!(
        sdr_white
            .iter()
            .all(|channel| (225..=240).contains(channel)),
        "SDR white on an HDR timeline: {sdr_white:?}"
    );
}

/// Delivered for an HDR file, an HDR timeline's frame comes back as the
/// signal it was made from: an HLG clip's levels through the working space
/// and out again, SDR white at HLG's reference white (75 %, 203 nits), and
/// the same for PQ, whose 203 nits are 58 % of its signal. Sixteen bits a
/// channel, opaque; an SDR timeline's frames stay eight-bit SDR.
#[test]
fn an_hdr_timeline_is_delivered_as_its_signal() {
    use concat_core::frame::{Depth, Signal};
    let Some(mut gpu) = gpu() else { return };
    gpu.deliver_hdr(true);
    let level = |frame: &Frame| {
        assert_eq!(frame.depth(), Depth::Sixteen);
        let at = (4 + 1) * 8;
        let channel = |offset: usize| {
            f64::from(u16::from_le_bytes([
                frame.pixels()[at + offset],
                frame.pixels()[at + offset + 1],
            ])) / 65_535.0
        };
        assert_eq!(channel(6), 1.0, "opaque");
        [channel(0), channel(2), channel(4)]
    };
    for signal in [Signal::Hlg, Signal::Pq] {
        let delivered = |gpu: &mut WgpuCompositor, frame: Frame| {
            gpu.render(&FramePlan {
                output: signal,
                ..plan(4, 4, vec![layer(frame)])
            })
        };
        for going in [0.25, 0.5, 0.75, 0.9] {
            let out = delivered(&mut gpu, deep(4, 4, going as f32, signal));
            assert_eq!(out.signal(), signal);
            let got = level(&out);
            assert!(
                got.iter().all(|channel| (channel - going).abs() < 0.003),
                "{signal:?} {going} came back {got:?}"
            );
        }
        let white = level(&delivered(&mut gpu, solid(4, 4, [255, 255, 255, 255])));
        let want = if signal == Signal::Hlg { 0.75 } else { 0.5807 };
        assert!(
            white.iter().all(|channel| (channel - want).abs() < 0.003),
            "{signal:?}: SDR white came out {white:?}"
        );
    }
    let sdr = gpu.render(&plan(4, 4, vec![layer(solid(4, 4, [255, 255, 255, 255]))]));
    assert_eq!(sdr.depth(), Depth::Eight, "an SDR timeline stays SDR");
}

/// A pass of a format 2 package - the scene-linear contract - with `body`
/// as its shader and no knobs, at `intensity`.
fn scene_linear(id: &str, body: &str, intensity: f32) -> ShaderPass {
    let manifest = concat_effects::Manifest::parse(&format!(
        "format = 2\n[effect]\nid = \"{id}\"\nname = \"Test\"\nkind = \"effect\"\n[wgsl]\nentry = \"effect.wgsl\"\n"
    ))
    .expect("a manifest");
    let shader = concat_effects::Shader::compile(&manifest, body).expect("compiles");
    shader.pass(&BTreeMap::new(), &manifest.params, intensity, None, None)
}

/// Within `tolerance` of `want` a channel, relative, for the colour and
/// alpha alike.
fn near(got: [f32; 4], want: [f32; 4], tolerance: f32) -> bool {
    got.iter()
        .zip(want)
        .all(|(got, want)| (got - want).abs() <= tolerance * want.abs().max(0.01))
}

/// Every built-in package's probes: a picture of one working-space colour
/// through its shader comes out as the probe pins, in floats.
#[test]
fn every_package_probe_holds() {
    let Some(mut gpu) = gpu() else { return };
    let mut failures = Vec::new();
    let mut probed = 0;
    for package in concat_effects::Catalogue::builtin().packages() {
        for (n, probe) in package.probes.iter().enumerate() {
            let got = match (package.probe_pass(probe), package.probe_transition(probe)) {
                (Some(pass), _) => gpu.probe(&[pass], probe.input, 8, 0.0),
                (None, Some(cut)) => {
                    gpu.probe_transition(&cut, probe.input, probe.to.unwrap_or_default(), 8, 0.0)
                }
                (None, None) => panic!("{}: a probe with no shader", package.id()),
            }
            .expect("reads back");
            probed += 1;
            if let Err(error) = probe.check(got) {
                failures.push(format!("{}: {}\n  {error}", package.id(), probe.label(n)));
            }
        }
    }
    eprintln!("{probed} probes");
    assert!(probed > 0, "no package has a probe");
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A format 2 stack hands its light on unclipped: four times the light and
/// a quarter of it again is the picture it was, a highlight far above
/// white in between. The same stack in format 1 is clipped to white at the
/// second pass, as those packages always were.
#[test]
fn a_scene_linear_stack_keeps_the_light_above_white() {
    let Some(mut gpu) = gpu() else { return };
    const UP: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(c.rgb * 4.0, c.a); }";
    const DOWN: &str = "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(c.rgb * 0.25, c.a); }";
    let colour = [0.7, 0.35, 0.1, 1.0];
    let linear = [
        scene_linear("test.up", UP, 1.0),
        scene_linear("test.down", DOWN, 1.0),
    ];
    let between = gpu.probe(&linear[..1], colour, 8, 0.0).expect("reads back");
    assert!(near(between, [2.8, 1.4, 0.4, 1.0], 0.004), "{between:?}");
    let got = gpu.probe(&linear, colour, 8, 0.0).expect("reads back");
    assert!(near(got, colour, 0.004), "there and back gave {got:?}");

    let legacy = [
        package("test.up", UP, "", &[], 1.0),
        package("test.down", DOWN, "", &[], 1.0),
    ];
    let clipped = gpu.probe(&legacy, colour, 8, 0.0).expect("reads back");
    assert!(clipped[0] < 0.1, "format 1 clips at white: {clipped:?}");
}

/// `to_log` puts middle grey, SDR white and black where ACEScct does, and
/// `from_log` brings any level back - a highlight, the toe, a negative
/// channel - as it went in.
#[test]
fn the_log_space_goes_there_and_back() {
    let Some(mut gpu) = gpu() else { return };
    let to_log = scene_linear(
        "test.log",
        "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(to_log(c.rgb), c.a); }",
        1.0,
    );
    let got = gpu
        .probe(std::slice::from_ref(&to_log), [0.18, 1.0, 0.0, 1.0], 4, 0.0)
        .expect("reads back");
    assert!(near(got, [0.4136, 0.5548, 0.0729, 1.0], 0.002), "{got:?}");

    let round = scene_linear(
        "test.round",
        "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(from_log(to_log(c.rgb)), c.a); }",
        1.0,
    );
    for colour in [[6.0, 0.18, -0.02, 1.0], [0.001, 0.0, 1.0, 1.0]] {
        let got = gpu
            .probe(std::slice::from_ref(&round), colour, 4, 0.0)
            .expect("reads back");
        assert!(near(got, colour, 0.004), "{colour:?} came back {got:?}");
    }
}

/// A format 2 pass is mixed over the untouched layer in light: a quarter
/// of the way from 0.2 to white is 0.4.
#[test]
fn a_scene_linear_pass_mixes_by_intensity_in_light() {
    let Some(mut gpu) = gpu() else { return };
    let white = scene_linear(
        "test.white",
        "fn effect(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(1.0, 1.0, 1.0, sample(uv).a); }",
        0.25,
    );
    let got = gpu
        .probe(&[white], [0.2, 0.2, 0.2, 1.0], 4, 0.0)
        .expect("reads back");
    assert!(near(got, [0.4, 0.4, 0.4, 1.0], 0.004), "{got:?}");
}

/// No look clips: a highlight four times SDR white comes out of every
/// built-in look drawn in the display space brighter than white does, as
/// light that went in brighter should.
#[test]
fn every_look_carries_a_highlight_past_white() {
    let Some(mut gpu) = gpu() else { return };
    let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    let mut clipped = Vec::new();
    for package in concat_effects::Catalogue::builtin().packages() {
        let display = package
            .manifest
            .wgsl
            .as_ref()
            .is_some_and(|wgsl| wgsl.space == concat_effects::Space::Display);
        if !display {
            continue;
        }
        let pass = package.trial_pass().expect("a shader");
        let white = gpu
            .probe(std::slice::from_ref(&pass), [1.0, 1.0, 1.0, 1.0], 8, 0.0)
            .expect("reads back");
        let bright = gpu
            .probe(&[pass], [4.0, 4.0, 4.0, 1.0], 8, 0.0)
            .expect("reads back");
        if luma(bright) <= luma(white) * 1.5 {
            clipped.push(format!(
                "{}: white {white:?}, 4x white {bright:?}",
                package.id()
            ));
        }
    }
    assert!(clipped.is_empty(), "\n{}", clipped.join("\n"));
}

/// `pixels`, straight RGBA in the working space and row by row, written
/// into a pooled texture `width` by `height` as half floats: a picture
/// with the light past white and the alpha an eight-bit frame cannot hold.
fn written(gpu: &mut WgpuCompositor, (width, height): (u32, u32), pixels: &[[f32; 4]]) -> usize {
    let index = gpu.claim(width, height);
    let texels: Vec<u8> = pixels
        .iter()
        .flatten()
        .flat_map(|channel| half::f16::from_f32(*channel).to_le_bytes())
        .collect();
    gpu.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &gpu.pool[&(width, height)][index].texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * WORK_BYTES as u32),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    index
}

/// The pooled texture at `index`, `width` by `height`, read back whole in
/// the half floats it holds.
fn read_floats(gpu: &WgpuCompositor, (width, height): (u32, u32), index: usize) -> Vec<[f32; 4]> {
    let row = width as usize * WORK_BYTES as usize;
    let padded = row.div_ceil(ROW_ALIGN) * ROW_ALIGN;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test float readback"),
        size: (padded * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        gpu.pool[&(width, height)][index].texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the device answers");
    let data = slice.get_mapped_range().expect("mapped");
    (0..height as usize)
        .flat_map(|y| data[y * padded..y * padded + row].chunks_exact(8))
        .map(|texel| {
            let channel = |at: usize| half::f16::from_le_bytes([texel[at], texel[at + 1]]).to_f32();
            [channel(0), channel(2), channel(4), channel(6)]
        })
        .collect()
}

/// `passes` over `picture`, `size` pixels, as a layer's effects run: the
/// last picture drawn, read back in floats.
fn treated(
    gpu: &mut WgpuCompositor,
    size: (u32, u32),
    picture: &[[f32; 4]],
    passes: &[ShaderPass],
) -> Vec<[f32; 4]> {
    gpu.used.values_mut().for_each(|used| *used = 0);
    gpu.composites += 1;
    let source = written(gpu, size, picture);
    let drawn = gpu.run_passes(size.0, size.1, source, passes, 0.0, 0.0);
    let out = read_floats(gpu, size, drawn);
    gpu.retire();
    out
}

/// A pass of a format 2 package drawn in several passes: `body` as its
/// shader, `passes` its `[[wgsl.pass]]` tables, `params` its knobs, at
/// `intensity`.
fn in_passes(body: &str, passes: &str, params: &str, intensity: f32) -> ShaderPass {
    let manifest = concat_effects::Manifest::parse(&format!(
        "format = 2\n[effect]\nid = \"test.passes\"\nname = \"Test\"\nkind = \"effect\"\n{params}\n[wgsl]\nentry = \"effect.wgsl\"\n{passes}"
    ))
    .expect("a manifest");
    let shader = concat_effects::Shader::compile(&manifest, body).expect("compiles");
    shader.pass(&BTreeMap::new(), &manifest.params, intensity, None, None)
}

/// The picture `picture`, `size` pixels, blurred by a Gaussian of `sigma`
/// pixels summed directly over both axes at `(x, y)`, out to five sigmas,
/// the edge clamped as the GPU's sampler clamps it: premultiplied colour,
/// then alpha, in doubles.
fn gaussian_at(
    picture: &[[f32; 4]],
    (width, height): (u32, u32),
    (x, y): (i64, i64),
    sigma: f64,
) -> [f64; 4] {
    let reach = (5.0 * sigma).ceil() as i64;
    let (mut sum, mut total) = ([0.0f64; 4], 0.0f64);
    for dy in -reach..=reach {
        let row = (y + dy).clamp(0, i64::from(height) - 1);
        for dx in -reach..=reach {
            let column = (x + dx).clamp(0, i64::from(width) - 1);
            let weight = (-((dx * dx + dy * dy) as f64) / (2.0 * sigma * sigma)).exp();
            let [r, g, b, a] = picture[(row * i64::from(width) + column) as usize].map(f64::from);
            for (into, value) in sum.iter_mut().zip([r * a, g * a, b * a, a]) {
                *into += weight * value;
            }
            total += weight;
        }
    }
    sum.map(|value| value / total)
}

/// A picture `size` pixels with everything a blur can get wrong in it: a
/// gradient, a checkerboard, dots of light `peak` times white, a hard edge
/// past white, a half-transparent strip down the left, and a transparent
/// band whose colour must not bleed into anything.
fn busy(size: (u32, u32), peak: f32) -> Vec<[f32; 4]> {
    (0..size.1)
        .flat_map(|y| (0..size.0).map(move |x| (x, y)))
        .map(|(x, y)| {
            let checker = if (x / 6 + y / 6) % 2 == 0 { 0.1 } else { 0.6 };
            let mut rgb = [
                0.02 + 0.9 * x as f32 / size.0 as f32,
                checker,
                0.05 + 0.5 * y as f32 / size.1 as f32,
            ];
            if x >= 150 {
                rgb = [1.5, 0.05, 0.05];
            }
            if x % 40 / 2 == 10 && y % 30 / 2 == 7 {
                rgb = [peak; 3];
            }
            match y {
                80..90 => [5.0, 5.0, 5.0, 0.0],
                _ if x < 20 => [rgb[0], rgb[1], rgb[2], 0.5],
                _ => [rgb[0], rgb[1], rgb[2], 1.0],
            }
        })
        .collect()
}

/// `picture`, `size` pixels, at `(x, y)` in pixels - a pixel's centre at
/// its whole number - read between its pixels as `sample_premultiplied`
/// reads it: the four about it premultiplied and mixed bilinearly, the edge
/// clamped as the GPU's sampler clamps it; in doubles.
fn premultiplied_at(
    picture: &[[f32; 4]],
    (width, height): (u32, u32),
    (x, y): (f64, f64),
) -> [f64; 4] {
    let x = x.clamp(0.0, f64::from(width - 1));
    let y = y.clamp(0.0, f64::from(height - 1));
    let (left, top) = (x.floor(), y.floor());
    let (fx, fy) = (x - left, y - top);
    let mut mixed = [0.0f64; 4];
    for (down, wy) in [(0, 1.0 - fy), (1, fy)] {
        for (across, wx) in [(0, 1.0 - fx), (1, fx)] {
            let column = (left as i64 + across).min(i64::from(width) - 1);
            let row = (top as i64 + down).min(i64::from(height) - 1);
            let [r, g, b, a] = picture[(row * i64::from(width) + column) as usize].map(f64::from);
            for (into, value) in mixed.iter_mut().zip([r * a, g * a, b * a, a]) {
                *into += wx * wy * value;
            }
        }
    }
    mixed
}

/// How far `got` is from `want` at a grid of pixels - every `stride`th of
/// each row and column, and the last, where `want` has an answer -
/// premultiplied: the worst, where, and the root mean square.
fn measured(
    got: &[[f32; 4]],
    (width, height): (u32, u32),
    stride: usize,
    want: impl Fn(i64, i64) -> Option<[f64; 4]>,
) -> (f64, (i64, i64), f64) {
    let along = |length: u32| {
        (0..i64::from(length))
            .step_by(stride)
            .chain([i64::from(length) - 1])
            .collect::<Vec<_>>()
    };
    let (mut worst, mut at, mut squares, mut count) = (0.0f64, (0, 0), 0.0f64, 0);
    for &y in &along(height) {
        for &x in &along(width) {
            let Some(want) = want(x, y) else { continue };
            let [r, g, b, a] = got[(y * i64::from(width) + x) as usize].map(f64::from);
            for (got, want) in [r * a, g * a, b * a, a].into_iter().zip(want) {
                let error = (got - want).abs();
                squares += error * error;
                count += 1;
                if error > worst {
                    (worst, at) = (error, (x, y));
                }
            }
        }
    }
    (worst, at, (squares / f64::from(count)).sqrt())
}

/// The Gaussian blur - across and then down, at a power of two fewer
/// pixels where its radius is wide, and read back over the layer - is the
/// 2-D Gaussian summed directly, at every radius from its least to its
/// most: over a picture with a gradient, a checkerboard, dots of light six
/// times white, a hard edge past white, a half-transparent strip, and a
/// transparent band whose colour must not bleed into anything.
#[test]
fn the_gaussian_blur_is_a_direct_gaussian() {
    const PEAK: f64 = 6.0;
    let Some(mut gpu) = gpu() else { return };
    let blur = concat_effects::Catalogue::builtin()
        .get("concat.gaussian-blur")
        .expect("a built-in");
    let size = (192, 108);
    let picture = busy(size, PEAK as f32);
    // Below a radius of six the last pass blurs down `across` itself, and
    // `down` is a sliver nothing reads.
    for (radius, across, down) in [
        (1.0, 1, 64),
        (4.0, 1, 64),
        (10.0, 2, 2),
        (24.0, 8, 8),
        (50.0, 16, 16),
    ] {
        let pass = blur
            .pass(&BTreeMap::from([("radius".to_owned(), radius)]), None)
            .expect("a shader");
        assert_eq!(
            pass.stages
                .iter()
                .map(|stage| stage.shrink)
                .collect::<Vec<_>>(),
            [[across, 1], [down, down]],
            "radius {radius}"
        );
        let got = treated(&mut gpu, size, &picture, &[pass]);
        // Every pixel where the sum is quick, a grid of them - the edges
        // among them - where it is not.
        let stride = (radius / 4.0).max(1.0) as usize;
        let (worst, at, rms) = measured(&got, size, stride, |x, y| {
            Some(gaussian_at(&picture, size, (x, y), radius))
        });
        eprintln!("radius {radius}: worst {worst:.5} at {at:?}, rms {rms:.6}");
        // Half floats, the fetch between two pixels, and the sum cut at
        // three sigmas each leave a trace; together they come to a quarter
        // of a percent of the brightest light at worst.
        assert!(
            worst < 0.0025 * PEAK,
            "radius {radius}: {worst} off at {at:?}"
        );
        assert!(rms < 0.0005 * PEAK, "radius {radius}: rms {rms}");
    }
}

/// A pass's picture is the layer's size divided by its shrink, rounded
/// up, and a later pass measures it through `<target>_texel`.
#[test]
fn a_pass_draws_its_picture_at_its_shrunk_size() {
    let Some(mut gpu) = gpu() else { return };
    let pass = in_passes(
        "fn small(uv: vec2<f32>) -> vec4<f32> { return sample(uv); }\n\
         fn effect(uv: vec2<f32>) -> vec4<f32> { let seen = small_at(uv); return vec4<f32>(1.0 / small_texel(), seen.b, 1.0); }",
        "[[wgsl.pass]]\ntarget = \"small\"\nshrink = [\"4\", \"2\"]\n",
        "",
        1.0,
    );
    for (side, want) in [(8, [2.0, 4.0]), (10, [3.0, 5.0])] {
        let got = gpu
            .probe(std::slice::from_ref(&pass), [0.1, 0.2, 0.3, 1.0], side, 0.0)
            .expect("reads back");
        assert!(
            near(got, [want[0], want[1], 0.3, 1.0], 0.004),
            "{side}: {got:?}"
        );
    }
}

/// A pass's picture holds what its function returned - past white and
/// below zero, nothing clipped or mixed - and only the last pass mixes
/// by intensity: a quarter of the way from 0.2 to the picture, once.
#[test]
fn only_the_last_pass_mixes_by_intensity() {
    let Some(mut gpu) = gpu() else { return };
    let at = |intensity: f32| {
        in_passes(
            "fn light(uv: vec2<f32>) -> vec4<f32> { return vec4<f32>(6.0, -0.5, 1.0, 1.0); }\n\
             fn effect(uv: vec2<f32>) -> vec4<f32> { return light_at(uv); }",
            "[[wgsl.pass]]\ntarget = \"light\"\nshrink = [\"2\", \"2\"]\n",
            "",
            intensity,
        )
    };
    let whole = gpu
        .probe(&[at(1.0)], [0.2, 0.2, 0.2, 1.0], 8, 0.0)
        .expect("reads back");
    assert!(near(whole, [6.0, -0.5, 1.0, 1.0], 0.004), "{whole:?}");
    let quarter = gpu
        .probe(&[at(0.25)], [0.2, 0.2, 0.2, 1.0], 8, 0.0)
        .expect("reads back");
    assert!(near(quarter, [1.65, 0.025, 0.4, 1.0], 0.004), "{quarter:?}");
}

/// A stack of packages drawn in passes hands its pictures from one package
/// to the next: five blurs claim no more textures than two, and draw a
/// flat picture as it was.
#[test]
fn a_stack_of_blurs_shares_its_pictures() {
    let Some(mut gpu) = gpu() else { return };
    let blur = concat_effects::Catalogue::builtin()
        .get("concat.gaussian-blur")
        .expect("a built-in")
        .trial_pass()
        .expect("a shader");
    let colour = [0.3, 2.0, 0.05, 1.0];
    let two = gpu
        .probe(&vec![blur.clone(); 2], colour, 64, 0.0)
        .expect("reads back");
    let after_two = gpu.pool_textures;
    let five = gpu
        .probe(&vec![blur; 5], colour, 64, 0.0)
        .expect("reads back");
    assert_eq!(gpu.pool_textures, after_two, "five blurs claimed more");
    for got in [two, five] {
        assert!(near(got, colour, 0.004), "{got:?}");
    }
    // At the default radius, ten: across at half the width, then down at
    // half of both.
    assert!(gpu.pool.contains_key(&(32, 64)) && gpu.pool.contains_key(&(32, 32)));
}

/// A look made from a table reads it in the space it was made for and
/// carries a level past the table's ends on past them: through the
/// identity, a highlight four times white comes out as it went in, read in
/// the display encoding or in log. And a table's values come through finer
/// than eight bits.
#[test]
fn a_table_is_read_in_its_space_and_carried_past_its_ends() {
    let Some(mut gpu) = gpu() else { return };
    let look = |space: &str, lut: Lut| {
        let manifest = concat_effects::Manifest::parse(&format!(
            "format = 2\n[effect]\nid = \"test.look\"\nname = \"Look\"\nkind = \"filter\"\n\
             [wgsl]\nentry = \"effect.wgsl\"\nspace = \"{space}\"\n"
        ))
        .expect("a manifest");
        concat_effects::Shader::compile(&manifest, concat_effects::looks::SHADER)
            .expect("compiles")
            .pass(&BTreeMap::new(), &[], 1.0, Some(Arc::new(lut)), None)
    };
    for space in ["display", "log"] {
        let identity = look(space, Lut::identity(33));
        for colour in [[4.0, 0.5, 0.02, 1.0], [0.18, 0.18, 0.18, 1.0]] {
            let got = gpu
                .probe(std::slice::from_ref(&identity), colour, 8, 0.0)
                .expect("reads back");
            assert!(
                near(got, colour, 0.01),
                "{space}: {colour:?} came out {got:?}"
            );
        }
    }
    // One colour everywhere, a value eight bits would round to 31/255.
    let flat = Lut::from_rgb(2, &[0.123_456, 0.5, 0.25].repeat(8)).expect("a table");
    let got = gpu
        .probe(&[look("display", flat)], [0.3, 0.3, 0.3, 1.0], 8, 0.0)
        .expect("reads back");
    let want = [0.123_456f32, 0.5, 0.25].map(|level| level.powf(2.4));
    assert!(
        near(got, [want[0], want[1], want[2], 1.0], 0.004),
        "{got:?} against {want:?}"
    );
}

/// The motion blur - a coarse Gaussian along its line, then straight lines
/// drawn between its samples - is the Gaussian of sigma `length` along the
/// line summed directly, over the busy picture: along the rows, down the
/// columns and at a slant, from its shortest streak to its longest.
#[test]
fn a_motion_blur_is_a_gaussian_along_its_line() {
    const PEAK: f64 = 6.0;
    let Some(mut gpu) = gpu() else { return };
    let blur = concat_effects::Catalogue::builtin()
        .get("concat.motion-blur")
        .expect("a built-in");
    let size = (192, 108);
    let picture = busy(size, PEAK as f32);
    for (length, angle) in [
        (2.0, 0.0),
        (5.0, 0.0),
        (18.0, 0.0),
        (60.0, 0.0),
        (18.0, 90.0),
        (12.0, 30.0),
    ] {
        let pass = blur
            .pass(
                &BTreeMap::from([("length".to_owned(), length), ("angle".to_owned(), angle)]),
                None,
            )
            .expect("a shader");
        let got = treated(&mut gpu, size, &picture, &[pass]);
        let (sin, cos) = f64::to_radians(angle).sin_cos();
        let step = (length / 16.0).min(0.25);
        let reach = (5.0 * length / step).ceil() as i64;
        let streak = |x: i64, y: i64| {
            let (mut sum, mut total) = ([0.0f64; 4], 0.0f64);
            for k in -reach..=reach {
                let t = k as f64 * step;
                let weight = (-t * t / (2.0 * length * length)).exp();
                let seen =
                    premultiplied_at(&picture, size, (x as f64 + t * cos, y as f64 - t * sin));
                for (into, value) in sum.iter_mut().zip(seen) {
                    *into += weight * value;
                }
                total += weight;
            }
            sum.map(|value| value / total)
        };
        let (worst, at, rms) = measured(&got, size, 3, |x, y| Some(streak(x, y)));
        eprintln!("length {length} at {angle}°: worst {worst:.5} at {at:?}, rms {rms:.6}");
        assert!(
            worst < 0.01 * PEAK,
            "length {length} at {angle}°: {worst} off at {at:?}"
        );
        assert!(
            rms < 0.0015 * PEAK,
            "length {length} at {angle}°: rms {rms}"
        );
    }
}

/// The zoom blur - three passes of sixteen scales - is the four thousand
/// and ninety-six scales of the picture about its centre summed directly,
/// about the middle and about a corner: to the half float over a ramp,
/// where the passes compose exactly, and over the busy picture as near as
/// passes that read each other between pixels come - they soften a streak
/// across its line by about half a pixel, which shows only where a streak
/// grazes a dot six times white.
#[test]
fn a_zoom_blur_is_its_scales_summed() {
    const PEAK: f64 = 6.0;
    let Some(mut gpu) = gpu() else { return };
    let zoom = concat_effects::Catalogue::builtin()
        .get("concat.zoom-blur")
        .expect("a built-in");
    let size = (192, 108);
    let ramp: Vec<[f32; 4]> = (0..size.1)
        .flat_map(|y| {
            (0..size.0).map(move |x| [x as f32 / size.0 as f32, 0.5, y as f32 / size.1 as f32, 1.0])
        })
        .collect();
    for (picture, worst_of, rms_of) in [
        (ramp, 0.002, 0.0005),
        (busy(size, PEAK as f32), 0.025 * PEAK, 0.002 * PEAK),
    ] {
        for (amount, centre) in [
            (25.0, (50.0, 50.0)),
            (100.0, (50.0, 50.0)),
            (60.0, (10.0, 90.0)),
        ] {
            let pass = zoom
                .pass(
                    &BTreeMap::from([
                        ("amount".to_owned(), amount),
                        ("x".to_owned(), centre.0),
                        ("y".to_owned(), centre.1),
                    ]),
                    None,
                )
                .expect("a shader");
            let got = treated(&mut gpu, size, &picture, &[pass]);
            let span = -(1.0 - f64::min(amount / 100.0, 0.99)).ln();
            let (cx, cy) = (centre.0 / 100.0, centre.1 / 100.0);
            let (width, height) = (f64::from(size.0), f64::from(size.1));
            let (worst, at, rms) = measured(&got, size, 4, |x, y| {
                let (u, v) = ((x as f64 + 0.5) / width, (y as f64 + 0.5) / height);
                let mut sum = [0.0f64; 4];
                for k in 0..4096 {
                    let scale = (-span * f64::from(k) / 4096.0).exp();
                    let (su, sv) = (cx + (u - cx) * scale, cy + (v - cy) * scale);
                    let seen =
                        premultiplied_at(&picture, size, (su * width - 0.5, sv * height - 0.5));
                    for (into, value) in sum.iter_mut().zip(seen) {
                        *into += value / 4096.0;
                    }
                }
                Some(sum)
            });
            eprintln!("zoom {amount} about {centre:?}: worst {worst:.5} at {at:?}, rms {rms:.6}");
            assert!(worst < worst_of, "zoom {amount}: {worst} off at {at:?}");
            assert!(rms < rms_of, "zoom {amount}: rms {rms}");
        }
    }
}

/// The fisheye draws every pixel of the frame from somewhere inside it, at
/// full strength and at any shape of frame - no black holes, as earlier
/// builds drew - and pulls the picture in towards the middle without ever
/// folding it back: a ramp across the frame still climbs from left to
/// right along the middle row, the middle and the corners where they were.
#[test]
fn the_fisheye_fills_the_frame() {
    let Some(mut gpu) = gpu() else { return };
    let fisheye = concat_effects::Catalogue::builtin()
        .get("concat.fisheye")
        .expect("a built-in");
    let pass = fisheye
        .pass(&BTreeMap::from([("strength".to_owned(), 100.0)]), None)
        .expect("a shader");
    for size in [(96, 54), (54, 96), (64, 64)] {
        let flat = vec![[0.3f32, 0.5, 0.7, 1.0]; (size.0 * size.1) as usize];
        let got = treated(&mut gpu, size, &flat, std::slice::from_ref(&pass));
        for (n, pixel) in got.iter().enumerate() {
            assert!(
                near(*pixel, [0.3, 0.5, 0.7, 1.0], 0.004),
                "{size:?}: pixel {n} is {pixel:?}"
            );
        }
        let ramp: Vec<[f32; 4]> = (0..size.1)
            .flat_map(|_| (0..size.0).map(|x| [(x as f32 + 0.5) / size.0 as f32, 0.0, 0.0, 1.0]))
            .collect();
        let got = treated(&mut gpu, size, &ramp, std::slice::from_ref(&pass));
        let row = &got[(size.1 / 2 * size.0) as usize..][..size.0 as usize];
        assert!(
            row.windows(2).all(|pair| pair[1][0] >= pair[0][0]),
            "{size:?}: the middle row folds back"
        );
        let quarter = row[(size.0 / 4) as usize][0];
        assert!(
            quarter > 0.3 && quarter < 0.5,
            "{size:?}: a quarter of the way across reads {quarter}, not pulled towards the middle"
        );
        let corner = got[0][0];
        assert!(
            (corner - ramp[0][0]).abs() < 0.01,
            "{size:?}: the corner moved to {corner}"
        );
    }
}
