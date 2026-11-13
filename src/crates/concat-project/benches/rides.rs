// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! What a renderer asks of an applied filter for every frame of every
//! layer that carries one: one knob's ride, and the whole panel's
//! parameters at an instant. These run per frame, so they are measured -
//! a change here is a change a graded export pays for once per frame per
//! layer, and a preview pays for again on every redraw.

use std::hint::black_box;

use concat_project::model::{AppliedFilter, KeyEase};
use criterion::{Criterion, criterion_group, criterion_main};

/// The colour panel every picture clip carries, set the way a grade
/// leaves it; riding, two of its knobs are keyed six times over with
/// eased segments between - the way a real ride looks.
fn panel(riding: bool) -> AppliedFilter {
    let mut filter = AppliedFilter::new("concat.adjust");
    for (key, value) in [
        ("temperature", 5900.0),
        ("tint", 4.0),
        ("saturation", 12.0),
        ("exposure", 0.35),
        ("contrast", 18.0),
        ("highlights", -10.0),
        ("shadows", 8.0),
        ("vignette", 15.0),
    ] {
        filter.params.insert(key.to_owned(), value);
    }
    if riding {
        for (key, span) in [("saturation", 36.0), ("exposure", 1.1)] {
            for step in 0..6 {
                let ease = if step % 2 == 0 {
                    KeyEase::IN_OUT
                } else {
                    KeyEase::OUT
                };
                filter.set_key(
                    key,
                    step as f64 / 5.0,
                    span * (step as f64 - 2.0) / 5.0,
                    ease,
                );
            }
        }
    }
    filter
}

fn reads(c: &mut Criterion) {
    let still = panel(false);
    let riding = panel(true);

    let mut group = c.benchmark_group("ride");
    group.bench_function("a still panel's parameters at an instant", |b| {
        b.iter(|| black_box(still.params_at(black_box(0.42))))
    });
    group.bench_function("a riding panel's parameters at an instant", |b| {
        b.iter(|| black_box(riding.params_at(black_box(0.42))))
    });
    group.bench_function("one riding knob at an instant", |b| {
        b.iter(|| {
            black_box(riding.value_at(black_box("saturation"), black_box(0.42), black_box(0.0)))
        })
    });
    group.finish();
}

criterion_group!(benches, reads);
criterion_main!(benches);
