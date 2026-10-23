// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Voices the user cloned and kept: the Audio page's Voices shelf, and
//! the rows of the text-to-speech voice picker past the built-in ones.
//!
//! A clone is a recording read as a voice by a model that reads in one
//! (Pocket TTS, Chatterbox). Kept, it is the few seconds of the recording
//! the model listens to, cut out and written as a WAV of its own - not the
//! file it came from, which may be a long video in one project - beside a
//! `voice.json` with its name and, when given them, a picture and a line
//! about it:
//!
//! ```text
//! <library>/voices/<id>/voice.json
//! <library>/voices/<id>/sample.wav
//! <library>/voices/<id>/picture.png
//! ```
//!
//! It stays on this machine: nothing here is uploaded or shared, and a
//! project that read in a saved voice holds only the speech it wrote.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::AppDirs;

/// How much of the recording is kept: a little over what any model
/// listens to (Pocket ten seconds, Chatterbox at least five), so a voice
/// is the same voice whichever reads it.
pub const SAMPLE_SECONDS: f64 = 12.0;

/// The rate the sample is kept at: what the models read voices at.
const SAMPLE_RATE: u32 = 24_000;

/// The largest picture taken, in bytes: a face for a circle the size of a
/// thumbnail does not need more.
const PICTURE_CAP: u64 = 8 * 1024 * 1024;

/// The pictures a voice can wear.
pub const PICTURE_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// The recordings a voice can be imported from: the sound files the
/// library takes, and the videos the bin takes, for their soundtrack.
pub const RECORDING_EXTENSIONS: [&str; 15] = [
    "mp3", "wav", "aac", "m4a", "flac", "ogg", "opus", "aif", "aiff", "mp4", "mov", "mkv", "webm",
    "avi", "m4v",
];

/// The most characters a voice's description keeps: a line under its
/// name, not a biography.
pub const DESCRIPTION_CHARS: usize = 140;

/// The frame a recording is measured in when its voice is looked for, in
/// seconds: about the shortest sound of a syllable, the usual frame of a
/// voice detector.
const FRAME_SECONDS: f64 = 0.02;

/// How loud a frame must be to count as speech, against the loud end of
/// the recording (see [`densest`]): twenty decibels under it. Relative, so
/// a quiet take and a hot one are read alike, and the hiss of a room
/// between the sentences is not taken for them.
const SPEECH_FLOOR: f32 = 0.1;

/// A recording whose loud end is under this, about eighty decibels down,
/// is silent: there is no voice in it to look for.
const SILENCE: f32 = 1e-4;

/// One saved voice.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedVoice {
    /// Its folder's name.
    pub id: String,
    /// What the user called it.
    pub name: String,
    /// The picture's file name in its folder, when it has one.
    #[serde(default)]
    pub picture: Option<String>,
    /// A line about it, when the user wrote one: what the shelf says
    /// under its name.
    #[serde(default)]
    pub description: Option<String>,
    /// When it was saved, as Unix seconds: the shelf's order, newest first.
    #[serde(default)]
    pub created: u64,
    /// The folder, filled in when read; not written.
    #[serde(skip)]
    pub dir: PathBuf,
}

impl SavedVoice {
    /// The recording the voice is read from.
    pub fn sample(&self) -> PathBuf {
        self.dir.join("sample.wav")
    }

    /// The picture, when there is one.
    pub fn picture_path(&self) -> Option<PathBuf> {
        self.picture.as_ref().map(|file| self.dir.join(file))
    }
}

/// Where saved voices live on this machine.
pub fn dir_in(dirs: &AppDirs) -> PathBuf {
    dirs.library().join("voices")
}

/// Every saved voice in `dir`, newest first. A folder without its sample
/// or its `voice.json` is not a voice and is left out.
pub fn list(dir: &Path) -> Vec<SavedVoice> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut voices: Vec<SavedVoice> = entries
        .flatten()
        .filter_map(|entry| {
            let folder = entry.path();
            let bytes = std::fs::read(folder.join("voice.json")).ok()?;
            let mut voice: SavedVoice = serde_json::from_slice(&bytes).ok()?;
            voice.dir = folder;
            let plain = voice
                .picture
                .as_deref()
                .is_none_or(|file| Path::new(file).file_name().is_some_and(|name| name == file));
            (plain && voice.sample().is_file()).then_some(voice)
        })
        .collect();
    voices.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.name.cmp(&b.name)));
    voices
}

/// Saves `name` as a voice: [`SAMPLE_SECONDS`] of `source` from `start`,
/// mono, written as a WAV of its own, and `picture` copied beside it. The
/// description is kept trimmed and cut to [`DESCRIPTION_CHARS`]; a blank
/// one is none. Slow - a decode - so a worker's.
pub fn save(
    dir: &Path,
    name: &str,
    source: &Path,
    start: f64,
    picture: Option<&Path>,
    description: Option<&str>,
) -> Result<SavedVoice, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a voice needs a name".to_owned());
    }
    let description = description
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.chars().take(DESCRIPTION_CHARS).collect::<String>());
    let options = concat_media::AudioOptions {
        start: (start.is_finite() && start > 0.0).then_some(start),
        duration: Some(SAMPLE_SECONDS),
        rate: SAMPLE_RATE,
        channels: 1,
        format: concat_media::SampleFormat::F32,
        ..Default::default()
    };
    let samples = concat_media::AudioDecoder::open(source, &options)
        .and_then(|mut decoder| decoder.collect_f32())
        .map_err(|error| {
            format!(
                "could not read the voice from {}: {error}",
                source.display()
            )
        })?;
    // A second is the least a voice can be told from.
    if samples.len() < SAMPLE_RATE as usize {
        return Err(format!(
            "too little sound in {} to keep as a voice",
            source.display()
        ));
    }
    let created = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let id = format!(
        "{:012x}",
        crate::media::fnv1a(format!("{name}|{created}|{}", source.display()).as_bytes())
            & 0xffff_ffff_ffff
    );
    let folder = dir.join(&id);
    // Written into a staging folder and renamed, so a voice half-saved is
    // never listed.
    let staging = dir.join(format!(".staging-{id}"));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .map_err(|error| format!("could not create {}: {error}", staging.display()))?;
    let written = (|| {
        std::fs::write(staging.join("sample.wav"), wav(&samples, SAMPLE_RATE))
            .map_err(|error| format!("could not write the voice: {error}"))?;
        let picture = match picture {
            Some(path) => Some(copy_picture(path, &staging)?),
            None => None,
        };
        let voice = SavedVoice {
            id: id.clone(),
            name: name.to_owned(),
            picture,
            description,
            created,
            dir: folder.clone(),
        };
        let json = serde_json::to_vec_pretty(&voice).map_err(|error| error.to_string())?;
        std::fs::write(staging.join("voice.json"), json)
            .map_err(|error| format!("could not write the voice: {error}"))?;
        std::fs::rename(&staging, &folder)
            .map_err(|error| format!("could not write the voice: {error}"))?;
        Ok(voice)
    })();
    if written.is_err() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    written
}

/// What listening to a recording found: how long it is, and where in it
/// the [`SAMPLE_SECONDS`] most given to speech start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Heard {
    /// The recording's length, in seconds.
    pub duration: f64,
    /// Seconds in where the kept sample is best taken from.
    pub start: f64,
}

/// Listens to `source` for where its voice is, so a recording that opens
/// on silence, a room or a jingle is kept from where the speaking is and
/// not from its first second. Its sound is decoded as [`save`] decodes it,
/// mono at the models' rate, and measured a [`FRAME_SECONDS`] frame at a
/// time as it comes, so an hour of video costs a few hundred thousand
/// levels rather than the hour's samples; see [`densest`] for the window
/// taken. Slow - the whole file decoded - so a worker's.
pub fn listen(source: &Path) -> Result<Heard, String> {
    let options = concat_media::AudioOptions {
        rate: SAMPLE_RATE,
        channels: 1,
        format: concat_media::SampleFormat::F32,
        ..Default::default()
    };
    let unheard =
        |error: concat_media::Error| format!("could not listen to {}: {error}", source.display());
    let mut decoder = concat_media::AudioDecoder::open(source, &options).map_err(unheard)?;
    let frame = (f64::from(SAMPLE_RATE) * FRAME_SECONDS).round() as usize;
    let mut levels = Vec::new();
    let mut power = 0.0_f64;
    let mut filled = 0;
    let mut samples = 0_usize;
    while let Some(chunk) = decoder.next_f32().map_err(unheard)? {
        samples += chunk.len();
        for sample in chunk {
            power += f64::from(sample) * f64::from(sample);
            filled += 1;
            if filled == frame {
                levels.push((power / frame as f64).sqrt() as f32);
                power = 0.0;
                filled = 0;
            }
        }
    }
    let window = (SAMPLE_SECONDS / FRAME_SECONDS).round() as usize;
    Ok(Heard {
        duration: samples as f64 / f64::from(SAMPLE_RATE),
        start: densest(&levels, window) as f64 * FRAME_SECONDS,
    })
}

/// The first frame of the run of `window` frames most given to speech in
/// `levels`, each a frame's RMS. A frame is speech when it is louder than
/// [`SPEECH_FLOOR`] of the recording's loud end - the level a twentieth of
/// the frames are louder than, rather than the loudest, so one slammed
/// door does not set the bar - and the run with the most speech frames is
/// taken; between runs with as many, the one with the most energy, which
/// is the one spoken up in; between runs alike in both, the earliest. A
/// recording no longer than a window, or one with nothing in it louder
/// than [`SILENCE`], starts at its beginning.
fn densest(levels: &[f32], window: usize) -> usize {
    if window == 0 || levels.len() <= window {
        return 0;
    }
    let mut sorted = levels.to_vec();
    sorted.sort_by(f32::total_cmp);
    let loud = sorted[(sorted.len() - 1) * 19 / 20];
    if loud < SILENCE {
        return 0;
    }
    let floor = loud * SPEECH_FLOOR;
    // Running sums over the frames so far, so each run is the difference
    // of two rather than a fresh count of its window.
    let mut spoken = vec![0_usize; levels.len() + 1];
    let mut energy = vec![0.0_f64; levels.len() + 1];
    for (at, level) in levels.iter().enumerate() {
        spoken[at + 1] = spoken[at] + usize::from(*level > floor);
        energy[at + 1] = energy[at] + f64::from(*level) * f64::from(*level);
    }
    let mut best = (0, 0, 0.0);
    for first in 0..=levels.len() - window {
        let count = spoken[first + window] - spoken[first];
        let power = energy[first + window] - energy[first];
        if count > best.1 || (count == best.1 && power > best.2) {
            best = (first, count, power);
        }
    }
    best.0
}

/// Deletes a saved voice's folder.
pub fn remove(voice: &SavedVoice) -> Result<(), String> {
    match std::fs::remove_dir_all(&voice.dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not delete {}: {error}", voice.dir.display())),
    }
}

/// Copies a picture into a voice's folder as `picture.<ext>`.
fn copy_picture(path: &Path, folder: &Path) -> Result<String, String> {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| PICTURE_EXTENSIONS.contains(&extension.as_str()))
        .ok_or_else(|| format!("{} is not a picture", path.display()))?;
    let size = std::fs::metadata(path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?
        .len();
    if size > PICTURE_CAP {
        return Err(format!(
            "{} is too large for a voice's picture",
            path.display()
        ));
    }
    let file = format!("picture.{extension}");
    std::fs::copy(path, folder.join(&file))
        .map_err(|error| format!("could not copy {}: {error}", path.display()))?;
    Ok(file)
}

/// Mono samples as a 16-bit PCM WAV.
fn wav(samples: &[f32], rate: u32) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("concat-voices-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `seconds` of a tone at `level`, as a recording's samples.
    fn tone(seconds: f64, level: f32) -> Vec<f32> {
        (0..(seconds * f64::from(SAMPLE_RATE)) as usize)
            .map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin() * level)
            .collect()
    }

    /// A voice is saved from a recording - its sample cut out and written
    /// on its own, its picture beside it, its description trimmed - listed
    /// newest first, and gone with its folder when removed.
    #[test]
    fn a_voice_is_kept_listed_and_removed() {
        let dir = scratch("keep");
        // Three seconds of a tone, as the recording.
        let source = dir.join("take.wav");
        std::fs::write(&source, wav(&tone(3.0, 0.5), SAMPLE_RATE)).unwrap();
        let picture = dir.join("face.PNG");
        std::fs::write(&picture, b"png").unwrap();

        let voice = save(
            &dir.join("voices"),
            " Narrator ",
            &source,
            0.5,
            Some(&picture),
            Some("  Warm, unhurried  "),
        )
        .expect("saves");
        assert_eq!(voice.name, "Narrator");
        assert!(voice.sample().is_file());
        assert_eq!(voice.picture.as_deref(), Some("picture.png"));
        assert_eq!(voice.description.as_deref(), Some("Warm, unhurried"));
        let listed = list(&dir.join("voices"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, voice.id);
        assert!(listed[0].picture_path().unwrap().is_file());
        assert_eq!(listed[0].description, voice.description);

        assert!(
            save(&dir.join("voices"), "  ", &source, 0.0, None, None).is_err(),
            "a name is needed"
        );
        assert!(
            save(&dir.join("voices"), "Late", &source, 2.5, None, None).is_err(),
            "half a second is too little"
        );
        remove(&listed[0]).unwrap();
        assert!(list(&dir.join("voices")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A description is a line: a blank one is none, a long one is cut at
    /// its cap by characters, never inside one; and a voice saved before
    /// voices had descriptions reads as one without.
    #[test]
    fn a_voice_description_is_one_capped_line() {
        let dir = scratch("describe");
        let source = dir.join("take.wav");
        std::fs::write(&source, wav(&tone(2.0, 0.5), SAMPLE_RATE)).unwrap();
        let voices = dir.join("voices");
        let blank = save(&voices, "Blank", &source, 0.0, None, Some("   ")).unwrap();
        assert_eq!(blank.description, None);
        let long = "é".repeat(DESCRIPTION_CHARS + 20);
        let cut = save(&voices, "Long", &source, 0.0, None, Some(&long)).unwrap();
        assert_eq!(
            cut.description.as_deref().map(|line| line.chars().count()),
            Some(DESCRIPTION_CHARS)
        );
        let old: SavedVoice =
            serde_json::from_str(r#"{"id":"a","name":"Old","created":1}"#).unwrap();
        assert_eq!(old.description, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The window taken is where the speaking is: past a silent opening,
    /// the louder of two runs as full of it, and never one loud click.
    #[test]
    fn the_densest_window_is_where_the_speaking_is() {
        let run = |parts: &[(usize, f32)]| -> Vec<f32> {
            parts
                .iter()
                .flat_map(|&(frames, level)| std::iter::repeat_n(level, frames))
                .collect()
        };
        // Silence, then speech: the window starts where the speech does,
        // the earliest of the windows wholly in it.
        assert_eq!(densest(&run(&[(50, 0.0), (30, 0.5)]), 20), 50);
        // Two runs of speech as long: the one spoken up in.
        assert_eq!(
            densest(&run(&[(20, 0.2), (30, 0.0), (20, 0.5), (10, 0.0)]), 20),
            50
        );
        // A door slammed in the silence before quiet speech: the bar is
        // set by the loud end of the recording, not its loudest frame, so
        // the speech still counts and the bang does not win on energy.
        let mut levels = run(&[(60, 0.0), (40, 0.0625), (20, 0.0)]);
        levels[5] = 1.0;
        assert_eq!(densest(&levels, 20), 60);
        // No longer than a window, or silent: from the start.
        assert_eq!(densest(&run(&[(10, 0.0), (10, 0.5)]), 20), 0);
        assert_eq!(densest(&run(&[(100, 0.0)]), 20), 0);
    }

    /// A recording that opens on silence is listened to from where its
    /// voice begins, and its length is the file's.
    #[test]
    fn a_recording_is_listened_to_for_its_voice() {
        let dir = scratch("listen");
        let mut samples = vec![0.0; SAMPLE_RATE as usize * 20];
        samples.extend(tone(12.5, 0.3));
        samples.extend(vec![0.0; SAMPLE_RATE as usize * 10]);
        let source = dir.join("interview.wav");
        std::fs::write(&source, wav(&samples, SAMPLE_RATE)).unwrap();
        let heard = listen(&source).expect("listens");
        assert!((heard.duration - 42.5).abs() < 0.05, "{heard:?}");
        assert!((19.98..=20.52).contains(&heard.start), "{heard:?}");
        assert!(listen(&dir.join("missing.wav")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wav_header_says_what_follows() {
        let bytes = wav(&[0.0, 1.0, -1.0], 24_000);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 44 + 6);
        assert_eq!(i16::from_le_bytes([bytes[46], bytes[47]]), i16::MAX);
    }
}
