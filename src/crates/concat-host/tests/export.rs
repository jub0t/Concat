// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Exporting a project end to end, the way the window does it: media
//! imported by probing, every edit applied as a command through a session,
//! the session flattened, the engine rendering it, and the file read back
//! and checked against what the timeline says.
//!
//! One scripted editing session takes every step a person can take -
//! adding, splitting, trimming, moving, retiming, reversing, freezing,
//! transitions, effects, keys, transforms, crops, layers, titles, stills,
//! sound-only clips, detached audio, muted and hidden tracks, a second
//! timeline, undo - and exports after each one. The export must succeed
//! and the file must hold the right number of frames, the right length of
//! sound, and, where the step lets us say, the right picture and sound at
//! a given instant.
//!
//! Every check is a panic on purpose. The shipping build aborts on a
//! panic in its render thread, so a crash here is a crash a person would
//! have had with their project open, and a crash in a test is the cheaper
//! of the two by a wide margin.
//!
//! The sources are made here rather than checked in: a silent picture
//! whose every frame is a solid colour naming its source second, and a
//! sound that is a tone during the odd seconds and silence during the
//! even. Read back, a frame's colour and a stretch's loudness say which
//! second of which source they came from, which is how a trim, a speed
//! change or a reverse is checked and not just survived.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use concat_core::animate::Track as GainTrack;
use concat_core::frame::Frame;
use concat_core::time::FrameRate;
use concat_host::dirs::AppDirs;
use concat_host::export::{self, ExportSpec};
use concat_host::session::Session;
use concat_host::titles::Titles;
use concat_host::{media, projects};
use concat_media::audio::{self as sound, AudioClip};
use concat_media::{
    AudioDecoder, AudioOptions, DecodeOptions, Decoder, EncodeOptions, Encoder,
    Error as MediaError, FrameSink, FrameSource, HwDevice, SampleFormat, VideoCodec,
};
use concat_project::animation;
use concat_project::commands::{ClipMove, ClipPatch, Command, TrackFlag, TrimEdge};
use concat_project::model::{
    AnimationSlot, AppliedFilter, ClipAnimation, Crop, KeyEase, KeyProperty, SpeedPoint, TextStyle,
    Transition, VideoSettings,
};

/// The sample rate every source and every export carries.
const RATE: u32 = 48_000;
/// The picture every source is: small, so a render is quick, and even on
/// both sides for the encoder's chroma.
const WIDTH: u32 = 160;
const HEIGHT: u32 = 90;

// ── the sources ──

/// The colour a source's frame has during source second `second`: a red
/// step of forty a second over a fixed green and blue, so a frame read
/// back names its second and a colour that is not one of these is not
/// footage.
fn colour_of_second(second: u32) -> [u8; 4] {
    [30 + 40 * second as u8, 60, 200, 255]
}

/// The source second a read-back colour names, or None for anything that
/// is not footage: black in a gap, a still, a title.
fn second_of_colour(rgb: [u8; 3]) -> Option<u32> {
    let close = |a: u8, b: u8| a.abs_diff(b) <= 14;
    if !close(rgb[1], 60) || !close(rgb[2], 200) {
        return None;
    }
    let second = (f64::from(rgb[0]) - 30.0) / 40.0;
    let nearest = second.round();
    if (second - nearest).abs() > 0.35 || nearest < 0.0 {
        return None;
    }
    Some(nearest as u32)
}

/// A silent picture of `seconds` at `rate`, each frame the colour of its
/// source second.
fn picture(path: &Path, rate: FrameRate, seconds: u32) {
    let options = EncodeOptions {
        codec: VideoCodec::H264,
        preset: "ultrafast".to_owned(),
        crf: 16,
        ten_bit: false,
        hardware: false,
    };
    let mut encoder =
        Encoder::create(path, WIDTH, HEIGHT, rate, &options).expect("the linked FFmpeg encodes");
    let fps = rate.fps().as_f64();
    let frames = (f64::from(seconds) * fps).round() as u32;
    let mut frame = Frame::black(WIDTH, HEIGHT);
    for index in 0..frames {
        let second = (f64::from(index) / fps).floor() as u32;
        frame.fill(colour_of_second(second));
        encoder.write_frame(&frame).expect("writes a frame");
    }
    encoder.finish().expect("finishes the picture");
}

/// The clock: a 1 kHz tone during the odd seconds, silence during the
/// even, as a stereo 16-bit WAV of `seconds`.
fn clock_wav(path: &Path, seconds: u32) {
    let count = seconds * RATE;
    let mut data = Vec::with_capacity(count as usize * 4);
    for index in 0..count {
        let t = f64::from(index) / f64::from(RATE);
        let on = (t.floor() as u32) % 2 == 1;
        let value = if on {
            (0.5 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin() * 32767.0) as i16
        } else {
            0
        };
        data.extend_from_slice(&value.to_le_bytes());
        data.extend_from_slice(&value.to_le_bytes());
    }
    let mut bytes = Vec::with_capacity(44 + data.len());
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 4).to_le_bytes());
    bytes.extend_from_slice(&4u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&data);
    std::fs::write(path, bytes).expect("writes the wav");
}

/// The clock as the AAC the engine's own mix writes.
fn clock_aac(wav: &Path, path: &Path, seconds: u32) {
    let clip = AudioClip {
        path: wav.to_path_buf(),
        stream: None,
        start: 0.0,
        duration: f64::from(seconds),
        source_start: 0.0,
        speed: 1.0,
        preserve_pitch: true,
        volume: 1.0,
        volume_curve: GainTrack::default(),
        fade_in: 0.0,
        fade_out: 0.0,
        filter_chain: String::new(),
    };
    sound::mix_to_file(&[clip], f64::from(seconds), path).expect("mixes the clock");
}

/// A still: one solid colour no source second has.
const STILL: [u8; 4] = [250, 250, 30, 255];

fn still(path: &Path) {
    let mut frame = Frame::black(WIDTH, HEIGHT);
    frame.fill(STILL);
    let bytes = concat_media::jpeg(&frame, 2).expect("encodes the still");
    std::fs::write(path, bytes).expect("writes the still");
}

/// Every source file the script imports.
struct Sources {
    /// What a screen recorder makes: 60 fps with sound. Six seconds.
    peek: PathBuf,
    /// A camera: 30 fps with sound. Four seconds.
    cam: PathBuf,
    /// A silent render at 24 fps. Three seconds.
    silent: PathBuf,
    /// A photograph.
    still: PathBuf,
    /// Sound alone, as AAC and as PCM. Six seconds.
    aac: PathBuf,
    wav: PathBuf,
}

impl Sources {
    fn make(dir: &Path) -> Sources {
        let wav = dir.join("clock.wav");
        clock_wav(&wav, 6);
        let aac = dir.join("clock.m4a");
        clock_aac(&wav, &aac, 6);

        let peek_picture = dir.join("peek-picture.mp4");
        picture(&peek_picture, FrameRate::SIXTY, 6);
        let peek = dir.join("peek.mp4");
        sound::mux(&peek_picture, &aac, &peek).expect("joins the screen recording");

        let cam_picture = dir.join("cam-picture.mp4");
        picture(&cam_picture, FrameRate::THIRTY, 4);
        let cam_sound = dir.join("cam.m4a");
        clock_aac(&wav, &cam_sound, 4);
        let cam = dir.join("cam.mp4");
        sound::mux(&cam_picture, &cam_sound, &cam).expect("joins the camera clip");

        let silent = dir.join("silent.mp4");
        picture(&silent, FrameRate::FILM, 3);

        let still_path = dir.join("still.jpg");
        still(&still_path);

        Sources {
            peek,
            cam,
            silent,
            still: still_path,
            aac,
            wav,
        }
    }
}

// ── the export, read back ──

/// An exported file, decoded whole.
struct Exported {
    label: String,
    fps: f64,
    frames: Vec<Frame>,
    /// Interleaved stereo at `RATE`, or None for a file with no sound.
    audio: Option<Vec<f32>>,
}

impl Exported {
    fn read(label: &str, path: &Path, rate: (i64, i64)) -> Exported {
        let mut decoder = Decoder::open(path, &DecodeOptions::default())
            .unwrap_or_else(|error| panic!("{label}: the export does not open: {error}"));
        let mut frames = Vec::new();
        while let Some(frame) = decoder
            .next_frame()
            .unwrap_or_else(|error| panic!("{label}: the export does not decode: {error}"))
        {
            frames.push(frame);
        }
        let options = AudioOptions {
            rate: RATE,
            channels: 2,
            format: SampleFormat::F32,
            ..AudioOptions::default()
        };
        let audio = match AudioDecoder::open(path, &options) {
            Ok(mut decoder) => Some(
                decoder
                    .collect_f32()
                    .unwrap_or_else(|error| panic!("{label}: the sound does not decode: {error}")),
            ),
            Err(MediaError::NoAudioStream { .. }) => None,
            Err(error) => panic!("{label}: the sound does not open: {error}"),
        };
        Exported {
            label: label.to_owned(),
            fps: rate.0 as f64 / rate.1 as f64,
            frames,
            audio,
        }
    }

    /// The file holds exactly the frames `seconds` of timeline is.
    fn expect_length(&self, seconds: f64) {
        let expected = (seconds * self.fps).round() as usize;
        assert_eq!(
            self.frames.len(),
            expected,
            "{}: {seconds}s at {} fps is {expected} frames",
            self.label,
            self.fps
        );
    }

    /// The file's sound runs the whole `seconds`, within an AAC frame.
    fn expect_sound(&self, seconds: f64) {
        let audio = self
            .audio
            .as_ref()
            .unwrap_or_else(|| panic!("{}: the export has no sound", self.label));
        let got = audio.len() as f64 / 2.0 / f64::from(RATE);
        assert!(
            (got - seconds).abs() < 0.05,
            "{}: the sound runs {got:.3}s, not {seconds}s",
            self.label
        );
    }

    fn expect_no_sound(&self) {
        assert!(
            self.audio.is_none(),
            "{}: the export has a sound stream it should not",
            self.label
        );
    }

    /// The colour at the middle of the frame shown at `at` seconds.
    fn colour_at(&self, at: f64) -> [u8; 3] {
        let index = ((at * self.fps).floor() as usize).min(self.frames.len() - 1);
        let frame = &self.frames[index];
        let [r, g, b, _] = frame
            .pixel(frame.width() / 2, frame.height() / 2)
            .expect("the middle is inside the frame");
        [r, g, b]
    }

    /// The frame at `at` shows source second `second`.
    fn expect_second(&self, at: f64, second: u32) {
        let rgb = self.colour_at(at);
        assert_eq!(
            second_of_colour(rgb),
            Some(second),
            "{}: at {at}s the picture is {rgb:?}, not source second {second}",
            self.label
        );
    }

    /// The frame at `at` is black: a gap, or a hidden track.
    fn expect_black(&self, at: f64) {
        let rgb = self.colour_at(at);
        assert!(
            rgb.iter().all(|channel| *channel < 24),
            "{}: at {at}s the picture is {rgb:?}, not black",
            self.label
        );
    }

    /// The frame at `at` is the still.
    fn expect_still(&self, at: f64) {
        let rgb = self.colour_at(at);
        let close = rgb
            .iter()
            .zip(STILL.iter())
            .all(|(got, want)| got.abs_diff(*want) <= 20);
        assert!(
            close,
            "{}: at {at}s the picture is {rgb:?}, not the still",
            self.label
        );
    }

    /// Whether the sound is loud over the tenth of a second from `at`.
    fn loud_at(&self, at: f64) -> bool {
        let audio = self
            .audio
            .as_ref()
            .unwrap_or_else(|| panic!("{}: the export has no sound", self.label));
        let from = ((at * f64::from(RATE)) as usize * 2).min(audio.len());
        let to = (from + RATE as usize / 10 * 2).min(audio.len());
        let window = &audio[from..to];
        if window.is_empty() {
            return false;
        }
        let energy: f64 = window.iter().map(|s| f64::from(*s) * f64::from(*s)).sum();
        (energy / window.len() as f64).sqrt() > 0.12
    }

    /// The sound at `at` is the tone: the source's odd second.
    fn expect_tone(&self, at: f64) {
        assert!(
            self.loud_at(at),
            "{}: at {at}s the sound is quiet where the tone should be",
            self.label
        );
    }

    /// The sound at `at` is silence: the source's even second, or nothing.
    fn expect_quiet(&self, at: f64) {
        assert!(
            !self.loud_at(at),
            "{}: at {at}s the sound is loud where it should be quiet",
            self.label
        );
    }
}

// ── the editing session ──

/// A project open the way the window opens one, with the export the
/// window runs.
struct Studio {
    session: Session,
    titles: Titles,
    exports: PathBuf,
    count: usize,
}

impl Studio {
    fn new(root: &Path, name: &str, video: VideoSettings) -> Studio {
        let info = projects::create(
            root.to_str().expect("utf-8"),
            name,
            video.width,
            video.height,
            video.rate_num,
            video.rate_den,
        )
        .expect("creates the project");
        let session = Session::open_info(&info).expect("opens the project");
        let exports = root.join(format!("{name}-exports"));
        std::fs::create_dir_all(&exports).expect("makes the export folder");
        Studio {
            session,
            titles: Titles::new(&AppDirs::under(&root.join("app"))),
            exports,
            count: 0,
        }
    }

    /// Imports a file as the window does: probed, then added.
    fn import(&mut self, path: &Path) -> String {
        let summary = media::probe(path.to_str().expect("utf-8"))
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        self.apply(Command::AddMedia {
            item: summary.to_new_media(),
        })
        .expect("an imported file has an id")
    }

    /// Applies one edit. An edit the model refuses is a failure here: the
    /// script only asks for what the window lets a person ask for.
    fn apply(&mut self, command: Command) -> Option<String> {
        let what = format!("{command:?}");
        self.session
            .apply(command)
            .unwrap_or_else(|error| panic!("{what}: {error}"))
            .created_id
    }

    fn project(&self) -> &concat_project::model::Project {
        self.session.project()
    }

    fn clip(&self, id: &str) -> &concat_project::model::Clip {
        self.project()
            .active()
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .unwrap_or_else(|| panic!("clip {id} is on the timeline"))
    }

    fn clip_ids(&self) -> Vec<String> {
        self.project()
            .active()
            .clips
            .iter()
            .map(|clip| clip.id.clone())
            .collect()
    }

    /// The clips added by `edit`, in timeline order.
    fn clips_made_by(&mut self, edit: Command) -> Vec<String> {
        let before = self.clip_ids();
        self.apply(edit);
        self.clip_ids()
            .into_iter()
            .filter(|id| !before.contains(id))
            .collect()
    }

    /// Where the active timeline ends: the last clip's end.
    fn end(&self) -> f64 {
        self.project()
            .active()
            .clips
            .iter()
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max)
    }

    /// Exports at the timeline's own rate.
    fn export(&mut self, label: &str) -> Exported {
        self.export_at(label, None)
    }

    /// Exports the way the window does: the session flattened, titles
    /// rasterised and rejoined, the engine rendering to a file - then the
    /// file read back whole. The project is saved and reopened too, and
    /// the reopened one must flatten to the same clips: what was exported
    /// is what will be exported tomorrow.
    fn export_at(&mut self, label: &str, rate: Option<(i64, i64)>) -> Exported {
        self.count += 1;
        let label = format!("{:02} {label}", self.count).replace('/', "-");
        let settings = self.session.settings();
        let spec = ExportSpec {
            output: self
                .exports
                .join(format!("{}.mp4", label.replace(' ', "-")))
                .to_string_lossy()
                .into_owned(),
            crf: 18,
            preset: "ultrafast".to_owned(),
            codec: VideoCodec::H264,
            ten_bit: false,
        };
        let titles = self
            .titles
            .clips(self.session.project(), settings.width, settings.height)
            .into_iter()
            .map(|title| title.clip)
            .collect();
        let mut request = export::request(&self.session, &spec, titles);
        if let Some((num, den)) = rate {
            request.rate_num = num;
            request.rate_den = den;
        }
        let cancel = AtomicBool::new(false);
        let written = export::run(&request, &cancel, |_| {})
            .unwrap_or_else(|error| panic!("{label}: the export failed: {error}"));

        self.session.save(None).expect("saves the project");
        let reopened = Session::open(self.session.path(), self.session.settings())
            .unwrap_or_else(|error| panic!("{label}: the saved project does not reopen: {error}"));
        assert!(
            reopened.flattened_clips() == self.session.flattened_clips(),
            "{label}: the project flattens differently after a save and reopen"
        );

        Exported::read(
            &label,
            Path::new(&written),
            (request.rate_num, request.rate_den),
        )
    }
}

/// A fresh scratch folder for one test, gone when the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("concat-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("makes the scratch folder");
        Scratch(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn effect(id: &str) -> AppliedFilter {
    AppliedFilter {
        id: id.to_owned(),
        params: Default::default(),
        enabled: true,
        keys: Default::default(),
    }
}

fn video(width: u32, height: u32, rate_num: i64, rate_den: i64) -> VideoSettings {
    VideoSettings {
        width,
        height,
        rate_num,
        rate_den,
    }
}

// ── the tests ──

/// Issue #112: a 60 fps export froze at three-quarters of its length and
/// held one frame under the rest of the sound. Every rate a person can
/// pick renders every frame, with the picture and the sound both reaching
/// the end of the clip.
#[test]
fn one_clip_exports_whole_at_every_rate() {
    let scratch = Scratch::new("rates");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Rates", video(WIDTH, HEIGHT, 30, 1));
    let peek = studio.import(&sources.peek);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: peek,
        start: 0.0,
    });

    for (num, den) in [
        (30, 1),
        (60, 1),
        (24, 1),
        (25, 1),
        (30000, 1001),
        (60000, 1001),
        (24000, 1001),
    ] {
        let exported = studio.export_at(&format!("peek at {num}/{den}"), Some((num, den)));
        exported.expect_length(6.0);
        exported.expect_sound(6.0);
        // The last second is the last second of the source, not a frame
        // from earlier held to the end.
        exported.expect_second(0.5, 0);
        exported.expect_second(3.5, 3);
        exported.expect_second(5.9, 5);
        exported.expect_tone(5.5);
        exported.expect_quiet(4.5);
    }
}

/// Issue #108: two clips on one track, one of them a 60 fps screen
/// recording with sound, export - and so does everything a person does to
/// them afterwards.
#[test]
fn every_edit_still_exports() {
    let scratch = Scratch::new("edits");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Edits", video(WIDTH, HEIGHT, 30, 1));
    let peek = studio.import(&sources.peek);
    let cam = studio.import(&sources.cam);
    let silent = studio.import(&sources.silent);
    let still_media = studio.import(&sources.still);
    let aac = studio.import(&sources.aac);
    let wav = studio.import(&sources.wav);

    // Two clips on one track: the screen recording, then the camera with a
    // gap before it, so the edits below have room.
    let peek_clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: peek.clone(),
            start: 0.0,
        })
        .expect("the clip has an id");
    let cam_clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: cam,
            start: 12.0,
        })
        .expect("the clip has an id");
    let exported = studio.export("two clips");
    exported.expect_length(16.0);
    exported.expect_sound(16.0);
    exported.expect_second(1.5, 1);
    exported.expect_second(5.5, 5);
    exported.expect_black(8.0);
    exported.expect_second(13.5, 1);
    exported.expect_tone(13.5);
    exported.expect_quiet(8.0);

    // Split, trim the tail's head off, slide it back to close the gap,
    // then drop the head.
    let mut pieces = studio.clips_made_by(Command::SplitClips {
        clip_ids: vec![peek_clip.clone()],
        time: 2.0,
    });
    assert_eq!(pieces.len(), 1, "a split makes one new clip");
    let tail = pieces.remove(0);
    let exported = studio.export("split");
    exported.expect_length(16.0);
    exported.expect_second(1.5, 1);
    exported.expect_second(2.5, 2);

    studio.apply(Command::TrimClip {
        clip_id: tail.clone(),
        edge: TrimEdge::Start,
        delta: 0.5,
    });
    let exported = studio.export("trimmed");
    exported.expect_black(2.25);
    exported.expect_second(2.75, 2);
    exported.expect_second(5.75, 5);

    let track = studio.clip(&tail).track_id.clone();
    studio.apply(Command::MoveClips {
        moves: vec![ClipMove {
            clip_id: tail.clone(),
            start: 2.0,
            track_id: track.clone(),
        }],
    });
    let exported = studio.export("moved");
    exported.expect_second(2.25, 2);
    exported.expect_second(5.25, 5);
    exported.expect_black(5.75);
    exported.expect_tone(2.75);

    studio.apply(Command::RemoveClips {
        clip_ids: vec![peek_clip.clone()],
    });
    let exported = studio.export("head removed");
    exported.expect_length(16.0);
    exported.expect_black(1.0);
    exported.expect_second(2.25, 2);

    // Retimed: twice as fast, then half speed with the pitch riding, then
    // backwards, then on a curve, then undone back to half speed.
    studio.apply(Command::SetClipSpeed {
        clip_id: tail.clone(),
        speed: 2.0,
    });
    let exported = studio.export("double speed");
    assert!(
        (studio.clip(&tail).duration - 1.75).abs() < 1e-6,
        "3.5s of source at 2x is 1.75s"
    );
    exported.expect_second(2.3, 3);
    exported.expect_second(3.5, 5);
    exported.expect_tone(2.25);

    studio.apply(Command::UpdateClip {
        clip_id: tail.clone(),
        patch: ClipPatch {
            preserve_pitch: Some(false),
            ..ClipPatch::default()
        },
    });
    studio.apply(Command::SetClipSpeed {
        clip_id: tail.clone(),
        speed: 0.5,
    });
    let exported = studio.export("half speed");
    exported.expect_second(2.25, 2);
    exported.expect_second(8.5, 5);
    exported.expect_sound(16.0);

    studio.apply(Command::UpdateClip {
        clip_id: tail.clone(),
        patch: ClipPatch {
            reverse: Some(true),
            ..ClipPatch::default()
        },
    });
    let exported = studio.export("reversed");
    exported.expect_second(2.25, 5);
    exported.expect_second(8.5, 2);

    studio.apply(Command::SetClipSpeedCurve {
        clip_id: tail.clone(),
        curve: Some(vec![
            SpeedPoint {
                at: 0.0,
                speed: 0.5,
            },
            SpeedPoint {
                at: 1.0,
                speed: 2.0,
            },
        ]),
    });
    let exported = studio.export("speed curve");
    exported.expect_length(16.0);
    exported.expect_sound(16.0);

    studio.session.undo();
    studio.session.undo();
    let exported = studio.export("undone to half speed");
    exported.expect_second(2.25, 2);
    exported.expect_second(8.5, 5);

    // A freeze frame: the still the window rasterises, cut in at 4s.
    let frozen = concat_media::jpeg(
        &{
            let mut frame = Frame::black(WIDTH, HEIGHT);
            frame.fill(STILL);
            frame
        },
        4,
    )
    .expect("encodes the freeze");
    media::write_artwork(studio.session.path(), "freeze-test.jpg", &frozen)
        .expect("writes the freeze");
    let frozen_path = format!("{}/cache/freeze-test.jpg", studio.session.path());
    let frozen_media = media::probe(&frozen_path)
        .expect("probes the freeze")
        .to_new_media();
    let before = studio.end();
    let made = studio.clips_made_by(Command::FreezeFrame {
        clip_id: tail.clone(),
        time: 4.0,
        duration: Some(1.0),
        still: Some(frozen_media),
    });
    assert!(!made.is_empty(), "a freeze adds the still");
    let exported = studio.export("freeze frame");
    exported.expect_still(4.5);
    exported.expect_second(3.5, 3);
    assert!(
        studio.end() >= before,
        "a freeze never shortens the timeline"
    );
    // Undo it, so the piece below keeps its handle for the transitions.
    studio.session.undo();

    // Transitions into the camera clip, which is slid up against the piece
    // before it and given a handle to dissolve from, and every shape the
    // menu offers - including one the engine has never heard of, which is
    // a cut and not a crash.
    let piece_end = {
        let clip = studio.clip(&tail);
        clip.start + clip.duration
    };
    studio.apply(Command::MoveClips {
        moves: vec![ClipMove {
            clip_id: cam_clip.clone(),
            start: piece_end,
            track_id: track.clone(),
        }],
    });
    studio.apply(Command::TrimClip {
        clip_id: cam_clip.clone(),
        edge: TrimEdge::Start,
        delta: 1.0,
    });
    for kind in [
        "cross-fade",
        "push",
        "zoom",
        "wipe-left",
        "wipe-right",
        "fade-black",
        "fade-white",
        "something-new",
    ] {
        studio.apply(Command::UpdateClip {
            clip_id: cam_clip.clone(),
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    id: kind.to_owned(),
                    duration: 0.5,
                })),
                ..ClipPatch::default()
            },
        });
        let end = studio.end();
        let exported = studio.export(&format!("transition {kind}"));
        exported.expect_length(end);
        exported.expect_sound(end);
    }
    studio.apply(Command::UpdateClip {
        clip_id: cam_clip.clone(),
        patch: ClipPatch {
            transition_in: Some(None),
            ..ClipPatch::default()
        },
    });

    // Effects on the picture and the sound, with and without keys.
    studio.apply(Command::UpdateClip {
        clip_id: cam_clip.clone(),
        patch: ClipPatch {
            video_effects: Some(vec![effect("concat.vhs"), effect("concat.hologram")]),
            filters: Some(vec![effect("concat.bass")]),
            ..ClipPatch::default()
        },
    });
    let end = studio.end();
    let exported = studio.export("effects");
    exported.expect_length(end);
    exported.expect_sound(end);

    for (at, value) in [(0.0, 0.0), (1.0, 100.0)] {
        studio.apply(Command::SetEffectKey {
            clip_id: cam_clip.clone(),
            entry: 0,
            key: "bleed".to_owned(),
            at,
            value,
            ease: KeyEase::default(),
        });
    }
    let exported = studio.export("effect keys");
    exported.expect_length(end);
    studio.apply(Command::ClearEffectKeys {
        clip_id: cam_clip.clone(),
        entry: 0,
        key: "bleed".to_owned(),
    });

    for (property, from, to) in [
        (KeyProperty::Opacity, 1.0, 0.2),
        (KeyProperty::Scale, 0.5, 1.0),
        (KeyProperty::Rotation, 0.0, 90.0),
        (KeyProperty::OffsetX, -0.2, 0.2),
        (KeyProperty::Volume, 1.0, 0.0),
    ] {
        for (at, value) in [(0.0, from), (1.0, to)] {
            studio.apply(Command::SetClipKey {
                clip_id: cam_clip.clone(),
                property,
                at,
                value,
                ease: KeyEase::default(),
            });
        }
    }
    let exported = studio.export("clip keys");
    exported.expect_length(end);
    exported.expect_sound(end);
    for property in [
        KeyProperty::Opacity,
        KeyProperty::Scale,
        KeyProperty::Rotation,
        KeyProperty::OffsetX,
        KeyProperty::Volume,
    ] {
        studio.apply(Command::ClearClipKeys {
            clip_id: cam_clip.clone(),
            property,
        });
    }

    // The transform, the crop, the flips, a blend, opacity, fades, gain,
    // and an animation on each slot.
    studio.apply(Command::SetClipTransform {
        clip_id: cam_clip.clone(),
        scale: Some(0.6),
        offset_x: Some(0.1),
        offset_y: Some(-0.1),
        rotation: Some(30.0),
        stretch_x: Some(1.2),
        stretch_y: Some(0.9),
    });
    studio.apply(Command::UpdateClip {
        clip_id: cam_clip.clone(),
        patch: ClipPatch {
            crop: Some(Some(Crop {
                left: 0.1,
                top: 0.1,
                right: 0.1,
                bottom: 0.1,
            })),
            flip_h: Some(true),
            flip_v: Some(true),
            blend: Some("screen".to_owned()),
            opacity: Some(0.7),
            fade_in: Some(0.5),
            fade_out: Some(0.5),
            volume: Some(0.5),
            ..ClipPatch::default()
        },
    });
    let exported = studio.export("transform crop blend fades");
    exported.expect_length(end);
    exported.expect_sound(end);

    for slot in [AnimationSlot::In, AnimationSlot::Out, AnimationSlot::Combo] {
        let names = animation::names(slot);
        assert!(!names.is_empty(), "{slot:?} offers animations");
        studio.apply(Command::SetClipAnimation {
            clip_id: cam_clip.clone(),
            slot,
            animation: Some(ClipAnimation {
                preset: names[0].to_owned(),
                duration: 0.5,
            }),
        });
    }
    let exported = studio.export("animations");
    exported.expect_length(end);

    // A layer over everything, and a title.
    studio.apply(Command::AddLayerClip {
        track_id: None,
        start: 2.0,
        duration: Some(3.0),
        effect_id: "concat.glow".to_owned(),
        name: "Glow".to_owned(),
    });
    let exported = studio.export("layer");
    exported.expect_length(end);
    studio.apply(Command::AddTextClip {
        track_id: None,
        start: 3.0,
        style: Some(TextStyle {
            content: "Hello".to_owned(),
            ..TextStyle::default()
        }),
        duration: Some(2.0),
        offset_y: None,
    });
    let exported = studio.export("title");
    exported.expect_length(end);
    exported.expect_sound(end);

    // A photograph, a silent clip, sound-only clips on their own track:
    // the AAC the mix writes and a plain PCM file.
    studio.apply(Command::AddClipAtFirstFree {
        media_id: still_media,
        start: 17.0,
    });
    studio.apply(Command::AddClipAtFirstFree {
        media_id: silent,
        start: 22.0,
    });
    let end = studio.end();
    let exported = studio.export("still and silent clip");
    exported.expect_length(end);
    exported.expect_still(19.0);
    exported.expect_second(23.5, 1);
    exported.expect_quiet(19.0);

    let sound_track = studio
        .apply(Command::AddTrack)
        .expect("a new track has an id");
    studio.apply(Command::AddClip {
        media_id: aac,
        track_id: sound_track.clone(),
        start: 17.0,
        ripple: false,
    });
    studio.apply(Command::AddClip {
        media_id: wav,
        track_id: sound_track.clone(),
        start: 23.0,
        ripple: false,
    });
    let end = studio.end();
    let exported = studio.export("sound only clips");
    exported.expect_length(end);
    exported.expect_sound(end);
    exported.expect_tone(18.5);
    exported.expect_quiet(19.5);
    exported.expect_tone(24.5);

    // The camera's sound detached to its own clip, then put back.
    studio.apply(Command::DetachAudio {
        clip_id: cam_clip.clone(),
    });
    let exported = studio.export("detached audio");
    exported.expect_length(end);
    exported.expect_sound(end);
    studio.apply(Command::ReattachAudio {
        clip_id: cam_clip.clone(),
    });
    let exported = studio.export("reattached audio");
    exported.expect_sound(end);

    // A muted track, then a hidden one, then both back.
    studio.apply(Command::SetTrackFlag {
        track_id: sound_track.clone(),
        flag: TrackFlag::Muted,
        value: true,
    });
    let exported = studio.export("muted track");
    exported.expect_quiet(18.5);
    studio.apply(Command::SetTrackFlag {
        track_id: track.clone(),
        flag: TrackFlag::Visible,
        value: false,
    });
    let exported = studio.export("hidden track");
    exported.expect_length(end);
    exported.expect_black(2.25);
    exported.expect_black(19.0);
    studio.apply(Command::SetTrackFlag {
        track_id: track.clone(),
        flag: TrackFlag::Visible,
        value: true,
    });
    studio.apply(Command::SetTrackFlag {
        track_id: sound_track,
        flag: TrackFlag::Muted,
        value: false,
    });

    // The same timeline at other rates than its own. The picture is read
    // past the glow layer's span, where the footage shows as itself.
    for (num, den) in [(60, 1), (30000, 1001), (24, 1)] {
        let exported = studio.export_at(&format!("whole edit at {num}/{den}"), Some((num, den)));
        exported.expect_length(end);
        exported.expect_sound(end);
        exported.expect_second(6.0, 4);
        exported.expect_still(19.0);
    }

    // A second timeline of its own frame and rate, exported active.
    let second = studio
        .apply(Command::AddTimeline)
        .expect("a new timeline has an id");
    studio.apply(Command::SetTimelineVideo {
        timeline_id: second.clone(),
        video: video(96, 54, 60, 1),
    });
    studio.apply(Command::SelectTimeline {
        timeline_id: second,
    });
    studio.apply(Command::AddClipAtFirstFree {
        media_id: peek,
        start: 0.0,
    });
    let exported = studio.export("second timeline at 60");
    exported.expect_length(6.0);
    exported.expect_sound(6.0);
    exported.expect_second(5.9, 5);
    assert_eq!(
        (exported.frames[0].width(), exported.frames[0].height()),
        (96, 54),
        "the second timeline's own frame"
    );
}

/// The edges: a timeline of nothing but a photograph, of nothing but
/// sound, a clip one frame long, a clip trimmed to the very end of its
/// media, and a clip that asks for more than its media has.
#[test]
fn the_edges_export_too() {
    let scratch = Scratch::new("edges");
    let sources = Sources::make(scratch.path());

    let mut studio = Studio::new(scratch.path(), "Still", video(WIDTH, HEIGHT, 30, 1));
    let still_media = studio.import(&sources.still);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: still_media,
        start: 0.0,
    });
    let end = studio.end();
    let exported = studio.export("only a still");
    exported.expect_length(end);
    exported.expect_no_sound();
    exported.expect_still(0.5);
    exported.expect_still(end - 0.1);

    let mut studio = Studio::new(scratch.path(), "Sound", video(WIDTH, HEIGHT, 30, 1));
    let wav = studio.import(&sources.wav);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: wav,
        start: 0.5,
    });
    let end = studio.end();
    let exported = studio.export("only sound");
    exported.expect_length(end);
    exported.expect_sound(end);
    exported.expect_black(1.0);
    exported.expect_quiet(0.2);
    exported.expect_tone(2.0);

    let mut studio = Studio::new(scratch.path(), "Short", video(WIDTH, HEIGHT, 30, 1));
    let cam = studio.import(&sources.cam);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: cam.clone(),
            start: 0.0,
        })
        .expect("the clip has an id");
    // One frame long, at the very end of the media - and, since a trim
    // of the head keeps the tail where it was, at the end of the timeline.
    studio.apply(Command::TrimClip {
        clip_id: clip.clone(),
        edge: TrimEdge::Start,
        delta: 4.0 - 1.0 / 30.0,
    });
    let length = studio.clip(&clip).duration;
    assert!(
        length > 0.0 && length <= 1.0 / 30.0 + 1e-6,
        "one frame: {length}"
    );
    let end = studio.end();
    let exported = studio.export("one frame at the end");
    exported.expect_length(end);
    exported.expect_black(1.0);
    exported.expect_second(end - 0.01, 3);

    // Slow motion over the media's last second, stretched to eight: the
    // decoders reach the end of the file long before the clip ends.
    let mut studio = Studio::new(scratch.path(), "Slow", video(WIDTH, HEIGHT, 30, 1));
    let cam = studio.import(&sources.cam);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: cam,
            start: 0.0,
        })
        .expect("the clip has an id");
    studio.apply(Command::TrimClip {
        clip_id: clip.clone(),
        edge: TrimEdge::Start,
        delta: 3.0,
    });
    studio.apply(Command::SetClipSpeed {
        clip_id: clip.clone(),
        speed: 0.125,
    });
    let start = studio.clip(&clip).start;
    let end = studio.end();
    assert!(
        (end - start - 8.0).abs() < 1e-6,
        "one second at an eighth is eight"
    );
    let exported = studio.export("slow motion at the end");
    exported.expect_length(end);
    exported.expect_sound(end);
    exported.expect_black(start - 0.5);
    exported.expect_second(start + 0.5, 3);
    exported.expect_second(end - 0.5, 3);
    exported.expect_tone(start + 4.0);
}

/// Hardware decode preferred, the way the settings toggle leaves it: the
/// export decodes its sources on the platform's device where there is one
/// and reads the same picture and sound back. The preference is the
/// process's, so the other scenarios running alongside share it for the
/// moment, and must not mind.
#[test]
fn an_export_decodes_on_the_hardware_when_preferred() {
    struct Preferred;
    impl Drop for Preferred {
        fn drop(&mut self) {
            concat_media::set_hardware_decode(false);
        }
    }
    let _preferred = Preferred;
    concat_media::set_hardware_decode(true);
    assert_eq!(
        concat_media::hardware_decode(),
        HwDevice::platform_default().is_some(),
        "on where the platform has a device, off where it has none"
    );

    let scratch = Scratch::new("hardware");
    let sources = Sources::make(scratch.path());
    // A reader opened the way the engine opens them follows the preference
    // onto the device, on a machine that has one.
    let mut reader = Decoder::open(&sources.peek, &DecodeOptions::default()).expect("opens");
    reader.next_frame().expect("decodes").expect("a frame");
    assert_eq!(reader.hardware(), HwDevice::platform_default());
    drop(reader);
    let mut studio = Studio::new(scratch.path(), "Hardware", video(WIDTH, HEIGHT, 30, 1));
    let peek = studio.import(&sources.peek);
    let cam = studio.import(&sources.cam);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: peek,
        start: 0.0,
    });
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: cam,
            start: 6.0,
        })
        .expect("the clip has an id");
    // A retimed clip on the device: the pacing sits on top of it exactly
    // as it does the CPU.
    studio.apply(Command::SetClipSpeed {
        clip_id: clip.clone(),
        speed: 2.0,
    });
    let start = studio.clip(&clip).start;
    let end = studio.end();
    assert!(
        (end - start - 2.0).abs() < 1e-6,
        "four seconds at double is two: {start} to {end}"
    );

    for (num, den) in [(30, 1), (60000, 1001)] {
        let exported = studio.export_at(&format!("hardware at {num}/{den}"), Some((num, den)));
        exported.expect_length(end);
        exported.expect_sound(end);
        exported.expect_second(0.5, 0);
        exported.expect_second(5.9, 5);
        exported.expect_second(start + 0.25, 0);
        exported.expect_second(start + 1.75, 3);
        exported.expect_tone(5.5);
        exported.expect_quiet(4.5);
    }
}
