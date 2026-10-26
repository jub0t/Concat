// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Pulling samples out of a file.
//!
//! One audio decoder for everything that needs sound as numbers: the
//! waveform peaks, the playback cache, the transcriber. Each asks for a
//! window of the file, a filter chain to run it through, and the rate,
//! channel count and sample format it wants back; libavfilter does the
//! trimming, the filtering and the resampling in one graph, so a clip's
//! speed and effects mean exactly the same thing here as in the export mix.

use std::path::{Path, PathBuf};

use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::decoder;
use ffmpeg_the_third::filter;
use ffmpeg_the_third::format;
use ffmpeg_the_third::util::frame::audio::Audio;

use crate::error::{Error, Result};
use crate::ffi;

/// The sample format decoded audio comes back in. Interleaved either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleFormat {
    /// Signed 16-bit.
    I16,
    /// 32-bit float in [-1, 1].
    F32,
}

impl SampleFormat {
    fn name(self) -> &'static str {
        match self {
            SampleFormat::I16 => "s16",
            SampleFormat::F32 => "flt",
        }
    }

    fn bytes(self) -> usize {
        match self {
            SampleFormat::I16 => 2,
            SampleFormat::F32 => 4,
        }
    }
}

/// What to decode and how it should come back.
#[derive(Clone, Debug)]
pub struct AudioOptions {
    /// Seconds into the file to start. The cut is exact: the container
    /// seeks near it, the graph trims to it.
    pub start: Option<f64>,
    /// Seconds of source to decode from `start`, or the rest of the file.
    pub duration: Option<f64>,
    /// FFmpeg audio filters to run the window through, in order, before
    /// resampling. Each is validated like a clip's chain.
    pub filters: Vec<String>,
    /// Samples per second to come back at.
    pub rate: u32,
    /// One for mono, two for stereo.
    pub channels: u16,
    /// The sample format to come back in.
    pub format: SampleFormat,
    /// Which audio stream of the file, by its index in the file. `None` is
    /// the first audio stream in file order, which is also what a file with
    /// one stream has; see `probe::audio_stream_index`.
    pub stream: Option<usize>,
}

impl Default for AudioOptions {
    fn default() -> Self {
        Self {
            start: None,
            duration: None,
            filters: Vec::new(),
            rate: 48_000,
            channels: 2,
            format: SampleFormat::I16,
            stream: None,
        }
    }
}

/// The graph between the codec and the caller, as one string. `origin` is
/// where the file starts, in seconds: the decoded frames carry it on their
/// timestamps, so a cut `start` seconds into the sound is made at
/// `origin + start` (see `ffi::origin`). `late` is how long after that the
/// sound's first sample comes - a file whose picture starts first - and is
/// put in front as silence, so the sound stays with its picture.
fn audio_filter(options: &AudioOptions, origin: f64, late: f64) -> String {
    let mut parts: Vec<String> = Vec::new();
    let pad = (late > 0.0005).then(|| format!("adelay=delays={:.3}:all=1", late * 1000.0));
    match (options.start.map(|start| start + origin), options.duration) {
        (Some(start), Some(duration)) => {
            parts.push(format!("atrim=start={start:.6}:duration={duration:.6}"));
            parts.push("asetpts=PTS-STARTPTS".to_owned());
        }
        (Some(start), None) => {
            parts.push(format!("atrim=start={start:.6}"));
            parts.push("asetpts=PTS-STARTPTS".to_owned());
        }
        // Sound that starts before the picture is cut where the picture
        // starts, as a cut would cut it.
        (None, Some(duration)) if late < -0.0005 => {
            parts.push(format!("atrim=start={origin:.6}:duration={duration:.6}"));
            parts.push("asetpts=PTS-STARTPTS".to_owned());
        }
        (None, None) if late < -0.0005 => {
            parts.push(format!("atrim=start={origin:.6}"));
            parts.push("asetpts=PTS-STARTPTS".to_owned());
        }
        (None, Some(duration)) => {
            parts.push(format!("atrim=duration={duration:.6}"));
        }
        (None, None) => {}
    }
    // After the cut, which restamps from its first sample; and cut again
    // to the length asked for, which the silence has pushed out.
    if let Some(pad) = pad {
        parts.push(pad);
        if let Some(duration) = options.duration {
            parts.push(format!("atrim=duration={duration:.6}"));
        }
    }
    parts.extend(
        options
            .filters
            .iter()
            .filter(|filter| !filter.is_empty())
            .cloned(),
    );
    parts.push(format!("aresample={}", options.rate));
    parts.push(format!(
        "aformat=sample_fmts={}:sample_rates={}:channel_layouts={}",
        options.format.name(),
        options.rate,
        if options.channels == 1 {
            "mono"
        } else {
            "stereo"
        }
    ));
    parts.join(",")
}

/// Decodes a file's audio through libavcodec and libavfilter.
pub struct AudioDecoder {
    path: PathBuf,
    input: format::context::Input,
    stream: usize,
    time_base: ffmpeg::Rational,
    /// The stream's first timestamp, in seconds: where its source time 0
    /// is, the way the picture and the export mix measure it.
    origin: f64,
    decoder: decoder::Audio,
    /// A packet the codec refused while full; see [`crate::feed`].
    held: crate::feed::Held<ffmpeg::Packet>,
    options: AudioOptions,
    graph: Option<filter::Graph>,
    /// The codec has been drained and the graph flushed; only what the sink
    /// still holds is left.
    flushed: bool,
    done: bool,
}

impl AudioDecoder {
    /// Opens the audio stream `options.stream` names, or `path`'s first.
    pub fn open(path: impl AsRef<Path>, options: &AudioOptions) -> Result<Self> {
        ffi::init();
        for chain in &options.filters {
            crate::audio::validate_chain(chain)?;
        }
        if options.channels != 1 && options.channels != 2 {
            return Err(Error::Probe {
                path: path.as_ref().to_path_buf(),
                detail: format!(
                    "{} channels requested; mono and stereo are the choices",
                    options.channels
                ),
            });
        }

        let path = path.as_ref();
        let mut input =
            ffmpeg::format::input(path).map_err(|error| ffi::fail("open", path, error))?;
        let stream_index =
            crate::probe::audio_stream_index(&input, options.stream).ok_or_else(|| {
                Error::NoAudioStream {
                    path: path.to_path_buf(),
                }
            })?;
        let stream = input.stream(stream_index).expect("just found");
        let time_base = stream.time_base();
        // A file whose sound starts late - MPEG-TS at 1.4 s, the MTS files
        // cameras write - is measured from where the file starts, as the
        // picture and the export mix are; without this the preview's sound
        // and a transcript ran ahead of the picture (audit 2026-10-04, #3).
        let origin = ffi::origin(&input).as_f64();
        let context = ffmpeg::codec::Context::from_parameters(stream.parameters())
            .map_err(|error| ffi::fail("codec parameters", path, error))?;
        let decoder = context
            .decoder()
            .audio()
            .map_err(|error| ffi::fail("open decoder", path, error))?;

        // Near the start, not at it: the graph's atrim does the exact cut,
        // and a seek only saves decoding what comes before.
        if let Some(start) = options.start
            && start > 0.0
        {
            ffi::seek_audio(&mut input, stream_index, start + origin)
                .map_err(|error| ffi::fail("seek", path, error))?;
        }
        ffi::read_only(&mut input, stream_index);

        Ok(Self {
            path: path.to_path_buf(),
            input,
            stream: stream_index,
            time_base,
            origin,
            decoder,
            held: None,
            options: options.clone(),
            graph: None,
            flushed: false,
            done: false,
        })
    }

    /// Samples per second of what comes back.
    pub fn rate(&self) -> u32 {
        self.options.rate
    }

    /// Channels of what comes back.
    pub fn channels(&self) -> u16 {
        self.options.channels
    }

    fn build_graph(&self, first: &Audio) -> Result<filter::Graph> {
        // How long after the cut the first sample comes: past zero only for
        // sound that starts after the file's origin, the picture first.
        let late = first
            .timestamp()
            .and_then(|ticks| ffi::seconds(ticks, self.time_base))
            .map_or(0.0, |at| {
                at.as_f64() - self.origin - self.options.start.unwrap_or(0.0)
            });
        let mut graph = filter::Graph::new();
        let args = format!(
            "time_base={}/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
            self.time_base.numerator(),
            self.time_base.denominator(),
            first.rate(),
            first.format().name(),
            first.ch_layout().description()
        );
        let missing = |name: &str| Error::Missing {
            what: "filter",
            name: name.to_owned(),
        };
        graph
            .add(
                &filter::find("abuffer").ok_or_else(|| missing("abuffer"))?,
                "in",
                &args,
            )
            .map_err(|error| ffi::fail("buffer source", &self.path, error))?;
        graph
            .add(
                &filter::find("abuffersink").ok_or_else(|| missing("abuffersink"))?,
                "out",
                "",
            )
            .map_err(|error| ffi::fail("buffer sink", &self.path, error))?;
        graph
            .output("in", 0)
            .and_then(|parser| parser.input("out", 0))
            .and_then(|parser| parser.parse(&audio_filter(&self.options, self.origin, late)))
            .map_err(|error| ffi::fail("filter graph", &self.path, error))?;
        graph
            .validate()
            .map_err(|error| ffi::fail("filter graph", &self.path, error))?;
        Ok(graph)
    }

    /// One decoded frame out of the codec, or `None` when the file is over.
    fn next_source(&mut self) -> Result<Option<Audio>> {
        let read = || crate::feed::read_packet(&mut self.input, self.stream);
        let mut frame = crate::feed::next_frame(&mut self.decoder, &mut self.held, read)
            .map_err(|stall| crate::feed::fail(stall, &self.path))?;
        if let Some(frame) = frame.as_mut() {
            crate::ffi::name_channels(frame);
        }
        Ok(frame)
    }

    /// The next frame out of the graph: interleaved samples at the requested
    /// rate and layout, or `None` once the window is exhausted.
    pub fn next_frame(&mut self) -> Result<Option<Audio>> {
        loop {
            if self.done {
                return Ok(None);
            }
            if let Some(graph) = self.graph.as_mut() {
                let mut out = Audio::empty();
                let pulled = {
                    let mut context = graph.get("out").expect("the graph has an output");
                    context.sink().frame(&mut out)
                };
                match pulled {
                    Ok(()) => return Ok(Some(out)),
                    Err(ffmpeg::Error::Eof) => {
                        self.done = true;
                        return Ok(None);
                    }
                    Err(error) if ffi::is_again(&error) => {}
                    Err(error) => return Err(ffi::fail("filter output", &self.path, error)),
                }
                if self.flushed {
                    self.done = true;
                    return Ok(None);
                }
            }

            // The graph wants more. Feed it the next decoded frame, or tell
            // it the file is over.
            match self.next_source()? {
                Some(frame) => {
                    if self.graph.is_none() {
                        self.graph = Some(self.build_graph(&frame)?);
                    }
                    let graph = self.graph.as_mut().expect("just built");
                    let mut context = graph.get("in").expect("the graph has an input");
                    context
                        .source()
                        .add(&frame)
                        .map_err(|error| ffi::fail("filter", &self.path, error))?;
                }
                None => {
                    let Some(graph) = self.graph.as_mut() else {
                        // No audio frame at all - a stream that claims sound
                        // and delivers none.
                        self.done = true;
                        return Ok(None);
                    };
                    let mut context = graph.get("in").expect("the graph has an input");
                    let _ = context.source().flush();
                    self.flushed = true;
                }
            }
        }
    }

    /// The interleaved bytes of one frame from the sink: samples times
    /// channels times the sample width, without the row padding libav adds.
    pub fn bytes_of(&self, frame: &Audio) -> Vec<u8> {
        let wanted =
            frame.samples() * usize::from(self.options.channels) * self.options.format.bytes();
        let data = frame.data(0);
        data[..wanted.min(data.len())].to_vec()
    }

    /// The next frame as interleaved 16-bit samples. Only for decoders
    /// opened with [`SampleFormat::I16`].
    pub fn next_i16(&mut self) -> Result<Option<Vec<i16>>> {
        let Some(frame) = self.next_frame()? else {
            return Ok(None);
        };
        let bytes = self.bytes_of(&frame);
        Ok(Some(
            bytes
                .chunks_exact(2)
                .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                .collect(),
        ))
    }

    /// The next frame as interleaved floats. Only for decoders opened with
    /// [`SampleFormat::F32`].
    pub fn next_f32(&mut self) -> Result<Option<Vec<f32>>> {
        let Some(frame) = self.next_frame()? else {
            return Ok(None);
        };
        let bytes = self.bytes_of(&frame);
        Ok(Some(
            bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect(),
        ))
    }

    /// Everything left in the window, as interleaved floats.
    pub fn collect_f32(&mut self) -> Result<Vec<f32>> {
        let mut all = Vec::new();
        while let Some(chunk) = self.next_f32()? {
            all.extend_from_slice(&chunk);
        }
        Ok(all)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_graph_trims_filters_and_lands_on_the_requested_shape() {
        let options = AudioOptions {
            start: Some(1.5),
            duration: Some(2.0),
            filters: vec!["atempo=2.000000".to_owned(), String::new()],
            rate: 16_000,
            channels: 1,
            format: SampleFormat::F32,
            stream: None,
        };
        assert_eq!(
            audio_filter(&options, 0.0, 0.0),
            "atrim=start=1.500000:duration=2.000000,asetpts=PTS-STARTPTS,atempo=2.000000,\
             aresample=16000,aformat=sample_fmts=flt:sample_rates=16000:channel_layouts=mono"
        );
    }

    #[test]
    fn a_whole_file_has_no_trim() {
        let graph = audio_filter(&AudioOptions::default(), 1.4, 0.0);
        assert!(graph.starts_with("aresample=48000,"), "{graph}");
        assert!(graph.ends_with("channel_layouts=stereo"), "{graph}");
    }

    /// The trim is made in the stream's own time, its start put back on.
    #[test]
    fn a_late_stream_is_trimmed_from_its_first_sample() {
        let options = AudioOptions {
            start: Some(0.5),
            ..AudioOptions::default()
        };
        assert!(audio_filter(&options, 1.4, 0.0).starts_with("atrim=start=1.900000,"));
    }

    /// Sound that starts before the picture is cut where the picture does,
    /// with no seek to have cut it.
    #[test]
    fn sound_that_starts_early_is_cut_where_the_picture_starts() {
        let whole = audio_filter(&AudioOptions::default(), 0.5, -0.5);
        assert!(
            whole.starts_with("atrim=start=0.500000,asetpts=PTS-STARTPTS,aresample"),
            "{whole}"
        );
    }

    /// Sound that starts after the file's origin - after its picture - is
    /// put in with silence in front, after any cut, so it plays with the
    /// picture rather than ahead of it.
    #[test]
    fn sound_that_starts_late_is_put_in_after_silence() {
        let whole = audio_filter(&AudioOptions::default(), 0.0, 0.5);
        assert!(whole.starts_with("adelay=delays=500.000:all=1,"), "{whole}");
        let cut = AudioOptions {
            start: Some(0.2),
            duration: Some(1.0),
            ..AudioOptions::default()
        };
        assert!(
            audio_filter(&cut, 0.0, 0.3).starts_with(
                "atrim=start=0.200000:duration=1.000000,asetpts=PTS-STARTPTS,\
                 adelay=delays=300.000:all=1,atrim=duration=1.000000,"
            ),
            "{}",
            audio_filter(&cut, 0.0, 0.3)
        );
    }

    /// A window into a file whose sound starts at 1.4 s - silence, then a
    /// tone a second in - comes back measured from the sound's first
    /// sample: half a second in, the tone begins half a second later, as
    /// the picture and the export mix would have it.
    #[test]
    fn a_window_into_a_late_stream_lands_on_source_time() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/tone-at-1s.ts");
        let options = AudioOptions {
            start: Some(0.5),
            duration: Some(1.0),
            rate: 8_000,
            channels: 1,
            format: SampleFormat::F32,
            ..AudioOptions::default()
        };
        let mut decoder = AudioDecoder::open(path, &options).expect("opens");
        let mut samples: Vec<f32> = Vec::new();
        while let Some(frame) = decoder.next_frame().expect("decodes") {
            let bytes = &frame.data(0)[..frame.samples() * 4];
            samples.extend(
                bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
        let loud = |range: std::ops::Range<usize>| {
            samples[range]
                .iter()
                .map(|s| s.abs())
                .fold(0.0_f32, f32::max)
        };
        assert!(
            samples.len() >= 7_600,
            "a second of sound, got {}",
            samples.len()
        );
        assert!(loud(0..3_800) < 0.01, "silence before the tone");
        assert!(loud(4_200..7_600) > 0.08, "the tone after half a second");
    }

    /// Where a tone first sounds in what a decoder of `path` hands back,
    /// in seconds, at 8 kHz mono.
    fn tone_onset(path: &str, start: Option<f64>) -> f64 {
        let options = AudioOptions {
            start,
            rate: 8_000,
            channels: 1,
            format: SampleFormat::F32,
            ..AudioOptions::default()
        };
        let mut decoder = AudioDecoder::open(path, &options).expect("opens");
        let mut samples: Vec<f32> = Vec::new();
        while let Some(frame) = decoder.next_frame().expect("decodes") {
            let bytes = &frame.data(0)[..frame.samples() * 4];
            samples.extend(
                bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_ne_bytes([b[0], b[1], b[2], b[3]])),
            );
        }
        let first = samples
            .iter()
            .position(|sample| sample.abs() > 0.05)
            .expect("a tone");
        first as f64 / 8_000.0
    }

    /// A file's sound is measured from where its picture starts, so the two
    /// play together as they do in a player. Each fixture sounds a tone half
    /// a second after its first frame: one whose picture starts half a
    /// second after its sound, one whose sound starts after its picture.
    /// Each measured from its own first sample, the first played its tone a
    /// second in and the second a fifth of a second in (#457).
    #[test]
    fn sound_plays_with_the_picture_whichever_starts_first() {
        for fixture in ["picture-starts-late.mov", "sound-starts-late.mov"] {
            let path = format!("{}/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
            let whole = tone_onset(&path, None);
            assert!(
                (whole - 0.5).abs() < 0.01,
                "{fixture}: the tone sounds {whole:.3} s after the first frame"
            );
            let cut = tone_onset(&path, Some(0.2));
            assert!(
                (cut - 0.3).abs() < 0.01,
                "{fixture}: cut 0.2 s in, the tone sounds at {cut:.3} s"
            );
        }
    }

    /// A WAV whose header names no channel layout - the plain PCM header,
    /// which many recorders write - is still filtered: a low-pass under a
    /// 440 Hz tone all but takes it away. It used to pass the tone through
    /// untouched, the filter unable to name the channels it was given.
    #[test]
    fn a_wav_that_names_no_channels_is_still_filtered() {
        let rate: u32 = 48_000;
        let mut data = Vec::new();
        for index in 0..rate {
            let t = f64::from(index) / f64::from(rate);
            let value = (0.5 * (2.0 * std::f64::consts::PI * 440.0 * t).sin() * 32767.0) as i16;
            data.extend_from_slice(&value.to_le_bytes());
            data.extend_from_slice(&value.to_le_bytes());
        }
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 4).to_le_bytes());
        bytes.extend_from_slice(&4u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&data);
        let path = std::env::temp_dir().join(format!("concat-unnamed-{}.wav", std::process::id()));
        std::fs::write(&path, bytes).expect("writes the wav");

        let loudest = |filters: Vec<String>| {
            let options = AudioOptions {
                filters,
                ..AudioOptions::default()
            };
            let mut decoder = AudioDecoder::open(&path, &options).expect("opens");
            let mut samples: Vec<i16> = Vec::new();
            while let Some(frame) = decoder.next_frame().expect("decodes") {
                let bytes = decoder.bytes_of(&frame);
                samples.extend(
                    bytes
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]])),
                );
            }
            // Past the filter's first tenth of a second, where it settles.
            samples
                .iter()
                .skip(2 * 4_800)
                .map(|sample| i32::from(*sample).abs())
                .max()
                .unwrap_or(0)
        };
        let plain = loudest(Vec::new());
        let filtered = loudest(vec!["lowpass=f=60,lowpass=f=60".to_owned()]);
        let _ = std::fs::remove_file(&path);
        assert!(plain > 15_000, "the tone is there, at {plain}");
        assert!(filtered < 1_000, "the tone came through at {filtered}");
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(AudioDecoder::open("does-not-exist.mp3", &AudioOptions::default()).is_err());
    }

    /// A window cut into a file with both sound and picture starts its
    /// sound where the cut is, not behind it. The Program Stream
    /// interleaves its sound behind its picture, and the seek used to be
    /// made on the picture's timestamps, forward: it landed on the picture
    /// at the cut and handed the sound from after it - a frame or two of
    /// silence in front of every trimmed .mpg. The transport stream
    /// interleaves tightly and landed either way; it is here to prove the
    /// seek on the sound's own stream keeps that. The same stream remuxed
    /// into Matroska is sought on its picture's index instead, a second
    /// short (see `ffi::seek_audio`), and must land no later - the cut at
    /// 3 s is the picture's keyframe itself. Each fixture sounds a tone
    /// from half a second in, so every cut here is inside it.
    #[test]
    fn a_cut_starts_its_sound_where_the_cut_is_whichever_way_the_file_interleaves() {
        let fixtures = concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/");
        for (name, cut) in [
            ("sound-behind-the-picture.mpg", 1.0),
            ("sound-behind-the-picture.mpg", 3.5),
            ("keyframes-3s-apart.ts", 1.0),
            ("keyframes-3s-apart.ts", 4.0),
            ("keyframes-3s-apart.mkv", 1.0),
            ("keyframes-3s-apart.mkv", 3.0),
            ("keyframes-3s-apart.mkv", 4.0),
        ] {
            let onset = tone_onset(&format!("{fixtures}{name}"), Some(cut));
            assert!(
                onset < 0.03,
                "{name} cut {cut} s in: the tone starts {onset:.3} s behind the cut"
            );
        }
    }
}

/// Timings for the sound of a clip cut into a long file, by hand: what it
/// costs to reach the first sample at the top and minutes in, through the
/// seek as 0.2.8 made it and as it is made now, and how fast a window of
/// it decodes. `CONCAT_AUDIO_PROBE` names the files, separated by `:`.
#[cfg(test)]
mod probe {
    use super::*;
    use std::time::Instant;

    fn first_sample(path: &str, at: f64, old_seek: bool) -> std::time::Duration {
        let started = Instant::now();
        let options = AudioOptions {
            start: (!old_seek).then_some(at),
            duration: Some(1.0),
            ..AudioOptions::default()
        };
        let mut decoder = AudioDecoder::open(path, &options).expect("opens");
        if old_seek {
            // 0.2.8 read every stream's packets.
            // SAFETY: as in `ffi::read_only`.
            unsafe {
                let context = decoder.input.as_mut_ptr();
                for index in 0..(*context).nb_streams as usize {
                    (*(*(*context).streams.add(index))).discard = ffmpeg::Discard::Default.into();
                }
            }
        }
        if old_seek && at > 0.0 {
            // The seek 0.2.8 made: on the default stream, the first packet
            // at or past the target - then the cut by the graph.
            let target = ((at + decoder.origin) * f64::from(ffmpeg::sys::AV_TIME_BASE)) as i64;
            decoder.input.seek(target, ..=target).expect("seeks");
            decoder.options.start = Some(at);
        }
        decoder.next_frame().expect("decodes").expect("a frame");
        started.elapsed()
    }

    #[test]
    #[ignore = "a probe"]
    fn audio_seek_and_decode_timing() {
        let Ok(files) = std::env::var("CONCAT_AUDIO_PROBE") else {
            return;
        };
        for path in files.split(':') {
            for at in [0.0, 480.0, 1100.0] {
                let old = first_sample(path, at, true);
                let new = first_sample(path, at, false);
                eprintln!("{path} at {at:>6.0} s: first sample 0.2.8 seek {old:?}, now {new:?}");
            }
            for (at, window, old) in [
                (0.0, 300.0, true),
                (480.0, 300.0, true),
                (0.0, 300.0, false),
                (480.0, 300.0, false),
            ] {
                let started = Instant::now();
                let mut decoder = AudioDecoder::open(
                    path,
                    &AudioOptions {
                        start: Some(at),
                        duration: Some(window),
                        format: SampleFormat::I16,
                        ..AudioOptions::default()
                    },
                )
                .expect("opens");
                if old {
                    // SAFETY: as in `ffi::read_only`.
                    unsafe {
                        let context = decoder.input.as_mut_ptr();
                        for index in 0..(*context).nb_streams as usize {
                            (*(*(*context).streams.add(index))).discard =
                                ffmpeg::Discard::Default.into();
                        }
                    }
                }
                let mut bytes = 0usize;
                while let Some(frame) = decoder.next_frame().expect("decodes") {
                    bytes += decoder.bytes_of(&frame).len();
                }
                let took = started.elapsed();
                eprintln!(
                    "{path}: {window} s from {at} s, every stream read {old}: {took:?} ({:.0}x real time, {} MB)",
                    window / took.as_secs_f64(),
                    bytes / 1_000_000
                );
            }
        }
    }
}
