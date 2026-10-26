// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

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
//! second of which source they came from, which is how a trim or a speed
//! change is checked and not just survived.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use concat_core::animate::Track as GainTrack;
use concat_core::frame::Frame;
use concat_core::time::FrameRate;
use concat_host::dirs::AppDirs;
use concat_host::export::{self, ExportSpec};
use concat_host::session::Session;
use concat_host::titles::Titles;
use concat_host::{media, projects, reverse};
use concat_media::audio::{self as sound, AudioClip};
use concat_media::{
    AudioDecoder, AudioOptions, DecodeOptions, Decoder, EncodeOptions, Encoder,
    Error as MediaError, FrameSink, FrameSource, HwDevice, RateMode, SampleFormat, VideoCodec,
};
use concat_project::commands::{ClipMove, ClipPatch, Command, TrackFlag, TrimEdge};
use concat_project::model::{
    AppliedFilter, ColorRange, Crop, KeyEase, KeyProperty, SpeedPoint, TextStyle, Transition,
    VideoSettings,
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
fn picture(path: &Path, rate: FrameRate, seconds: u32, codec: VideoCodec) {
    let options = EncodeOptions {
        codec,
        preset: "ultrafast".to_owned(),
        crf: 16,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 0,
        software_only: false,
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

/// The four quadrant colours of [`quadrants`], top left, top right, bottom
/// left, bottom right: saturated and far apart, so they survive 4:2:0 and
/// a flip or a crop is read off which one lands where.
const QUADRANTS: [[u8; 3]; 4] = [[220, 30, 30], [30, 200, 30], [30, 30, 220], [230, 220, 30]];

/// Two seconds of a picture split into four coloured quadrants, silent.
/// The geometry source: a solid frame looks the same flipped or cropped.
fn quadrants(path: &Path) {
    let options = EncodeOptions {
        codec: VideoCodec::H264,
        preset: "ultrafast".to_owned(),
        crf: 12,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 0,
        software_only: false,
    };
    let mut encoder = Encoder::create(path, WIDTH, HEIGHT, FrameRate::THIRTY, &options)
        .expect("the linked FFmpeg encodes");
    let mut frame = Frame::black(WIDTH, HEIGHT);
    let stride = WIDTH as usize * 4;
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            let quadrant =
                usize::from(y >= HEIGHT as usize / 2) * 2 + usize::from(x >= WIDTH as usize / 2);
            let [r, g, b] = QUADRANTS[quadrant];
            let at = y * stride + x * 4;
            frame.pixels_mut()[at..at + 4].copy_from_slice(&[r, g, b, 255]);
        }
    }
    for _ in 0..60 {
        encoder.write_frame(&frame).expect("writes a frame");
    }
    encoder.finish().expect("finishes the quadrants");
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
        Sources::make_in(dir, VideoCodec::H264)
    }

    /// The same set with every picture in `codec`.
    fn make_in(dir: &Path, codec: VideoCodec) -> Sources {
        let wav = dir.join("clock.wav");
        clock_wav(&wav, 6);
        let aac = dir.join("clock.m4a");
        clock_aac(&wav, &aac, 6);

        let peek_picture = dir.join("peek-picture.mp4");
        picture(&peek_picture, FrameRate::SIXTY, 6, codec);
        let peek = dir.join("peek.mp4");
        sound::mux(&peek_picture, &aac, &peek).expect("joins the screen recording");

        let cam_picture = dir.join("cam-picture.mp4");
        picture(&cam_picture, FrameRate::THIRTY, 4, codec);
        let cam_sound = dir.join("cam.m4a");
        clock_aac(&wav, &cam_sound, 4);
        let cam = dir.join("cam.mp4");
        sound::mux(&cam_picture, &cam_sound, &cam).expect("joins the camera clip");

        let silent = dir.join("silent.mp4");
        picture(&silent, FrameRate::FILM, 3, codec);

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

    /// The colour at (`x`, `y`), as fractions of the frame, in the frame
    /// shown at `at` seconds.
    fn colour_at_point(&self, at: f64, x: f64, y: f64) -> [u8; 3] {
        let index = ((at * self.fps).floor() as usize).min(self.frames.len() - 1);
        let frame = &self.frames[index];
        let px = ((f64::from(frame.width()) * x) as u32).min(frame.width() - 1);
        let py = ((f64::from(frame.height()) * y) as u32).min(frame.height() - 1);
        let [r, g, b, _] = frame.pixel(px, py).expect("inside the frame");
        [r, g, b]
    }

    /// The colours at each (`x`, `y`) point of the frame at `at` are the
    /// ones named, within what 4:2:0 and the encoder move a colour by.
    fn expect_colours(&self, at: f64, points: &[((f64, f64), [u8; 3])]) {
        for &((x, y), want) in points {
            let got = self.colour_at_point(at, x, y);
            let near = got
                .iter()
                .zip(want)
                .all(|(got, want)| (i16::from(*got) - i16::from(want)).abs() <= 40);
            assert!(
                near,
                "{}: at {at}s the point ({x}, {y}) is {got:?}, not {want:?}",
                self.label
            );
        }
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
    /// The range the next export is written in; video range, as the
    /// sheet's default is, unless a test says otherwise.
    range: concat_media::ColorRange,
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
            [0, 0, 0, 255],
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
            range: concat_media::ColorRange::Limited,
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
        // Each export names itself before it starts and again once the
        // file is written, so a run that hangs (every_edit_still_exports
        // on Windows CI, at the time of writing) says which edit and which
        // half: the render, or the reopen and read-back after it. Shown
        // with --nocapture, or under a failure.
        eprintln!("{label}: exporting");
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
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            color_range: self.range,
            hdr: false,
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
        eprintln!("{label}: written, reading it back");

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
        canvas: [0, 0, 0, 255],
        width,
        height,
        rate_num,
        rate_den,
        color_space: Default::default(),
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

/// Issue #202: on Windows laptops with NVIDIA chips an export took the app
/// down at its first frame, with nothing in the log. The export had opened
/// a device of its own through every API wgpu was built with, where the
/// window draws on one API and one device that has worked for every
/// monitor frame. So the window lends the export a sibling of the monitor's
/// compositor, on the window's device: an export drawn on a lent compositor
/// writes the same picture as one drawn on a device of its own.
#[test]
fn an_export_draws_on_a_lent_compositor() {
    let Some(window) = export::WgpuCompositor::new() else {
        eprintln!("no GPU or software renderer here; nothing to lend");
        return;
    };
    let scratch = Scratch::new("lent");
    let source = scratch.path().join("clock.mp4");
    picture(&source, FrameRate::THIRTY, 2, VideoCodec::H264);
    let mut studio = Studio::new(scratch.path(), "Lent", video(WIDTH, HEIGHT, 30, 1));
    let media = studio.import(&source);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: media,
        start: 0.0,
    });
    let spec = ExportSpec {
        output: scratch
            .path()
            .join("lent.mp4")
            .to_string_lossy()
            .into_owned(),
        crf: 18,
        preset: "ultrafast".to_owned(),
        codec: VideoCodec::H264,
        ten_bit: false,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        color_range: concat_media::ColorRange::Limited,
        hdr: false,
    };
    let request = export::request(&studio.session, &spec, Vec::new());
    let written = export::run_on(
        &request,
        Some(window.sibling()),
        &AtomicBool::new(false),
        |_| {},
    )
    .unwrap_or_else(|error| panic!("the export on a lent compositor failed: {error}"));

    let exported = Exported::read("lent", Path::new(&written), (30, 1));
    exported.expect_length(2.0);
    exported.expect_second(0.5, 0);
    exported.expect_second(1.5, 1);
    exported.expect_no_sound();
}

/// A crop and the flips reach the exported picture, in source terms: the
/// crop names the source's own edges, the flips mirror what the crop kept,
/// and a cropped picture is fitted into the frame and centred, bars
/// around it. Pinned before the geometry moves out of the decoder's
/// filters and into the frame plan (phase 1 of the HDR plan), so the move
/// is held to the picture people already get.
#[test]
fn crop_and_flips_reach_the_picture() {
    let scratch = Scratch::new("geometry");
    let path = scratch.path().join("quadrants.mp4");
    quadrants(&path);
    let [red, green, blue, yellow] = QUADRANTS;
    let black = [0, 0, 0];
    let (left, right, top, bottom) = (0.25, 0.75, 0.25, 0.75);

    let mut studio = Studio::new(scratch.path(), "Geometry", video(WIDTH, HEIGHT, 30, 1));
    let media = studio.import(&path);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: media,
            start: 0.0,
        })
        .expect("the clip has an id");
    let set = |studio: &mut Studio, crop: Option<Crop>, flip_h: bool, flip_v: bool| {
        studio.apply(Command::UpdateClip {
            clip_id: clip.clone(),
            patch: ClipPatch {
                crop: Some(crop),
                flip_h: Some(flip_h),
                flip_v: Some(flip_v),
                ..ClipPatch::default()
            },
        });
    };

    let plain = studio.export("quadrants as they are");
    plain.expect_colours(
        1.0,
        &[
            ((left, top), red),
            ((right, top), green),
            ((left, bottom), blue),
            ((right, bottom), yellow),
        ],
    );

    set(&mut studio, None, true, false);
    studio.export("flipped across").expect_colours(
        1.0,
        &[
            ((left, top), green),
            ((right, top), red),
            ((left, bottom), yellow),
            ((right, bottom), blue),
        ],
    );

    set(&mut studio, None, false, true);
    studio.export("flipped upside down").expect_colours(
        1.0,
        &[
            ((left, top), blue),
            ((right, top), yellow),
            ((left, bottom), red),
            ((right, bottom), green),
        ],
    );

    // The right half kept: a picture half as wide, centred, bars either side.
    let right_half = Crop {
        left: 0.5,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    set(&mut studio, Some(right_half), false, false);
    studio.export("left half cropped away").expect_colours(
        1.0,
        &[
            ((0.4, top), green),
            ((0.6, top), green),
            ((0.4, bottom), yellow),
            ((0.6, bottom), yellow),
            ((0.05, 0.5), black),
            ((0.95, 0.5), black),
        ],
    );

    // The bottom half kept, then mirrored: the crop is taken on the
    // source, before the flip, so blue and yellow trade sides.
    let bottom_half = Crop {
        left: 0.0,
        top: 0.5,
        right: 0.0,
        bottom: 0.0,
    };
    set(&mut studio, Some(bottom_half), true, false);
    studio
        .export("top half cropped away, then flipped")
        .expect_colours(
            1.0,
            &[
                ((left, 0.5), yellow),
                ((right, 0.5), blue),
                ((0.5, 0.05), black),
                ((0.5, 0.95), black),
            ],
        );
}

/// A background fills the bars of a wide picture in a tall frame - a
/// colour, or the picture itself enlarged and blurred - and the monitor
/// shows what the export writes: the paused monitor's frame, drawn from the
/// same flattened clips, agrees with the file at every point checked. A
/// fill then takes the bars away altogether.
#[test]
fn a_background_fills_the_bars_in_the_export_and_the_monitor() {
    use concat_project::model::{Background, Framing};
    let scratch = Scratch::new("background");
    let path = scratch.path().join("quadrants.mp4");
    quadrants(&path);
    let [red, green, blue, yellow] = QUADRANTS;
    let black = [0, 0, 0];
    let (width, height) = (90, 160);

    let mut studio = Studio::new(scratch.path(), "Background", video(width, height, 30, 1));
    let media = studio.import(&path);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: media,
            start: 0.0,
        })
        .expect("the clip has an id");
    let timeline_id = studio.project().active_timeline_id.clone();
    // The picture is 90x51 across the middle of the frame: its quadrants
    // either side of (0.5, 0.5), the bars above 0.34 and below 0.66.
    let picture = [
        ((0.25, 0.42), red),
        ((0.75, 0.42), green),
        ((0.25, 0.58), blue),
        ((0.75, 0.58), yellow),
    ];
    let monitor = |studio: &Studio| {
        let request = concat_export::PreviewFrameRequest {
            canvas: [0, 0, 0, 255],
            time: 1.0,
            width,
            height,
            rate_num: 30,
            rate_den: 1,
            clips: studio.session.flattened_clips(),
            color_space: Default::default(),
        };
        let pixels =
            concat_export::preview_frame(&concat_media::ReaderPool::with_defaults(), &request)
                .expect("the monitor draws the frame");
        Frame::from_rgba(width, height, pixels).expect("a whole frame")
    };
    let agree = |label: &str, exported: &Exported, shown: &Frame| {
        for y in [0.1, 0.25, 0.42, 0.58, 0.75, 0.9] {
            for x in [0.1, 0.25, 0.4, 0.6, 0.75, 0.9] {
                let file = exported.colour_at_point(1.0, x, y);
                let [r, g, b, _] = shown
                    .pixel(
                        (f64::from(width) * x) as u32,
                        (f64::from(height) * y) as u32,
                    )
                    .expect("inside the frame");
                assert!(
                    file.iter()
                        .zip([r, g, b])
                        .all(|(file, shown)| file.abs_diff(shown) <= 24),
                    "{label}: at ({x}, {y}) the file is {file:?} and the monitor {:?}",
                    [r, g, b]
                );
            }
        }
    };

    let plain = studio.export("no background");
    plain.expect_colours(1.0, &picture);
    plain.expect_colours(1.0, &[((0.5, 0.1), black), ((0.5, 0.9), black)]);
    agree("no background", &plain, &monitor(&studio));

    let set = |studio: &mut Studio, background: Background| {
        studio.apply(Command::SetTimelineBackground {
            timeline_id: timeline_id.clone(),
            background,
        });
    };
    let teal = [30, 160, 140];
    set(
        &mut studio,
        Background::Color {
            color: "#1ea08c".to_owned(),
        },
    );
    let coloured = studio.export("a colour background");
    coloured.expect_colours(1.0, &picture);
    coloured.expect_colours(1.0, &[((0.5, 0.1), teal), ((0.5, 0.9), teal)]);
    agree("a colour background", &coloured, &monitor(&studio));

    // The rest is over the timeline's blur, a wide Gaussian, and on
    // Windows' software adapter (WARP) its first frame outran the device
    // wait and the test's five minutes; there, as for the glow, it is left
    // out unless CONCAT_REQUIRE_GPU is set. Linux's lavapipe and every real
    // GPU still draw it.
    let software = export::WgpuCompositor::new().is_some_and(|gpu| gpu.on_software_adapter());
    if software && std::env::var_os("CONCAT_REQUIRE_GPU").is_none() {
        eprintln!("a software adapter only, and CONCAT_REQUIRE_GPU is not set; no blur");
        return;
    }

    // Enlarged to cover the frame, the picture's middle third fills it:
    // the left quadrants on the left, the right ones on the right, red and
    // green above, blue and yellow below, a little softened.
    set(&mut studio, Background::Blur { amount: 0.3 });
    let blurred = studio.export("a blurred background");
    blurred.expect_colours(1.0, &picture);
    blurred.expect_colours(
        1.0,
        &[
            ((0.15, 0.1), red),
            ((0.85, 0.1), green),
            ((0.15, 0.9), blue),
            ((0.85, 0.9), yellow),
        ],
    );
    agree("a blurred background", &blurred, &monitor(&studio));

    // The clip's own black over the timeline's blur.
    studio.apply(Command::UpdateClip {
        clip_id: clip.clone(),
        patch: ClipPatch {
            background: Some(Some(Background::None)),
            ..ClipPatch::default()
        },
    });
    studio
        .export("the clip's own black")
        .expect_colours(1.0, &[((0.5, 0.1), black), ((0.5, 0.9), black)]);

    // Filled, the picture is the frame: no bars to fill.
    studio.apply(Command::FrameClip {
        clip_id: clip,
        framing: Framing::Fill,
        at: 0.0,
    });
    let filled = studio.export("filled");
    filled.expect_colours(
        1.0,
        &[
            ((0.4, 0.1), red),
            ((0.6, 0.1), green),
            ((0.4, 0.9), blue),
            ((0.6, 0.9), yellow),
        ],
    );
    agree("filled", &filled, &monitor(&studio));
}

/// Three seconds of one flat colour, silent.
fn solid(path: &Path, colour: [u8; 3]) {
    let options = EncodeOptions {
        codec: VideoCodec::H264,
        preset: "ultrafast".to_owned(),
        crf: 12,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: false,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 0,
        software_only: false,
    };
    let mut encoder = Encoder::create(path, WIDTH, HEIGHT, FrameRate::THIRTY, &options)
        .expect("the linked FFmpeg encodes");
    let mut frame = Frame::black(WIDTH, HEIGHT);
    let [r, g, b] = colour;
    frame.fill([r, g, b, 255]);
    for _ in 0..90 {
        encoder.write_frame(&frame).expect("writes a frame");
    }
    encoder.finish().expect("finishes the colour");
}

/// The fade-to-colour and wipe transitions reach the exported picture
/// where the timeline says: a wipe shows the old picture on one side and
/// the new on the other halfway through, and a fade to black or white is
/// the colour outright at the cut, the pictures whole either side of it.
/// Pinned before the transitions move out of the decoder's filters and
/// into the frame plan (phase 1 of the HDR plan).
#[test]
fn fades_and_wipes_reach_the_picture() {
    let scratch = Scratch::new("transitions");
    let (red_path, blue_path) = (
        scratch.path().join("red.mp4"),
        scratch.path().join("blue.mp4"),
    );
    let (red, blue) = ([220, 30, 30], [30, 30, 220]);
    solid(&red_path, red);
    solid(&blue_path, blue);

    let mut studio = Studio::new(scratch.path(), "Transitions", video(WIDTH, HEIGHT, 30, 1));
    let red_media = studio.import(&red_path);
    let blue_media = studio.import(&blue_path);
    let first = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: red_media,
            start: 0.0,
        })
        .expect("the clip has an id");
    let track = studio.project().active().tracks[0].id.clone();
    let second = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: blue_media,
            start: 3.0,
        })
        .expect("the clip has an id");
    // A second of handle at the head of the blue clip, for an overlap to
    // take, and the red one ending at 2 s, where the blue one then starts.
    studio.apply(Command::TrimClip {
        clip_id: second.clone(),
        edge: TrimEdge::Start,
        delta: 1.0,
        ripple: false,
    });
    studio.apply(Command::TrimClip {
        clip_id: first.clone(),
        edge: TrimEdge::End,
        delta: -1.0,
        ripple: false,
    });
    studio.apply(Command::MoveClips {
        moves: vec![ClipMove {
            clip_id: second.clone(),
            start: 2.0,
            track_id: track,
        }],
    });
    let with = |studio: &mut Studio, kind: &str| {
        studio.apply(Command::UpdateClip {
            clip_id: second.clone(),
            patch: ClipPatch {
                transition_in: Some(Some(Transition {
                    id: kind.to_owned(),
                    duration: 1.0,
                    params: Default::default(),
                })),
                ..ClipPatch::default()
            },
        });
        studio.export(&format!("transition {kind}"))
    };

    // The wipes overlap the two clips for the second centred on the cut.
    let wiped = with(&mut studio, "wipe-left");
    wiped.expect_colours(0.5, &[((0.5, 0.5), red)]);
    wiped.expect_colours(2.0, &[((0.2, 0.5), red), ((0.8, 0.5), blue)]);
    wiped.expect_colours(2.9, &[((0.5, 0.5), blue)]);
    let wiped = with(&mut studio, "wipe-right");
    wiped.expect_colours(2.0, &[((0.2, 0.5), blue), ((0.8, 0.5), red)]);

    // The fades split the second across the cut, and do not overlap.
    for (kind, colour) in [("fade-black", [0, 0, 0]), ("fade-white", [255, 255, 255])] {
        let faded = with(&mut studio, kind);
        faded.expect_colours(1.2, &[((0.5, 0.5), red)]);
        faded.expect_colours(2.0, &[((0.5, 0.5), colour)]);
        faded.expect_colours(2.9, &[((0.5, 0.5), blue)]);
        let middle = faded.colour_at_point(1.75, 0.5, 0.5);
        assert!(
            middle != red
                && middle
                    .iter()
                    .zip(colour)
                    .any(|(got, to)| got.abs_diff(to) > 40),
            "{kind}: halfway into the fade the picture is {middle:?}"
        );
    }
}

/// An iPhone's HDR - ten-bit HEVC tagged HLG - exports through the new road:
/// decoded deep in its own signal, converted and conformed to the SDR
/// timeline on the GPU as it uploads. The picture is the grey it was
/// written as, conformed: neither black nor blown out, and not tinted.
#[test]
fn an_hlg_clip_exports_through_the_gpu_conversion() {
    if !VideoCodec::Hevc.available() {
        eprintln!("no HEVC encoder in the linked FFmpeg; skipped");
        return;
    }
    let scratch = Scratch::new("hlg");
    let path = scratch.path().join("hlg.mp4");
    {
        let options = EncodeOptions {
            codec: VideoCodec::Hevc,
            preset: "ultrafast".to_owned(),
            crf: 12,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            ten_bit: true,
            color_range: concat_media::ColorRange::Limited,
            hardware: false,
            threads: 0,
            software_only: false,
        };
        let mut encoder = Encoder::create_mislabelled_hdr(
            &path,
            WIDTH,
            HEIGHT,
            FrameRate::THIRTY,
            &options,
            false,
        )
        .expect("an HEVC encoder");
        let mut frame = Frame::black(WIDTH, HEIGHT);
        frame.fill([160, 160, 160, 255]);
        for _ in 0..30 {
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
    }
    let mut studio = Studio::new(scratch.path(), "HLG", video(WIDTH, HEIGHT, 30, 1));
    let media = studio.import(&path);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: media,
        start: 0.0,
    });
    let exported = studio.export("an HLG clip");
    let [r, g, b] = exported.colour_at(0.5);
    assert!(
        [r, g, b].iter().all(|channel| (40..=240).contains(channel))
            && r.abs_diff(g) <= 6
            && g.abs_diff(b) <= 6,
        "the HLG grey exported as {:?}",
        [r, g, b]
    );
}

/// An iPhone's HLG clip turns the timeline HLG, and the timeline exports as
/// HDR: HEVC in ten bits, BT.2020 and HLG, the grey the clip was written
/// as coming back as the same signal. The same timeline written SDR is
/// BT.709 in eight bits, its grey tone-mapped as the monitor shows it.
#[test]
fn an_hlg_timeline_exports_as_hdr_or_sdr() {
    use concat_core::frame::{Depth, Signal};
    use concat_media::{DecodeOptions, Decoder, FrameSource};
    if !VideoCodec::Hevc.available() {
        eprintln!("no HEVC encoder in the linked FFmpeg; skipped");
        return;
    }
    const LEVEL: f64 = 0.6;
    let scratch = Scratch::new("hlg-out");
    let path = scratch.path().join("iphone.mp4");
    let options = EncodeOptions {
        codec: VideoCodec::Hevc,
        preset: "ultrafast".to_owned(),
        crf: 12,
        rate_mode: RateMode::Vbr,
        bitrate_kbps: 0,
        ten_bit: true,
        color_range: concat_media::ColorRange::Limited,
        hardware: false,
        threads: 0,
        software_only: false,
    };
    {
        let mut encoder = Encoder::create_hdr(
            &path,
            WIDTH,
            HEIGHT,
            FrameRate::THIRTY,
            &options,
            Signal::Hlg,
        )
        .expect("an HEVC encoder");
        let value = (LEVEL * 65_535.0).round() as u16;
        let pixel: Vec<u8> = [value, value, value, u16::MAX]
            .iter()
            .flat_map(|channel| channel.to_le_bytes())
            .collect();
        let frame = Frame::from_rgba64(
            WIDTH,
            HEIGHT,
            pixel.repeat((WIDTH * HEIGHT) as usize),
            Signal::Hlg,
        )
        .expect("a deep frame");
        for _ in 0..30 {
            encoder.write_frame(&frame).expect("writes");
        }
        encoder.finish().expect("finishes");
    }
    let mut studio = Studio::new(scratch.path(), "HLG out", video(WIDTH, HEIGHT, 30, 1));
    let media = studio.import(&path);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: media,
        start: 0.0,
    });
    assert_eq!(
        studio.project().active().video.color_space,
        concat_project::model::ColorSpace::Hlg,
        "the timeline turned HLG with its first HDR clip"
    );

    let written = |hdr: bool, codec: VideoCodec| {
        let spec = ExportSpec {
            output: scratch
                .path()
                .join(format!("out-{hdr}.mp4"))
                .to_string_lossy()
                .into_owned(),
            crf: 12,
            preset: "ultrafast".to_owned(),
            codec,
            ten_bit: false,
            rate_mode: RateMode::Vbr,
            bitrate_kbps: 0,
            color_range: concat_media::ColorRange::Limited,
            hdr,
        };
        let request = export::request(&studio.session, &spec, Vec::new());
        export::run(&request, &AtomicBool::new(false), |_| {})
            .unwrap_or_else(|error| panic!("hdr {hdr}: the export failed: {error}"))
    };

    let hdr = written(true, VideoCodec::Hevc);
    let info = concat_media::probe::probe(&hdr).expect("probes");
    assert_eq!(info.video.expect("a picture").signal, Signal::Hlg);
    let mut decoder =
        Decoder::open(&hdr, &DecodeOptions::default().deep(true)).expect("opens the export");
    let frame = decoder.next_frame().expect("decodes").expect("a frame");
    assert_eq!(
        (frame.depth(), frame.signal()),
        (Depth::Sixteen, Signal::Hlg)
    );
    let at = ((HEIGHT / 2 * WIDTH + WIDTH / 2) * 8) as usize;
    let got = f64::from(u16::from_le_bytes([
        frame.pixels()[at],
        frame.pixels()[at + 1],
    ])) / 65_535.0;
    assert!(
        (got - LEVEL).abs() < 0.015,
        "the HLG grey {LEVEL} came back {got}"
    );

    let sdr = written(false, VideoCodec::H264);
    let info = concat_media::probe::probe(&sdr).expect("probes");
    assert_eq!(info.video.expect("a picture").signal, Signal::Sdr);
    let exported = Exported::read("SDR", Path::new(&sdr), (30, 1));
    let [r, g, b] = exported.colour_at(0.5);
    assert!(
        [r, g, b].iter().all(|channel| (60..=230).contains(channel))
            && r.abs_diff(g) <= 6
            && g.abs_diff(b) <= 6,
        "the HLG grey written SDR came out {:?}",
        [r, g, b]
    );
}

/// Issue #103: the levels a file is read as reach the export. A source
/// written video range reads as it is by default; told it is full range,
/// its levels expand and the colour of a second is no longer the colour
/// of that second; named limited, or cleared, it reads true again. And an
/// export written full range is tagged so and plays its seconds back
/// true, because the tag the encoder wrote is the tag the decoder reads.
/// https://github.com/jub0t/Concat/issues/103
#[test]
fn a_colour_range_named_on_the_media_reaches_the_export() {
    let scratch = Scratch::new("range");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Range", video(WIDTH, HEIGHT, 30, 1));
    let peek = studio.import(&sources.peek);
    studio.apply(Command::AddClipAtFirstFree {
        media_id: peek.clone(),
        start: 0.0,
    });

    let as_tagged = studio.export("as tagged");
    as_tagged.expect_second(0.5, 0);
    as_tagged.expect_second(3.5, 3);
    let tagged_colour = as_tagged.colour_at(3.5);

    // Told the file is full range when it is not: every level moves away
    // from the middle, so the colour of the second is no longer the colour
    // of the second. The file is unchanged; only its reading is.
    studio.apply(Command::SetMediaColorRange {
        media_id: peek.clone(),
        range: Some(ColorRange::Full),
    });
    let stretched = studio.export("read as full range");
    stretched.expect_length(6.0);
    let stretched_colour = stretched.colour_at(3.5);
    assert!(
        stretched_colour
            .iter()
            .zip(tagged_colour.iter())
            .any(|(now, was)| now.abs_diff(*was) > 8),
        "reading a video-range file as full range moves its levels: \
         {tagged_colour:?} read as tagged, {stretched_colour:?} read as full"
    );

    // Named what it really is, the picture is what the tag gave.
    studio.apply(Command::SetMediaColorRange {
        media_id: peek.clone(),
        range: Some(ColorRange::Limited),
    });
    studio.export("read as limited").expect_second(3.5, 3);

    // Cleared, the tag is read again.
    studio.apply(Command::SetMediaColorRange {
        media_id: peek,
        range: None,
    });
    studio.export("as tagged again").expect_second(3.5, 3);

    // Written full range: still the seconds it was, read back through the
    // tag it carries.
    studio.range = concat_media::ColorRange::Full;
    let full = studio.export("written full range");
    full.expect_length(6.0);
    full.expect_sound(6.0);
    full.expect_second(0.5, 0);
    full.expect_second(3.5, 3);
    full.expect_second(5.9, 5);
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
        ripple: false,
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
        ripple: false,
        clip_ids: vec![peek_clip.clone()],
    });
    let exported = studio.export("head removed");
    exported.expect_length(16.0);
    exported.expect_black(1.0);
    exported.expect_second(2.25, 2);

    // Retimed: twice as fast, then half speed with the pitch riding, then
    // on a curve, then undone back to half speed.
    studio.apply(Command::SetClipSpeed {
        clip_id: tail.clone(),
        ripple: false,
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
        ripple: false,
        speed: 0.5,
    });
    let exported = studio.export("half speed");
    exported.expect_second(2.25, 2);
    exported.expect_second(8.5, 5);
    exported.expect_sound(16.0);

    studio.apply(Command::SetClipSpeedCurve {
        clip_id: tail.clone(),
        ripple: false,
        curve: Some(vec![
            SpeedPoint {
                at: 0.0,
                speed: 0.5,
                slope: None,
            },
            SpeedPoint {
                at: 1.0,
                speed: 2.0,
                slope: None,
            },
        ]),
    });
    let exported = studio.export("speed curve");
    exported.expect_length(16.0);
    exported.expect_sound(16.0);

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
        ripple: false,
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
                    params: Default::default(),
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
            video_effects: Some(vec![effect("concat.vhs"), effect("concat.film-grain")]),
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

    // The transform, the crop, the flips, a blend, opacity, fades, gain.
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

    // A layer over everything, and a title. The glow is a wide Gaussian,
    // and on Windows' software adapter (WARP) its first frame ran past
    // twelve minutes; there, as for the effect cards, it is left out unless
    // CONCAT_REQUIRE_GPU is set. Linux's lavapipe and every real GPU still
    // export it. The layer sits inside the cut, so nothing after it depends
    // on it.
    let software = export::WgpuCompositor::new().is_some_and(|gpu| gpu.on_software_adapter());
    if software && std::env::var_os("CONCAT_REQUIRE_GPU").is_none() {
        eprintln!("a software adapter only, and CONCAT_REQUIRE_GPU is not set; no glow layer");
    } else {
        studio.apply(Command::AddLayerClip {
            track_id: None,
            start: 2.0,
            duration: Some(3.0),
            effect_id: "concat.glow".to_owned(),
            name: "Glow".to_owned(),
            sound: false,
        });
        let exported = studio.export("layer");
        exported.expect_length(end);
    }
    studio.apply(Command::AddTextClip {
        above: false,
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
        .apply(Command::AddTrack { bottom: false })
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
        ripple: false,
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
        ripple: false,
    });
    studio.apply(Command::SetClipSpeed {
        clip_id: clip.clone(),
        ripple: false,
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

/// Hardware decode preferred, the way the app always sets it: the
/// export decodes its sources on the platform's device where there is one
/// and reads the same picture and sound back.
///
/// A platform can name a device the machine cannot open: Windows' CI
/// runner has WARP, a software Direct3D adapter with no video decoder, so
/// D3D11VA falls back to the CPU there, as it should, and the reader never
/// reports the device. The device is tried first on its own, by name and
/// without touching the preference; where it does not decode, the test
/// skips and says so, unless the job says the GPU tests must run
/// (`CONCAT_REQUIRE_GPU`, as cards.rs reads it), in which case it fails.
///
/// The preference is the process's. Under nextest every test is a process
/// of its own and nothing else sees it; under `cargo test` the scenarios
/// running alongside share it while this one runs, and the guard puts back
/// whatever it was before, so none of them is left with it after.
#[test]
fn an_export_decodes_on_the_hardware_when_preferred() {
    let scratch = Scratch::new("hardware");
    // HEVC, not the H.264 the other scenarios use: 8-bit H.264 is the one
    // stream the preference leaves on the CPU, which decodes it faster
    // (concat_media::hardware::hardware_wins).
    let sources = Sources::make_in(scratch.path(), VideoCodec::Hevc);

    if let Some(device) = HwDevice::platform_default() {
        let probe = DecodeOptions {
            hardware: concat_media::HwPolicy::Device(device),
            ..DecodeOptions::default()
        };
        let mut reader = Decoder::open(&sources.peek, &probe).expect("opens");
        reader.next_frame().expect("decodes").expect("a frame");
        if reader.hardware() != Some(device) {
            let why = format!(
                "{} is this platform's device, but it does not decode HEVC on this machine \
                 (a software adapter, or no video decoder)",
                device.label()
            );
            if std::env::var_os("CONCAT_REQUIRE_GPU").is_some() {
                panic!("{why}, and CONCAT_REQUIRE_GPU is set");
            }
            eprintln!("{why}, and CONCAT_REQUIRE_GPU is not set; skipping");
            return;
        }
    }

    struct Restore(Option<HwDevice>);
    impl Drop for Restore {
        fn drop(&mut self) {
            concat_media::hardware::set_hardware_device(self.0);
        }
    }
    let _restore = Restore(concat_media::hardware::hardware_device());
    concat_media::set_hardware_decode(true);
    assert_eq!(
        concat_media::hardware_decode(),
        HwDevice::platform_default().is_some(),
        "on where the platform has a device, off where it has none"
    );

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
        ripple: false,
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

/// A sound layer over two seconds of the clock changes the export's sound
/// there and nowhere else. Two clips sound under it, the clock and a
/// second copy of it a lane up, so what it treats is their sum. The tone
/// of the clock's third second, inside the layer, comes out of the
/// low-pass changed; its first and fifth, either side, are what an export
/// without the layer has, give or take what the encoder does near the
/// layer's edges. Muting the layer's lane takes it off.
#[test]
fn a_sound_layer_treats_the_mix_for_its_span_alone() {
    let scratch = Scratch::new("sound-layer");
    let wav = scratch.path().join("clock.wav");
    clock_wav(&wav, 6);
    let mut studio = Studio::new(scratch.path(), "Sound layer", video(WIDTH, HEIGHT, 30, 1));
    let clock = studio.import(&wav);
    let lanes: Vec<String> = studio
        .project()
        .active()
        .tracks
        .iter()
        .map(|track| track.id.clone())
        .collect();
    for lane in &lanes[..2] {
        studio.apply(Command::AddClip {
            media_id: clock.clone(),
            track_id: lane.clone(),
            start: 0.0,
            ripple: false,
        });
    }
    let plain = studio.export("no layer");
    plain.expect_sound(6.0);

    let layer = studio
        .apply(Command::AddLayerClip {
            track_id: None,
            start: 2.0,
            duration: Some(2.0),
            effect_id: "concat.underwater".to_owned(),
            name: "Underwater".to_owned(),
            sound: true,
        })
        .expect("a layer has an id");
    let treated = studio.export("sound layer");
    treated.expect_length(6.0);
    treated.expect_sound(6.0);

    // How far apart two exports' sound is over `[from, to)`, as the root
    // mean square of their difference.
    let apart = |a: &Exported, b: &Exported, from: f64, to: f64| {
        let (a, b) = (
            a.audio.as_ref().expect("sound"),
            b.audio.as_ref().expect("sound"),
        );
        let at = |seconds: f64| (seconds * f64::from(RATE)) as usize * 2;
        let range = at(from)..at(to).min(a.len()).min(b.len());
        let count = range.len().max(1) as f64;
        let sum: f64 = range
            .map(|index| (f64::from(a[index]) - f64::from(b[index])).powi(2))
            .sum();
        (sum / count).sqrt()
    };
    let inside = apart(&plain, &treated, 3.1, 3.9);
    let before = apart(&plain, &treated, 1.1, 1.9);
    let after = apart(&plain, &treated, 5.1, 5.9);
    eprintln!("sound layer: {inside:.4} inside, {before:.4} before, {after:.4} after");
    assert!(
        inside > 0.1,
        "the layer's span is {inside:.4} from the plain mix"
    );
    assert!(
        before < 0.01,
        "before the layer the mix moved by {before:.4}"
    );
    assert!(after < 0.01, "after the layer the mix moved by {after:.4}");
    // The tone is still there to treat: a filter, not a mute.
    treated.expect_tone(1.5);
    treated.expect_tone(5.5);

    // A muted lane silences the layer as it silences a clip.
    let lane = studio.clip(&layer).track_id.clone();
    studio.apply(Command::SetTrackFlag {
        track_id: lane,
        flag: TrackFlag::Muted,
        value: true,
    });
    let muted = studio.export("sound layer muted");
    let off = apart(&plain, &muted, 3.1, 3.9);
    assert!(off < 0.01, "a muted layer still moved the mix by {off:.4}");
}

/// A reversed copy of a span plays its seconds backwards, picture and
/// sound, and comes out of the host's reverse job as one file the
/// timeline reads like any other. The screen recording's seconds 1 to 6
/// are three hundred frames, which span two of the job's segments, so the
/// segments come back in the right order too; the clock's tone, on the
/// source's odd seconds, falls where those seconds land in the copy.
/// Asked again, the copy already written is handed back untouched. A
/// sound clip's span comes back as a file of sound alone.
#[test]
fn a_reversed_span_plays_its_seconds_backwards() {
    let scratch = Scratch::new("reverse");
    let sources = Sources::make(scratch.path());
    let project = scratch.path().join("project");
    let reversers = reverse::Reversers::new();

    let peek = sources.peek.to_string_lossy().into_owned();
    let request = reverse::ReverseRequest {
        target: reverse::target_for(&project, &peek, 1.0, 5.0, false).expect("named"),
        media_path: peek,
        audio_only: false,
        start: 1.0,
        duration: 5.0,
    };
    let mut last = 0.0f32;
    let written = reversers
        .reverse(&request, &mut |fraction| {
            assert!(
                fraction >= last,
                "progress went back from {last} to {fraction}"
            );
            last = fraction;
        })
        .expect("reverses the span");
    assert_eq!(written, request.target);
    assert!(!reversers.is_busy(), "the slot is free again");

    let copy = Exported::read("reversed", &written, (60, 1));
    copy.expect_length(5.0);
    copy.expect_sound(5.0);
    // The copy's second t is the source's 6 - t: seconds 5, 4, 3, 2, 1.
    copy.expect_second(0.25, 5);
    copy.expect_second(2.5, 3);
    copy.expect_second(4.75, 1);
    // The clock rang on the odd seconds, which now open and close the
    // copy with a quiet even second between each pair.
    copy.expect_tone(0.25);
    copy.expect_quiet(1.5);
    copy.expect_tone(2.5);
    copy.expect_quiet(3.5);
    copy.expect_tone(4.75);

    let again = reversers
        .reverse(&request, &mut |_| {
            panic!("a copy on disk is not written again")
        })
        .expect("finds the copy");
    assert_eq!(again, written);

    let aac = sources.aac.to_string_lossy().into_owned();
    let sound_request = reverse::ReverseRequest {
        target: reverse::target_for(&project, &aac, 2.0, 3.0, true).expect("named"),
        media_path: aac,
        audio_only: true,
        start: 2.0,
        duration: 3.0,
    };
    let sound = reversers
        .reverse(&sound_request, &mut |_| {})
        .expect("reverses the sound");
    let info = concat_media::probe(&sound).expect("probes the sound");
    assert!(
        info.video.is_none() && info.audio.is_some(),
        "a sound clip's copy is sound alone"
    );
    let mut decoder = AudioDecoder::open(
        &sound,
        &AudioOptions {
            rate: RATE,
            channels: 2,
            format: SampleFormat::F32,
            ..AudioOptions::default()
        },
    )
    .expect("opens the sound");
    let heard = Exported {
        label: "reversed sound".to_owned(),
        fps: 1.0,
        frames: Vec::new(),
        audio: Some(decoder.collect_f32().expect("decodes the sound")),
    };
    heard.expect_sound(3.0);
    // Seconds 2 to 5 backwards: the copy's second t is the source's 5 - t,
    // so the odd second, 3, rings in the middle with quiet either side.
    heard.expect_quiet(0.5);
    heard.expect_tone(1.5);
    heard.expect_quiet(2.5);
}

/// The selection after deleting pauses is a new source, not a longer
/// window on the old recording. Rendering the replacement must never bring
/// the deleted seconds back, and its sound must make the same cuts.
#[test]
fn consolidation_after_pause_removal_preserves_picture_sound_and_undo() {
    use concat_host::consolidate::Consolidation;

    let scratch = Scratch::new("consolidation");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Merged", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&sources.peek);
    studio.apply(Command::AddClipAtFirstFree {
        media_id,
        start: 0.0,
    });
    for time in [1.0, 2.0, 3.0, 5.0] {
        studio.apply(Command::SplitClips {
            clip_ids: studio.clip_ids(),
            time,
        });
    }
    let removed = studio
        .project()
        .active()
        .clips
        .iter()
        .filter(|clip| clip.source_start == 1.0 || clip.source_start == 3.0)
        .map(|clip| clip.id.clone())
        .collect();
    studio.apply(Command::RemoveClips {
        clip_ids: removed,
        ripple: true,
    });
    let ids = studio.clip_ids();
    assert_eq!(ids.len(), 3);
    for id in &ids {
        studio.apply(Command::UpdateClip {
            clip_id: id.clone(),
            patch: ClipPatch {
                volume: Some(0.6),
                ..Default::default()
            },
        });
    }
    let before = studio.project().clone();
    let plan = Consolidation::new(studio.project(), &ids).expect("valid selection");
    let folder = Path::new(studio.session.path()).join("media");
    std::fs::create_dir_all(&folder).unwrap();
    let output = folder.join("merged.mp4");
    let request = plan.request(studio.project(), Path::new(studio.session.path()), &output);
    export::run(&request, &AtomicBool::new(false), |_| {}).expect("renders merged media");
    let exported = Exported::read("merged media", &output, (30, 1));
    exported.expect_length(3.0);
    exported.expect_sound(3.0);
    for (at, second) in [(0.5, 0), (1.5, 2), (2.5, 5)] {
        exported.expect_second(at, second);
    }
    exported.expect_quiet(0.5);
    exported.expect_quiet(1.5);
    exported.expect_tone(2.5);

    let item = media::probe(output.to_str().unwrap())
        .unwrap()
        .to_new_media();
    let command = plan
        .replacement(studio.project(), item)
        .expect("inputs unchanged");
    studio.apply(command);
    assert_eq!(studio.clip_ids().len(), 1);
    let merged = &studio.project().active().clips[0];
    assert_eq!(
        (merged.volume, merged.speed, merged.source_start),
        (1.0, 1.0, 0.0)
    );
    // The usual export also saves/reopens the project, exercising persistence.
    let final_export = studio.export("one merged clip");
    final_export.expect_length(3.0);
    final_export.expect_sound(3.0);
    final_export.expect_second(1.5, 2);
    final_export.expect_second(2.5, 5);
    final_export.expect_tone(2.5);
    studio.session.undo();
    assert_eq!(studio.project(), &before);
    studio.session.redo();
    assert_eq!(studio.clip_ids().len(), 1);

    let cancelled = AtomicBool::new(true);
    let cancelled_path = folder.join("cancelled.mp4");
    let request = plan.request(&before, Path::new(studio.session.path()), &cancelled_path);
    assert!(export::run(&request, &cancelled, |_| {}).is_err());
    assert!(
        !cancelled_path.exists(),
        "a cancelled render is never a completed source"
    );
}

#[test]
fn consolidation_of_silent_fractional_frames_covers_the_original_span() {
    use concat_host::consolidate::Consolidation;

    let scratch = Scratch::new("consolidation-fractional");
    let source = scratch.path().join("silent.mp4");
    picture(&source, FrameRate::THIRTY, 3, VideoCodec::H264);
    let mut studio = Studio::new(scratch.path(), "Fractional", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    let id = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 0.0,
        })
        .unwrap();
    studio.apply(Command::TrimClip {
        clip_id: id,
        edge: TrimEdge::End,
        delta: -1.92,
        ripple: false,
    });
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 0.54,
    });
    let plan = Consolidation::new(studio.project(), &studio.clip_ids()).unwrap();
    let output = scratch.path().join("merged.mp4");
    let request = plan.request(studio.project(), Path::new(studio.session.path()), &output);
    export::run(&request, &AtomicBool::new(false), |_| {}).unwrap();
    let exported = Exported::read("fractional source", &output, (30, 1));
    exported.expect_length(1.1); // 33 frames cover the original 1.08 seconds.
    exported.expect_no_sound();
    let item = media::probe(output.to_str().unwrap())
        .unwrap()
        .to_new_media();
    studio.apply(plan.replacement(studio.project(), item).unwrap());
    assert!((studio.project().active().clips[0].duration - 1.08).abs() < 1e-9);
}

#[test]
fn consolidation_preserves_the_timeline_clock_of_animated_effects() {
    use concat_host::consolidate::Consolidation;

    // Bloom is a wide blur rendered twice over, and on Windows' software
    // adapter (WARP) that ran past the test's five-minute limit; there, as
    // for the glow above, it is left out unless CONCAT_REQUIRE_GPU is set.
    let software = export::WgpuCompositor::new().is_some_and(|gpu| gpu.on_software_adapter());
    if software && std::env::var_os("CONCAT_REQUIRE_GPU").is_none() {
        eprintln!("a software adapter only, and CONCAT_REQUIRE_GPU is not set; skipping");
        return;
    }

    let scratch = Scratch::new("consolidation-clock");
    let source = scratch.path().join("bright.mp4");
    let mut encoder = Encoder::create(
        &source,
        WIDTH,
        HEIGHT,
        FrameRate::THIRTY,
        &EncodeOptions {
            preset: "ultrafast".into(),
            ..Default::default()
        },
    )
    .unwrap();
    let mut frame = Frame::black(WIDTH, HEIGHT);
    frame.fill([220, 220, 220, 255]);
    for _ in 0..60 {
        encoder.write_frame(&frame).unwrap();
    }
    encoder.finish().unwrap();
    let mut studio = Studio::new(scratch.path(), "Clock", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    let id = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 5.0,
        })
        .unwrap();
    studio.apply(Command::UpdateClip {
        clip_id: id,
        patch: ClipPatch {
            video_effects: Some(vec![AppliedFilter {
                id: "concat.bloom-pulse".into(),
                params: [("amount".into(), 100.0), ("speed".into(), 2.0)]
                    .into_iter()
                    .collect(),
                enabled: true,
                keys: Default::default(),
            }]),
            ..Default::default()
        },
    });
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 6.0,
    });
    let baseline = studio.export("before consolidation");
    let plan = Consolidation::new(studio.project(), &studio.clip_ids()).unwrap();
    let output = scratch.path().join("merged.mp4");
    let request = plan.request(studio.project(), Path::new(studio.session.path()), &output);
    assert_eq!(request.effect_time_offset, 5.0);
    export::run(&request, &AtomicBool::new(false), |_| {}).unwrap();
    let baked = Exported::read("baked effect", &output, (30, 1));
    for at in [0.2, 0.5, 0.8, 1.2, 1.5] {
        let expected = baseline.colour_at(5.0 + at);
        let got = baked.colour_at(at);
        assert!(
            got.iter()
                .zip(expected)
                .all(|(got, expected)| got.abs_diff(expected) <= 3),
            "effect phase changed at {at}s: expected {expected:?}, got {got:?}"
        );
    }
}

#[test]
fn consolidation_keeps_the_global_frame_grid_at_fractional_starts() {
    use concat_host::consolidate::Consolidation;

    let scratch = Scratch::new("consolidation-frame-grid");
    let source = scratch.path().join("source.mp4");
    picture(&source, FrameRate::THIRTY, 3, VideoCodec::H264);
    let mut studio = Studio::new(scratch.path(), "Grid", video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    let id = studio
        .apply(Command::AddClipAtFirstFree {
            media_id,
            start: 5.01,
        })
        .unwrap();
    studio.apply(Command::TrimClip {
        clip_id: id,
        edge: TrimEdge::End,
        delta: -0.98,
        ripple: false,
    });
    studio.apply(Command::SplitClips {
        clip_ids: studio.clip_ids(),
        time: 5.05,
    });
    // JSON parsing can normalize a raw subframe float by one ULP. Compare
    // actual persisted renders here, not the generic helper's exact f64
    // equality assertion; the boundary being tested is the video frame grid.
    let draw = |studio: &mut Studio, label: &str| {
        studio.session.save(None).unwrap();
        let reopened = Session::open(studio.session.path(), studio.session.settings()).unwrap();
        let output = studio.exports.join(format!("{label}.mp4"));
        let request = export::request(
            &reopened,
            &ExportSpec {
                output: output.to_string_lossy().into_owned(),
                crf: 16,
                preset: "fast".into(),
                ..Default::default()
            },
            Vec::new(),
        );
        export::run(&request, &AtomicBool::new(false), |_| {}).unwrap();
        Exported::read(label, &output, (30, 1))
    };
    let baseline = draw(&mut studio, "original-frame-grid");
    let plan = Consolidation::new(studio.project(), &studio.clip_ids()).unwrap();
    let output = scratch.path().join("merged.mp4");
    let request = plan.request(studio.project(), Path::new(studio.session.path()), &output);
    export::run(&request, &AtomicBool::new(false), |_| {}).unwrap();
    let baked = Exported::read("rebased grid", &output, (30, 1));
    let item = media::probe(output.to_str().unwrap())
        .unwrap()
        .to_new_media();
    studio.apply(plan.replacement(studio.project(), item).unwrap());
    let final_export = draw(&mut studio, "adopted-frame-grid");
    assert_eq!(final_export.frames.len(), baseline.frames.len());
    for frame in [0, 1, 2, 3, 4, 30, 60] {
        let expected = baseline.frames[150 + frame]
            .pixel(WIDTH / 2, HEIGHT / 2)
            .unwrap();
        let got = baked.frames[frame].pixel(WIDTH / 2, HEIGHT / 2).unwrap();
        let adopted = final_export.frames[150 + frame]
            .pixel(WIDTH / 2, HEIGHT / 2)
            .unwrap();
        assert!(
            adopted
                .iter()
                .zip(expected)
                .all(|(got, expected)| got.abs_diff(expected) <= 3),
            "adopted frame {frame}: expected {expected:?}, got {adopted:?}"
        );
        assert!(
            got.iter()
                .zip(expected)
                .all(|(got, expected)| got.abs_diff(expected) <= 3),
            "frame {frame}: expected {expected:?}, got {got:?}"
        );
    }
}
