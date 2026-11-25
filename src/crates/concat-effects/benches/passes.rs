// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The passes a chain of effects hands its renderer, measured for one
//! frame: the whole road from the document's knobs to the uniform bytes.
//! Every frame of a graded timeline walks it, in the export's loop and in
//! the preview's, so it is measured.

use std::hint::black_box;

use concat_effects::Catalogue;
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

fn passes(c: &mut Criterion) {
    let catalogue = Catalogue::builtin();
    let still = [panel(false)];
    let riding = [panel(true)];

    let mut group = c.benchmark_group("passes");
    group.bench_function("a still chain's passes for one frame", |b| {
        b.iter(|| {
            black_box(catalogue.shader_passes_at(
                black_box(&still),
                black_box(0.42),
                black_box(None),
            ))
        })
    });
    group.bench_function("a riding chain's passes for one frame", |b| {
        b.iter(|| {
            black_box(catalogue.shader_passes_at(
                black_box(&riding),
                black_box(0.42),
                black_box(None),
            ))
        })
    });
    group.finish();
}

criterion_group!(benches, passes);
criterion_main!(benches);
