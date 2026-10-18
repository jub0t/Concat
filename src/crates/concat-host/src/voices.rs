// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Voices the user cloned and kept: the Audio page's Voices shelf, and
//! the rows of the text-to-speech voice picker past the built-in ones.
//!
//! A clone is a recording read as a voice by a model that reads in one
//! (Pocket TTS, Chatterbox). Kept, it is the few seconds of the recording
//! the model listens to, cut out and written as a WAV of its own - not the
//! file it came from, which may be a long video in one project - beside a
//! `voice.json` with its name and, when given one, a picture:
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
/// mono, written as a WAV of its own, and `picture` copied beside it.
/// Slow - a decode - so a worker's.
pub fn save(
    dir: &Path,
    name: &str,
    source: &Path,
    start: f64,
    picture: Option<&Path>,
) -> Result<SavedVoice, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a voice needs a name".to_owned());
    }
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
        .map_err(|error| format!("could not read the voice from {}: {error}", source.display()))?;
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
        return Err(format!("{} is too large for a voice's picture", path.display()));
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

    /// A voice is saved from a recording - its sample cut out and written
    /// on its own, its picture beside it - listed newest first, and gone
    /// with its folder when removed.
    #[test]
    fn a_voice_is_kept_listed_and_removed() {
        let dir = scratch("keep");
        // Three seconds of a tone, as the recording.
        let tone: Vec<f32> = (0..SAMPLE_RATE * 3)
            .map(|n| (n as f32 * 440.0 * std::f32::consts::TAU / SAMPLE_RATE as f32).sin() * 0.5)
            .collect();
        let source = dir.join("take.wav");
        std::fs::write(&source, wav(&tone, SAMPLE_RATE)).unwrap();
        let picture = dir.join("face.PNG");
        std::fs::write(&picture, b"png").unwrap();

        let voice = save(&dir.join("voices"), " Narrator ", &source, 0.5, Some(&picture))
            .expect("saves");
        assert_eq!(voice.name, "Narrator");
        assert!(voice.sample().is_file());
        assert_eq!(voice.picture.as_deref(), Some("picture.png"));
        let listed = list(&dir.join("voices"));
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, voice.id);
        assert!(listed[0].picture_path().unwrap().is_file());

        assert!(save(&dir.join("voices"), "  ", &source, 0.0, None).is_err(), "a name is needed");
        assert!(
            save(&dir.join("voices"), "Late", &source, 2.5, None).is_err(),
            "half a second is too little"
        );
        remove(&listed[0]).unwrap();
        assert!(list(&dir.join("voices")).is_empty());
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
