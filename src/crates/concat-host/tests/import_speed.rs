// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What an import costs a file, step by step: the probe, the bin card's
//! picture, the waveform and the filmstrip - each as the import runs it,
//! from cold (no artwork cached).
//!
//! Ignored: it needs footage. `CONCAT_IMPORT_FOOTAGE=a.mp4,b.mov cargo
//! test -p concat-host --release --test import_speed -- --ignored --nocapture`.

use std::time::Instant;

fn ms(begun: Instant) -> f64 {
    begun.elapsed().as_secs_f64() * 1000.0
}

#[test]
#[ignore = "a measurement, not a check"]
fn import_speed() {
    let Ok(footage) = std::env::var("CONCAT_IMPORT_FOOTAGE") else {
        return;
    };
    concat_media::set_hardware_decode(true);
    for path in footage.split(',') {
        let begun = Instant::now();
        let summary = concat_host::media::probe(path).expect("probes");
        let probe = ms(begun);
        let begun = Instant::now();
        let _ = concat_host::media::card_still(path, 1.0, 176);
        let card = ms(begun);
        let begun = Instant::now();
        let _ = concat_host::media::peaks(path, None, None);
        let wave = ms(begun);
        let begun = Instant::now();
        let _ = concat_host::media::filmstrip(path, 24, 64);
        let strip = ms(begun);
        eprintln!(
            "import {:.1} s of {}: probe {probe:.0} ms, card {card:.0} ms, waveform {wave:.0} ms, \
             filmstrip {strip:.0} ms",
            summary.duration.unwrap_or_default(),
            std::path::Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
        );
    }
}
