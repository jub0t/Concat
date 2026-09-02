// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Planning and running the audio mix.
//!
//! The audio equivalent of `wolfcut-render`'s frame plan: [`mix_graph`] turns a
//! set of audible clips into one FFmpeg `filter_complex`, as a pure function
//! that is unit-tested without running anything. [`mix_to_file`] and [`mux`]
//! then hand the plan to FFmpeg.
//!
//! This lives in the engine, not the app host, so that there is exactly one
//! definition of what speed, fades and gain mean for sound - the same reason
//! `Clip::source_time_at` is the one definition for picture.

use std::path::Path;

use crate::error::{Error, Result};
use crate::process::{base_command, run_to_completion};

/// Name used in error messages; the binary run comes from [`crate::ffmpeg`].
const FFMPEG: &str = "ffmpeg";

/// The rate everything is mixed at.
pub const MIX_RATE: u32 = 48_000;

/// The playback rates the engine supports, as source-seconds per
/// timeline-second. One definition, shared by preview and export - the two
/// disagreeing at the extremes is how a preview stops matching its render.
pub const SPEED_RANGE: std::ops::RangeInclusive<f64> = 0.0625..=16.0;

/// Clamps a rate into [`SPEED_RANGE`].
pub fn clamp_speed(speed: f64) -> f64 {
    speed.clamp(*SPEED_RANGE.start(), *SPEED_RANGE.end())
}

/// One audible clip, in timeline terms.
#[derive(Clone, Debug)]
pub struct AudioClip {
    /// The media file.
    pub path: std::path::PathBuf,
    /// Where the clip sits on the timeline, in seconds.
    pub start: f64,
    /// How long it runs on the timeline, in seconds.
    pub duration: f64,
    /// Seconds into the source file where the clip begins.
    pub source_start: f64,
    /// Source seconds consumed per timeline second. One is normal.
    pub speed: f64,
    /// True holds pitch while the rate changes; false is the tape effect.
    pub preserve_pitch: bool,
    /// Linear gain, one being unity.
    pub volume: f64,
    /// Fade up over this many seconds from the clip's start.
    pub fade_in: f64,
    /// Fade out over this many seconds into the clip's end.
    pub fade_out: f64,
    /// Extra FFmpeg audio filters, or empty. Validated, not trusted.
    pub filter_chain: String,
}

/// Refuses a filter chain that could break out of its slot in the graph.
///
/// A chain is spliced into a `filter_complex`, where `;` starts a new chain
/// and `[..]` rebinds streams - a chain containing either is no longer a
/// filter applied to this clip, whatever else it might be.
pub fn validate_chain(chain: &str) -> Result<()> {
    let forbidden = |c: char| matches!(c, ';' | '[' | ']' | '\n' | '\r');
    match chain.chars().find(|&c| forbidden(c)) {
        None => Ok(()),
        Some(found) => Err(Error::InvalidFilterChain {
            chain: chain.to_owned(),
            detail: format!("contains {found:?}, which would escape the clip's filter slot"),
        }),
    }
}

/// The filters that retime one clip, in order. Empty at normal speed.
///
/// Pitch-preserving uses `atempo`, staged because one instance only accepts
/// [0.5, 2]. The tape effect first lands on [`MIX_RATE`] so the rate shift is
/// exact whatever the source's rate was, then shifts, then lands back.
pub fn speed_filters(speed: f64, preserve_pitch: bool) -> Vec<String> {
    let speed = clamp_speed(speed);
    if (speed - 1.0).abs() < f64::EPSILON {
        return Vec::new();
    }
    if preserve_pitch {
        tempo_stages(speed).iter().map(|factor| format!("atempo={factor:.6}")).collect()
    } else {
        vec![
            format!("aresample={MIX_RATE}"),
            format!("asetrate={MIX_RATE}*{speed:.6}"),
            format!("aresample={MIX_RATE}"),
        ]
    }
}

/// Splits a rate into `atempo` stages that each stay inside its valid range.
///
/// One filter instance is limited, so 4x has to become 2x then 2x. Multiplying
/// the stages back together is what keeps the result exact.
pub fn tempo_stages(speed: f64) -> Vec<f64> {
    let mut stages = Vec::new();
    let mut remaining = clamp_speed(speed);

    while remaining > 2.0 {
        stages.push(2.0);
        remaining /= 2.0;
    }
    while remaining < 0.5 {
        stages.push(0.5);
        remaining /= 0.5;
    }

    stages.push(remaining);
    stages
}

/// Builds the `filter_complex` that trims, retimes, shapes, delays and mixes
/// every clip into one `[out]` stream of exactly `duration` seconds.
///
/// Input `N` in the command must be `clips[N].path`, and every input must
/// actually contain an audio stream - FFmpeg refuses a graph that names
/// `[N:a]` on a silent input rather than treating it as silence. The caller
/// decides membership by probing; this function decides what the mix means.
pub fn mix_graph(clips: &[AudioClip], duration: f64) -> Result<String> {
    let mut chains: Vec<String> = Vec::new();

    for (index, clip) in clips.iter().enumerate() {
        validate_chain(&clip.filter_chain)?;
        let delay_ms = (clip.start * 1000.0).round().max(0.0) as i64;
        let speed = clamp_speed(clip.speed);

        // Each input is trimmed to its in-point, restamped from zero, then
        // delayed to where it sits on the timeline. `all=1` applies the delay
        // to every channel; without it only the left channel moves, which is
        // a memorable way to discover the flag exists.
        //
        // A sped-up clip covers more source than its timeline length, so the
        // trim takes `duration * speed` before the rate is applied.
        let mut stage = format!(
            "[{index}:a]atrim=start={:.6}:duration={:.6},asetpts=PTS-STARTPTS",
            clip.source_start,
            clip.duration * speed
        );

        for filter in speed_filters(speed, clip.preserve_pitch) {
            stage.push(',');
            stage.push_str(&filter);
        }

        // Filters first: they are what the clip *is* now, and gain and fades
        // are adjustments applied to that result. Reversing the order would
        // let a compressor inside a filter chain undo the fade.
        if !clip.filter_chain.is_empty() {
            stage.push(',');
            stage.push_str(&clip.filter_chain);
        }

        if (clip.volume - 1.0).abs() > f64::EPSILON {
            stage.push_str(&format!(",volume={:.4}", clip.volume.max(0.0)));
        }
        if clip.fade_in > 0.0 {
            stage.push_str(&format!(",afade=t=in:st=0:d={:.4}", clip.fade_in));
        }
        if clip.fade_out > 0.0 {
            let start = (clip.duration - clip.fade_out).max(0.0);
            stage.push_str(&format!(",afade=t=out:st={start:.4}:d={:.4}", clip.fade_out));
        }

        stage.push_str(&format!(",adelay={delay_ms}:all=1[a{index}]"));
        chains.push(stage);
    }

    let inputs: String = (0..clips.len()).map(|index| format!("[a{index}]")).collect();
    let mix = if clips.len() == 1 {
        // amix with one input would still apply its normalisation curve.
        format!("{inputs}anull[mixed]")
    } else {
        format!("{inputs}amix=inputs={}:normalize=0[mixed]", clips.len())
    };

    // The `aresample` is load-bearing, and not about sample rates.
    //
    // `atempo` emits a fractional number of samples per frame, and the leftover
    // rides along in the timestamps it produces. Mixing such a branch with an
    // untouched one gave `amix` an output whose timestamps eventually came out
    // as AV_NOPTS_VALUE, and the muxer rejected the packet:
    //
    //     non monotonically increasing dts to muxer: 9223372036854775807
    //
    // The effect was a two-second mix written as a ~40 ms file - so any export
    // of more than one clip, where any clip had a speed other than 1 with
    // "preserve pitch" on, produced a silently truncated soundtrack. The
    // exporter reported success either way, which is how it went unnoticed.
    //
    // Passing the mix through the resampler regenerates one continuous
    // timestamp series from the sample count and absorbs the drift. Verified
    // against every shape this function emits: both speed directions, the
    // multi-stage `atempo` used beyond 2x, the `asetrate` path, several clips
    // at once, and the single-clip `anull` path.
    //
    // Do not "simplify" this away because the rate on both sides is MIX_RATE.
    chains.push(format!(
        "{mix};[mixed]aresample={MIX_RATE},apad,atrim=duration={duration:.6}[out]"
    ));

    Ok(chains.join(";"))
}

/// Mixes `clips` into one AAC file at `destination`.
///
/// Every clip's file must contain an audio stream; see [`mix_graph`].
pub fn mix_to_file(clips: &[AudioClip], duration: f64, destination: &Path) -> Result<()> {
    let graph = mix_graph(clips, duration)?;

    let mut command = base_command(crate::binaries::ffmpeg());
    command.arg("-y");
    for clip in clips {
        command.arg("-i").arg(&clip.path);
    }
    command
        .args(["-filter_complex", &graph])
        .args(["-map", "[out]"])
        .args(["-c:a", "aac", "-b:a", "192k"])
        .arg(destination);

    run_to_completion(&mut command, FFMPEG, destination)
}

/// Joins an already-encoded video file and audio file into `output`.
///
/// Streams are copied, not re-encoded: the picture is already exactly as
/// asked, and a second pass would cost time and a generation of quality.
pub fn mux(video: &Path, audio: &Path, output: &Path) -> Result<()> {
    let mut command = base_command(crate::binaries::ffmpeg());
    command
        .arg("-y")
        .arg("-i")
        .arg(video)
        .arg("-i")
        .arg(audio)
        .args(["-c:v", "copy", "-c:a", "copy", "-shortest"])
        .args(["-movflags", "+faststart"])
        .arg(output);

    run_to_completion(&mut command, FFMPEG, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(path: &str) -> AudioClip {
        AudioClip {
            path: path.into(),
            start: 0.0,
            duration: 2.0,
            source_start: 0.0,
            speed: 1.0,
            preserve_pitch: true,
            volume: 1.0,
            fade_in: 0.0,
            fade_out: 0.0,
            filter_chain: String::new(),
        }
    }

    #[test]
    fn tempo_stages_multiply_back_to_the_requested_rate() {
        for speed in [0.25, 0.5, 0.9, 1.0, 1.5, 2.0, 3.0, 4.0, 8.0] {
            let product: f64 = tempo_stages(speed).iter().product();
            assert!(
                (product - speed).abs() < 1e-9,
                "stages for {speed} multiplied to {product}",
            );
        }
    }

    #[test]
    fn every_stage_is_within_the_filter_range() {
        for speed in [0.0625, 0.25, 0.5, 1.0, 4.0, 8.0, 16.0] {
            for factor in tempo_stages(speed) {
                assert!(
                    (0.5..=2.0).contains(&factor),
                    "{factor} is outside atempo's range, from speed {speed}",
                );
            }
        }
    }

    #[test]
    fn out_of_range_speeds_clamp_rather_than_explode() {
        assert_eq!(clamp_speed(0.0), 0.0625);
        assert_eq!(clamp_speed(100.0), 16.0);
        assert_eq!(clamp_speed(1.0), 1.0);
    }

    #[test]
    fn normal_speed_needs_no_filters() {
        assert!(speed_filters(1.0, true).is_empty());
        assert!(speed_filters(1.0, false).is_empty());
    }

    #[test]
    fn the_tape_effect_lands_on_the_mix_rate_before_shifting() {
        let filters = speed_filters(2.0, false);
        assert_eq!(filters[0], "aresample=48000", "the shift must start from a known rate");
        assert!(filters[1].starts_with("asetrate=48000*"));
        assert_eq!(filters[2], "aresample=48000");
    }

    #[test]
    fn a_single_clip_avoids_amix_normalisation() {
        let graph = mix_graph(&[clip("a.mp4")], 2.0).expect("valid");
        assert!(graph.contains("anull[mixed]"), "graph was: {graph}");
        assert!(!graph.contains("amix"), "graph was: {graph}");
    }

    #[test]
    fn several_clips_mix_without_normalisation() {
        let graph = mix_graph(&[clip("a.mp4"), clip("b.mp4")], 3.0).expect("valid");
        assert!(graph.contains("amix=inputs=2:normalize=0"), "graph was: {graph}");
        assert!(graph.contains("[0:a]") && graph.contains("[1:a]"), "graph was: {graph}");
    }

    #[test]
    fn the_final_stage_pads_and_trims_to_the_timeline_duration() {
        let graph = mix_graph(&[clip("a.mp4")], 4.5).expect("valid");
        assert!(
            graph.ends_with("aresample=48000,apad,atrim=duration=4.500000[out]"),
            "graph was: {graph}"
        );
    }

    #[test]
    fn a_sped_up_clip_trims_more_source_than_its_timeline_length() {
        let mut fast = clip("a.mp4");
        fast.speed = 2.0;
        let graph = mix_graph(&[fast], 2.0).expect("valid");
        assert!(graph.contains("duration=4.000000"), "graph was: {graph}");
        assert!(graph.contains("atempo=2.000000"), "graph was: {graph}");
    }

    #[test]
    fn fades_and_volume_apply_after_the_filter_chain() {
        let mut shaped = clip("a.mp4");
        shaped.filter_chain = "highpass=f=80".to_owned();
        shaped.volume = 0.5;
        shaped.fade_out = 0.25;
        let graph = mix_graph(&[shaped], 2.0).expect("valid");

        let chain_at = graph.find("highpass").expect("chain present");
        let volume_at = graph.find(",volume=").expect("volume present");
        let fade_at = graph.find(",afade=t=out").expect("fade present");
        assert!(chain_at < volume_at && volume_at < fade_at, "graph was: {graph}");
    }

    #[test]
    fn a_chain_that_escapes_its_slot_is_refused() {
        for bad in ["volume=2;amovie=x[a]", "anull[a1]", "a\nb"] {
            let mut hostile = clip("a.mp4");
            hostile.filter_chain = bad.to_owned();
            assert!(
                matches!(mix_graph(&[hostile], 2.0), Err(Error::InvalidFilterChain { .. })),
                "{bad:?} should have been refused",
            );
        }
    }

    #[test]
    fn ordinary_chains_pass_validation() {
        for good in ["", "highpass=f=80,equalizer=f=300:t=q:w=1.1:g=-1.4", "volume=0.5"] {
            assert!(validate_chain(good).is_ok(), "{good:?} should be fine");
        }
    }
}
