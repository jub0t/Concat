// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! How playback feels on a cut-heavy timeline: the monitor's own frame
//! path, asked at the pace of a playing clock and decoding ahead the way
//! the window does, over clips of real footage that cut every
//! `CONCAT_PLAY_CUT` seconds to another place in another file. Says how
//! many frames came in time, how long each was waited for, and how many
//! the clock had passed before they came - the stutter a person sees.
//!
//! Ignored: it needs footage. `CONCAT_PLAY_FOOTAGE=a.mp4,b.mp4 cargo test
//! -p concat-host --release --test playback_feel -- --ignored --nocapture`;
//! `CONCAT_PLAY_SIZE` the monitor's frame (960x540), `CONCAT_PLAY_LAYERS`
//! clips stacked (1), `CONCAT_PLAY_SECONDS` how long it plays (8).

use std::sync::Arc;
use std::time::{Duration, Instant};

use concat_export::{ClipKind, ExportClip};
use concat_host::preview::{FrameSpec, Monitor};
use concat_project::DocumentSettings;

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "a measurement, not a check"]
fn playback_feel() {
    let Ok(footage) = std::env::var("CONCAT_PLAY_FOOTAGE") else {
        return;
    };
    concat_media::set_hardware_decode(true);
    let files: Vec<_> = footage
        .split(',')
        .map(|path| {
            let probed = concat_media::probe(path).expect("footage");
            let video = probed.require_video().expect("a picture").clone();
            let length = probed.duration.map_or(10.0, |d| d.as_f64());
            (path.to_owned(), video.width, video.height, length)
        })
        .collect();
    let (width, height) = std::env::var("CONCAT_PLAY_SIZE")
        .ok()
        .and_then(|size| {
            let (w, h) = size.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or((960, 540));
    let (cut, layers, seconds) = (
        env("CONCAT_PLAY_CUT", 1.0_f64),
        env("CONCAT_PLAY_LAYERS", 1_usize),
        env("CONCAT_PLAY_SECONDS", 8.0_f64),
    );
    let rate = 30.0;
    let mut clips = Vec::new();
    for layer in 0..layers {
        let mut start = 0.0;
        let mut index = layer;
        while start < seconds + 1.0 {
            let (path, w, h, length) = &files[index % files.len()];
            // Somewhere else in the file each time, never running on.
            let from = ((index * 7) % 13) as f64 / 13.0 * (length - cut).max(0.0);
            clips.push(ExportClip {
                path: path.clone(),
                source_start: from,
                media_width: Some(*w),
                media_height: Some(*h),
                media_duration: Some(*length),
                has_audio: Some(false),
                scale: 0.9_f64.powi(layer as i32),
                offset_x: 0.05 * layer as f64,
                ..ExportClip::blank(ClipKind::Video, start, cut, layer)
            });
            start += cut;
            index += 1;
        }
    }
    let clips = Arc::new(clips);
    let settings = DocumentSettings {
        name: "feel".into(),
        width: 1920,
        height: 1080,
        rate_num: 30,
        rate_den: 1,
        canvas: [0, 0, 0, 255],
    };
    let monitor = Monitor::new();
    let spec_at = move |time: f64| FrameSpec {
        time,
        width,
        height,
        moving: true,
        proxy: false,
        color_space: Default::default(),
        canvas: [0, 0, 0, 255],
        nearest: false,
    };
    // The paused frame where play is pressed, as the window has it - and
    // a check that the clips are pictures at all.
    let first = monitor
        .frame_sources(Arc::clone(&clips), &settings, spec_at(0.0))
        .expect("the first frame");
    let pictures = first
        .plan()
        .layers
        .iter()
        .filter(|layer| layer.source.is_some())
        .count();
    assert_eq!(pictures, layers, "every layer decoded at the start");
    let begun = Instant::now();
    let frame_time = Duration::from_secs_f64(1.0 / rate);
    let (mut waits, mut late, mut dropped, mut shown) = (Vec::new(), 0, 0_i64, 0);
    let mut last: Option<i64> = None;
    while begun.elapsed().as_secs_f64() < seconds {
        let now = begun.elapsed().as_secs_f64();
        let index = (now * rate).floor() as i64;
        if last == Some(index) {
            std::thread::sleep(Duration::from_micros(500));
            continue;
        }
        if let Some(last) = last {
            dropped += (index - last - 1).max(0);
        }
        last = Some(index);
        let time = index as f64 / rate;
        let asked = Instant::now();
        monitor
            .frame_sources(Arc::clone(&clips), &settings, spec_at(time))
            .expect("a frame");
        let waited = asked.elapsed();
        if waited > frame_time {
            late += 1;
        }
        waits.push(waited.as_secs_f64() * 1000.0);
        shown += 1;
        // As the window does: the frames ahead, on a thread of their own.
        let (monitor, clips, settings) = (monitor.clone(), Arc::clone(&clips), settings.clone());
        std::thread::spawn(move || {
            monitor.prefetch(
                clips,
                &settings,
                spec_at(time),
                concat_media::prefetch::AHEAD,
            )
        });
    }
    waits.sort_by(f64::total_cmp);
    let at = |q: f64| waits[((waits.len() - 1) as f64 * q).round() as usize];
    eprintln!(
        "play {width}x{height}, {layers} layer(s), a cut every {cut} s: {shown} frames shown, \
         {dropped} dropped, {late} late; waited {:.1} ms median, {:.1} at 90%, {:.1} at 99%, \
         {:.1} at worst",
        at(0.5),
        at(0.9),
        at(0.99),
        at(1.0)
    );
}
