// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The export pipeline, end to end, against the FFmpeg the app actually ships.
//!
//! The engine's unit tests are pure on purpose; this suite is the opposite. It
//! generates real fixtures, runs `export::render` - decoders, compositor,
//! encoder, audio graph, mux - and probes what came out. It exists because the
//! bugs that reach users live exactly here: a protocol missing from the
//! trimmed FFmpeg build, a filtergraph naming a stream that does not exist.
//!
//! Fixtures are generated with the *system* FFmpeg (the trimmed one has no
//! lavfi), and the export itself runs against the *bundled* pair when it is
//! staged in `ffmpeg/` - so a component missing from the shipped build fails
//! this test instead of a user's export. No system FFmpeg? The suite skips.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

use concat_desktop_lib::export::{ClipKind, ExportClip, ExportRequest, Reporter, render};

/// Points the engine at the staged bundle exactly once, before anything runs.
fn use_bundled_pair() {
    let staged = Path::new(env!("CARGO_MANIFEST_DIR")).join("ffmpeg");
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let ffmpeg = staged.join(format!("ffmpeg{suffix}"));
    let ffprobe = staged.join(format!("ffprobe{suffix}"));
    if ffmpeg.is_file() && ffprobe.is_file() {
        concat_media::set_binaries(ffmpeg, ffprobe);
    }
}

/// True when a system FFmpeg exists to generate fixtures with.
fn system_ffmpeg() -> bool {
    Command::new("ffmpeg")
        .arg("-version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Writes a fixture with the system FFmpeg. `lavfi` is unavailable in the
/// bundled build, which is exactly why fixtures do not go through it.
fn fixture(directory: &Path, name: &str, args: &[&str]) -> PathBuf {
    let path = directory.join(name);
    let status = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .arg(&path)
        .status()
        .expect("system ffmpeg runs");
    assert!(status.success(), "could not generate fixture {name}");
    path
}

fn media_clip(path: &Path, kind: &str, start: f64, duration: f64, track: usize) -> ExportClip {
    ExportClip {
        path: path.to_string_lossy().into_owned(),
        kind: match kind {
            "audio" => ClipKind::Audio,
            "image" => ClipKind::Image,
            _ => ClipKind::Video,
        },
        start,
        duration,
        source_start: 0.0,
        track,
        hidden: false,
        muted: false,
        volume: 1.0,
        fade_in: 0.0,
        fade_out: 0.0,
        filter_chain: String::new(),
        speed: 1.0,
        preserve_pitch: true,
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        rotation: 0.0,
        opacity: 1.0,
        video_filter_chain: String::new(),
        transition: None,
        video_fade_in: 0.0,
        media_width: Some(320),
        media_height: Some(180),
        // Deliberately absent: this suite exists to exercise the probe
        // fallback and the graph-membership rules against real files.
        has_audio: None,
    }
}

fn request(output: &Path, clips: Vec<ExportClip>) -> ExportRequest {
    ExportRequest {
        output: output.to_string_lossy().into_owned(),
        width: 320,
        height: 180,
        rate_num: 30,
        rate_den: 1,
        crf: 30,
        preset: "ultrafast".to_owned(),
        clips,
    }
}

fn export(request: &ExportRequest) -> Result<String, String> {
    let cancel = AtomicBool::new(false);
    let mut progress = |_: i64, _: i64, _: &'static str| {};
    render(request, Reporter { progress: &mut progress, cancel: &cancel })
}

/// One suite rather than many `#[test]`s: fixtures are cheap but not free,
/// and `set_binaries` is process-global either way.
#[test]
fn exports_survive_the_shapes_that_have_actually_broken() {
    if !system_ffmpeg() {
        eprintln!("no system ffmpeg; skipping the export integration suite");
        return;
    }
    use_bundled_pair();

    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("export-integration");
    std::fs::create_dir_all(&directory).expect("tmp dir");

    let with_sound = fixture(
        &directory,
        "with-sound.mp4",
        &[
            "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2",
            "-f", "lavfi", "-i", "sine=frequency=440:duration=2",
            "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
        ],
    );
    let silent_video = fixture(
        &directory,
        "silent.mp4",
        &[
            "-f", "lavfi", "-i", "testsrc2=size=320x180:rate=30:duration=2",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
        ],
    );
    let still = fixture(
        &directory,
        "still.png",
        &["-f", "lavfi", "-i", "color=red:size=320x180", "-frames:v", "1"],
    );

    // The everyday case: one clip with picture and sound.
    let output = directory.join("plain.mp4");
    export(&request(&output, vec![media_clip(&with_sound, "video", 0.0, 2.0, 0)]))
        .expect("plain export");
    let info = concat_media::probe(&output).expect("probe plain");
    assert!(info.video.is_some() && info.audio.is_some());
    let duration = info.duration.expect("has duration").as_f64();
    assert!((duration - 2.0).abs() < 0.15, "expected ~2s, got {duration}");

    // The regression that shipped: an audioless video in the mix. The graph
    // must not name a stream the file does not have.
    let output = directory.join("mixed-mute.mp4");
    export(&request(
        &output,
        vec![
            media_clip(&silent_video, "video", 0.0, 2.0, 0),
            media_clip(&with_sound, "video", 0.5, 1.0, 1),
        ],
    ))
    .expect("export with an audioless clip");
    let info = concat_media::probe(&output).expect("probe mixed-mute");
    assert!(info.audio.is_some(), "the sounded clip still reaches the mix");

    // Retiming: a 2x clip consumes 2s of source in 1s of timeline, and
    // picture and sound must agree on that.
    let output = directory.join("fast.mp4");
    let mut fast = media_clip(&with_sound, "video", 0.0, 1.0, 0);
    fast.speed = 2.0;
    export(&request(&output, vec![fast])).expect("2x export");
    let info = concat_media::probe(&output).expect("probe fast");
    let duration = info.duration.expect("has duration").as_f64();
    assert!((duration - 1.0).abs() < 0.15, "expected ~1s at 2x, got {duration}");
    assert!(info.audio.is_some(), "the sped-up sound made it through the graph");

    // A still overlay above footage: the image decoder loops, the compositor
    // stacks, and the export still carries the footage's sound.
    let output = directory.join("overlay.mp4");
    let mut overlay = media_clip(&still, "image", 0.0, 2.0, 1);
    overlay.muted = true;
    export(&request(
        &output,
        vec![media_clip(&with_sound, "video", 0.0, 2.0, 0), overlay],
    ))
    .expect("overlay export");
    let info = concat_media::probe(&output).expect("probe overlay");
    assert!(info.video.is_some() && info.audio.is_some());

    // A clip whose filter chain tries to escape its slot is refused, not run.
    let output = directory.join("hostile.mp4");
    let mut hostile = media_clip(&with_sound, "video", 0.0, 1.0, 0);
    hostile.filter_chain = "anull[out];[0:v]null".to_owned();
    let refused = export(&request(&output, vec![hostile]));
    assert!(refused.is_err(), "a graph-escaping chain must be refused");

    // Cancellation aborts cleanly instead of writing a file.
    let output = directory.join("cancelled.mp4");
    let cancelled = AtomicBool::new(true);
    let mut progress = |_: i64, _: i64, _: &'static str| {};
    let result = render(
        &request(&output, vec![media_clip(&with_sound, "video", 0.0, 2.0, 0)]),
        Reporter { progress: &mut progress, cancel: &cancelled },
    );
    assert_eq!(result, Err("export cancelled".to_owned()));
    assert!(!output.exists(), "a cancelled export must not leave a file");
}
