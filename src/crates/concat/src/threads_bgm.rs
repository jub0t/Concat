// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! Royalty-free procedural ambient BGM loops for Threads-to-TikTok timelines.

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

const SAMPLE_RATE: u32 = 44_100;
const CHANNELS: u16 = 2;
const CROSSFADE_SECONDS: f32 = 0.25;

/// Built-in and custom background music choices surfaced in the Threads wizard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BgmPreset {
    Off,
    #[default]
    LoFiChill,
    StoryUpbeat,
    DramaticTea,
    Custom,
}

impl BgmPreset {
    pub const ALL: &'static [BgmPreset] = &[
        BgmPreset::Off,
        BgmPreset::LoFiChill,
        BgmPreset::StoryUpbeat,
        BgmPreset::DramaticTea,
        BgmPreset::Custom,
    ];

    pub fn from_index(index: i32) -> Self {
        Self::ALL
            .get(usize::try_from(index).unwrap_or(0))
            .copied()
            .unwrap_or(Self::Off)
    }

    pub fn index(self) -> i32 {
        match self {
            Self::Off => 0,
            Self::LoFiChill => 1,
            Self::StoryUpbeat => 2,
            Self::DramaticTea => 3,
            Self::Custom => 4,
        }
    }

    pub fn slug(self) -> Option<&'static str> {
        match self {
            Self::LoFiChill => Some("lofi-chill"),
            Self::StoryUpbeat => Some("story-upbeat"),
            Self::DramaticTea => Some("dramatic-tea"),
            Self::Off | Self::Custom => None,
        }
    }
}

/// Ensures the requested built-in BGM preset WAV exists on disk and returns its path.
pub fn ensure_preset_wav(data_dir: &Path, preset: BgmPreset) -> Result<PathBuf, String> {
    let Some(slug) = preset.slug() else {
        return Err("BGM preset does not map to a built-in loop.".to_owned());
    };
    let dir = data_dir.join("threads-bgm");
    std::fs::create_dir_all(&dir)
        .map_err(|error| format!("Could not create threads-bgm folder: {error}"))?;
    let path = dir.join(format!("{slug}-v1.wav"));
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > 44 {
            return Ok(path);
        }
    }

    let wav = synthesize_preset_wav(preset)?;
    let tmp = path.with_extension("wav.tmp");
    std::fs::write(&tmp, &wav)
        .map_err(|error| format!("Could not write temporary BGM file: {error}"))?;
    std::fs::rename(&tmp, &path)
        .or_else(|_| {
            std::fs::copy(&tmp, &path)?;
            let _ = std::fs::remove_file(&tmp);
            Ok::<_, std::io::Error>(())
        })
        .map_err(|error| format!("Could not finalize BGM file: {error}"))?;
    Ok(path)
}

/// Synthesizes a seamless stereo 16-bit PCM WAV loop for the given preset.
pub fn synthesize_preset_wav(preset: BgmPreset) -> Result<Vec<u8>, String> {
    let samples = match preset {
        BgmPreset::LoFiChill => render_lofi_chill(),
        BgmPreset::StoryUpbeat => render_story_upbeat(),
        BgmPreset::DramaticTea => render_dramatic_tea(),
        BgmPreset::Off | BgmPreset::Custom => {
            return Err("No built-in waveform for this BGM option.".to_owned());
        }
    };
    Ok(encode_wav_stereo(&samples, SAMPLE_RATE))
}

fn render_lofi_chill() -> Vec<[f32; 2]> {
    // 75 BPM -> 0.8s/beat, 4 bars * 4 beats = 12.8s seamless loop.
    let beat_sec = 0.8f32;
    let total_beats = 16usize;
    let loop_sec = beat_sec * total_beats as f32;
    let chords: [[f32; 4]; 4] = [
        [174.61, 220.00, 261.63, 329.63], // Fmaj7
        [164.81, 196.00, 246.94, 293.66], // Em7
        [146.83, 174.61, 220.00, 261.63], // Dm7
        [130.81, 164.81, 196.00, 246.94], // Cmaj7
    ];

    render_loop_with_tail(loop_sec, |t| {
        let wrapped = t.rem_euclid(loop_sec);
        let chord_sec = beat_sec * 4.0;
        let chord_idx = ((wrapped / chord_sec).floor() as usize).min(3);
        let local_t = wrapped - chord_idx as f32 * chord_sec;

        // Warm electric piano chord envelope: soft attack, gentle decay.
        let env = (1.0 - (-local_t * 28.0).exp()) * (-local_t * 0.45).exp()
            * smoothstep_edge(chord_sec - local_t, 0.08);
        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for (i, &freq) in chords[chord_idx].iter().enumerate() {
            let detune = (i as f32 - 1.5) * 0.25;
            let s_l = warm_rhodes(t, freq + detune);
            let s_r = warm_rhodes(t, freq - detune);
            let w = if i == 0 { 0.30 } else { 0.21 };
            l += s_l * w * env;
            r += s_r * w * env;
        }

        // Soft sub-bass thump on beat 1 & 3 of every bar.
        let beat_pos = wrapped / beat_sec;
        let beat_in_bar = beat_pos.rem_euclid(4.0);
        let beat_frac = beat_pos.fract();
        let kick_env = if beat_in_bar < 1.0 || (beat_in_bar >= 2.0 && beat_in_bar < 3.0) {
            let kt = beat_frac * beat_sec;
            (-kt * 13.0).exp() * (1.0 - (-kt * 160.0).exp())
        } else {
            0.0
        };
        let kick = (TAU * 54.0 * t + 1.4 * (-beat_frac * 9.0).exp()).sin() * kick_env * 0.22;
        l += kick;
        r += kick;

        // Gentle offbeat brushed click for tempo pocket.
        let half_pos = (wrapped / (beat_sec * 0.5)).fract();
        let brush_env = (-half_pos * beat_sec * 38.0).exp() * (1.0 - (-half_pos * 120.0).exp());
        let brush = prng_noise((t * SAMPLE_RATE as f32) as u32) * brush_env * 0.032;
        l += brush * 0.85;
        r += brush * 1.15;

        [l * 0.65, r * 0.65]
    })
}

fn render_story_upbeat() -> Vec<[f32; 2]> {
    // 96 BPM -> 0.625s/beat, 4 bars * 4 beats = 10.0s seamless loop.
    let beat_sec = 0.625f32;
    let loop_sec = beat_sec * 16.0;
    let arp_chords: [[f32; 4]; 4] = [
        [261.63, 329.63, 392.00, 493.88], // Cmaj7
        [220.00, 261.63, 329.63, 392.00], // Am7
        [174.61, 220.00, 261.63, 329.63], // Fmaj7
        [196.00, 246.94, 293.66, 392.00], // G6
    ];

    render_loop_with_tail(loop_sec, |t| {
        let wrapped = t.rem_euclid(loop_sec);
        let chord_sec = beat_sec * 4.0;
        let chord_idx = ((wrapped / chord_sec).floor() as usize).min(3);
        let local_t = wrapped - chord_idx as f32 * chord_sec;

        // Warm air pad foundation (lower octave).
        let pad_env = smoothstep_edge(local_t, 0.14) * smoothstep_edge(chord_sec - local_t, 0.14);
        let root = arp_chords[chord_idx][0] * 0.5;
        let fifth = arp_chords[chord_idx][2] * 0.5;
        let pad_l = (warm_pad(t, root + 0.3) * 0.18 + warm_pad(t, fifth - 0.2) * 0.14) * pad_env;
        let pad_r = (warm_pad(t, root - 0.3) * 0.18 + warm_pad(t, fifth + 0.2) * 0.14) * pad_env;

        // Soft felt-marimba eighth-note arpeggio.
        let step_sec = beat_sec * 0.5;
        let step_idx = (wrapped / step_sec).floor() as usize;
        let step_t = wrapped - step_idx as f32 * step_sec;
        let note = arp_chords[chord_idx][step_idx % 4];
        let pluck_env = (1.0 - (-step_t * 95.0).exp()) * (-step_t * 6.8).exp();
        let pan = if step_idx % 2 == 0 { 0.42 } else { 0.58 };
        let pluck = ((TAU * note * t).sin() + 0.18 * (TAU * note * 2.0 * t).sin())
            * pluck_env
            * 0.28;

        // Light muted pulse every beat.
        let beat_t = (wrapped / beat_sec).fract() * beat_sec;
        let pulse = (TAU * 72.0 * t).sin() * (-beat_t * 18.0).exp() * 0.14;

        [
            (pad_l + pluck * (1.0 - pan) + pulse) * 0.72,
            (pad_r + pluck * pan + pulse) * 0.72,
        ]
    })
}

fn render_dramatic_tea() -> Vec<[f32; 2]> {
    // 90 BPM -> 0.6667s/beat, 4 bars * 4 beats = 10.6667s loop.
    let beat_sec = 2.0 / 3.0;
    let loop_sec = beat_sec * 16.0;
    let chords: [[f32; 3]; 4] = [
        [110.00, 164.81, 261.63], // Am
        [87.31, 130.81, 220.00],  // F
        [146.83, 174.61, 220.00], // Dm
        [123.47, 164.81, 246.94], // Esus4 / tension
    ];
    let bell_notes = [523.25f32, 440.00, 392.00, 493.88];

    render_loop_with_tail(loop_sec, |t| {
        let wrapped = t.rem_euclid(loop_sec);
        let chord_sec = beat_sec * 4.0;
        let chord_idx = ((wrapped / chord_sec).floor() as usize).min(3);
        let local_t = wrapped - chord_idx as f32 * chord_sec;
        let swell = smoothstep_edge(local_t, 0.28) * smoothstep_edge(chord_sec - local_t, 0.22);

        let mut l = 0.0f32;
        let mut r = 0.0f32;
        for (i, &f) in chords[chord_idx].iter().enumerate() {
            let det = (i as f32 - 1.0) * 0.35;
            let w = if i == 0 { 0.26 } else { 0.17 };
            l += warm_pad(t, f + det) * w * swell;
            r += warm_pad(t, f - det) * w * swell;
        }

        // Sparse glass chime accent at the start of each bar.
        let bell_f = bell_notes[chord_idx];
        let bell_env = (1.0 - (-local_t * 80.0).exp()) * (-local_t * 2.2).exp();
        let bell = ((TAU * bell_f * t).sin() + 0.15 * (TAU * bell_f * 2.0 * t).sin())
            * bell_env
            * 0.12;
        l += bell * 0.65;
        r += bell * 1.05;

        // Heartbeat double-pulse on beat 1 & beat 1.5 of every bar.
        let bar_t = wrapped - (chord_idx as f32 * chord_sec);
        let hb1 = (-bar_t * 14.0).exp() * (TAU * 48.0 * t).sin() * 0.22;
        let hb2_t = (bar_t - beat_sec * 0.75).max(0.0);
        let hb2 = if bar_t >= beat_sec * 0.75 {
            (-hb2_t * 16.0).exp() * (TAU * 45.0 * t).sin() * 0.14
        } else {
            0.0
        };
        l += hb1 + hb2;
        r += hb1 + hb2;

        [l * 0.68, r * 0.68]
    })
}

fn render_loop_with_tail(
    loop_sec: f32,
    sample_at: impl Fn(f32) -> [f32; 2],
) -> Vec<[f32; 2]> {
    let total_frames = (loop_sec * SAMPLE_RATE as f32).round().max(1.0) as usize;
    let fade_frames = ((CROSSFADE_SECONDS * SAMPLE_RATE as f32).round() as usize).min(total_frames / 4);
    let mut out = Vec::with_capacity(total_frames);

    for i in 0..total_frames {
        let t = i as f32 / SAMPLE_RATE as f32;
        let mut s = sample_at(t);
        // Crossfade the end tail into the loop head so repeat boundaries never click.
        if i < fade_frames {
            let alpha = i as f32 / fade_frames as f32;
            let tail = sample_at(loop_sec + t);
            s[0] = s[0] * alpha + tail[0] * (1.0 - alpha);
            s[1] = s[1] * alpha + tail[1] * (1.0 - alpha);
        }
        // Gentle soft limiter around -8 dBFS so BGM mixes cleanly under voice.
        out.push([(s[0] * 0.85).tanh() * 0.45, (s[1] * 0.85).tanh() * 0.45]);
    }
    out
}

fn warm_rhodes(t: f32, freq: f32) -> f32 {
    (TAU * freq * t).sin() * 0.78
        + (TAU * freq * 2.0 * t).sin() * 0.17
        + (TAU * freq * 3.0 * t).sin() * 0.05
}

fn warm_pad(t: f32, freq: f32) -> f32 {
    (TAU * freq * t).sin() * 0.72
        + (TAU * freq * 2.0 * t).sin() * 0.20
        + (TAU * freq * 0.5 * t).sin() * 0.08
}

fn smoothstep_edge(dist: f32, width: f32) -> f32 {
    let x = (dist / width.max(0.001)).clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

fn prng_noise(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(747_796_405).wrapping_add(2_891_336_453);
    x = ((x >> ((x >> 28).wrapping_add(4))) ^ x).wrapping_mul(277_803_737);
    x = (x >> 22) ^ x;
    (x as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn encode_wav_stereo(frames: &[[f32; 2]], sample_rate: u32) -> Vec<u8> {
    let data_bytes = (frames.len() * CHANNELS as usize * 2) as u32;
    let byte_rate = sample_rate * u32::from(CHANNELS) * 2;
    let block_align = CHANNELS * 2;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&CHANNELS.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for [l, r] in frames {
        let li = (l.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        let ri = (r.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16;
        out.extend_from_slice(&li.to_le_bytes());
        out.extend_from_slice(&ri.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesizes_valid_non_clipping_stereo_wavs_for_all_presets() {
        for preset in [
            BgmPreset::LoFiChill,
            BgmPreset::StoryUpbeat,
            BgmPreset::DramaticTea,
        ] {
            let wav = synthesize_preset_wav(preset).expect("preset synthesizes");
            assert!(wav.starts_with(b"RIFF"));
            assert_eq!(&wav[8..12], b"WAVE");
            assert!(wav.len() > 44 + 44_100 * 4 * 8);

            let peak = wav[44..]
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]).unsigned_abs())
                .max()
                .unwrap_or(0);
            assert!(peak > 500 && peak < (i16::MAX as u16) - 100);
        }
    }

    #[test]
    fn ensure_preset_wav_caches_to_disk() {
        let tmp = std::env::temp_dir().join(format!("concat_bgm_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let first = ensure_preset_wav(&tmp, BgmPreset::LoFiChill).expect("writes file");
        assert!(first.exists());
        let len = std::fs::metadata(&first).unwrap().len();
        assert!(len > 44);
        let second = ensure_preset_wav(&tmp, BgmPreset::LoFiChill).expect("reuses cache");
        assert_eq!(first, second);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
