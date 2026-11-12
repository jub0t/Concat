// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A made-up run, to see what the log looks like without starting the app:
//!
//! ```text
//! cargo run -p concat-log --example demo
//! ```
//!
//! The lines go to the terminal, coloured, and to a file in the system's
//! temporary folder, which is printed at the end - the two a real run
//! writes. Change `line.rs` and run it again to see the change.

use std::time::Duration;

fn main() {
    let folder = std::env::temp_dir().join("concat-log-demo");
    let (filter, _) = concat_log::Filter::parse(
        &std::env::var(concat_log::LEVEL_VARIABLE).unwrap_or_else(|_| "all=debug".to_owned()),
    );
    let opened = concat_log::open(concat_log::Config {
        folder: Some(folder),
        filter,
        platform: None,
        home: std::env::var_os("HOME").map(Into::into),
    })
    .expect("the only logger in this process");
    concat_log::catch_panics();

    log::info!("build: Concat 0.2.10 · macos-aarch64 · commit 784021e7d9d7 · optimised build");
    log::info!("system: macOS 26.1");
    log::info!("cpu: Apple M5 · 10 threads");
    log::info!("memory: {}", concat_log::bytes(32 << 30));
    log::warn!(
        "last run: concat-20261110-212508.log did not end cleanly (it crashed, was killed, or the machine went down); the last thing that went wrong in it: FATAL [main] crash: SIGSEGV (bad memory access) at 0x10 in libonnxruntime.dylib+0x1a2b3c"
    );
    log::info!("gpu: Apple M5 · integrated · Metal");
    log::info!("display: 3024x1964 at 2x · window 1512x945");
    log::info!("audio: MacBook Pro Speakers · 48 kHz stereo");

    let mut start = concat_log::op!("start");
    start.step("fonts");
    pause(40);
    start.step("effects");
    pause(60);
    start.step("window");
    pause(90);
    start.done("the window is up");

    let home = std::env::var("HOME").unwrap_or_default();
    log::info!(
        "project: opened {home}/Movies/Trip to Hunza in 212 ms · 3 tracks, 48 clips, 2 m 14 s"
    );
    log::info!(
        "import: IMG_2231.MOV · 3840x2160 60 fps HEVC 10-bit · 38.2 s · {}",
        concat_log::bytes(431_702_016)
    );
    log::warn!("decode: IMG_2231.MOV is on the CPU: the GPU's decoder refused HEVC 4:2:2");
    log::debug!("decode: IMG_2231.MOV seeks 4.2 s apart; a scrub may wait on up to 250 frames");
    log::info!("edit: Split at 00:00:12:14");
    log::info!("settings: Draw on the GPU off");

    let mut captions = concat_log::op!("captions");
    captions.note(format_args!(
        "small model, {} of sound",
        concat_log::span(Duration::from_secs(134))
    ));
    captions.step("load model");
    pause(120);
    captions.step("find speech");
    pause(80);
    captions.step("transcribe");
    pause(400);
    captions.done("120 captions");

    for _ in 0..400 {
        log::error!("gpu: the compositor has no device to draw on");
    }
    log::error!(
        "gpu: the compositor's device was lost: the driver stopped answering; drawing in software from here"
    );

    let mut export = concat_log::op!("export");
    export.note("1920x1080 30 fps H.264, h264_videotoolbox, 8 Mb/s");
    export.step("draw and encode");
    pause(150);
    export.failed("the disk is full; the file was not written");

    log::info!("close: by the window's close button");
    concat_log::end();
    if let Some(path) = opened.file {
        eprintln!("\nthe same run, as its file has it: {}", path.display());
    }
}

fn pause(millis: u64) {
    std::thread::sleep(Duration::from_millis(millis));
}
