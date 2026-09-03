// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
        }
    }
}

/// The graph between the codec and the caller, as one string.
fn audio_filter(options: &AudioOptions) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (options.start, options.duration) {
        (Some(start), Some(duration)) => {
            parts.push(format!("atrim=start={start:.6}:duration={duration:.6}"));
            parts.push("asetpts=PTS-STARTPTS".to_owned());
        }
        (Some(start), None) => {
            parts.push(format!("atrim=start={start:.6}"));
            parts.push("asetpts=PTS-STARTPTS".to_owned());
        }
        (None, Some(duration)) => {
            parts.push(format!("atrim=duration={duration:.6}"));
        }
        (None, None) => {}
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
    decoder: decoder::Audio,
    options: AudioOptions,
    graph: Option<filter::Graph>,
    /// The codec has been drained and the graph flushed; only what the sink
    /// still holds is left.
    flushed: bool,
    done: bool,
}

impl AudioDecoder {
    /// Opens `path`'s first audio stream.
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
        let stream = input
            .streams()
            .best(ffmpeg::media::Type::Audio)
            .ok_or_else(|| Error::NoAudioStream {
                path: path.to_path_buf(),
            })?;
        let stream_index = stream.index();
        let time_base = stream.time_base();
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
            let target = (start * f64::from(ffmpeg::sys::AV_TIME_BASE)) as i64;
            input
                .seek(target, ..=target)
                .map_err(|error| ffi::fail("seek", path, error))?;
        }

        Ok(Self {
            path: path.to_path_buf(),
            input,
            stream: stream_index,
            time_base,
            decoder,
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
            .and_then(|parser| parser.parse(&audio_filter(&self.options)))
            .map_err(|error| ffi::fail("filter graph", &self.path, error))?;
        graph
            .validate()
            .map_err(|error| ffi::fail("filter graph", &self.path, error))?;
        Ok(graph)
    }

    /// One decoded frame out of the codec, or `None` when the file is over.
    fn next_source(&mut self) -> Result<Option<Audio>> {
        loop {
            let mut frame = Audio::empty();
            match self.decoder.receive_frame(&mut frame) {
                Ok(()) => return Ok(Some(frame)),
                Err(ffmpeg::Error::Eof) => return Ok(None),
                Err(error) if ffi::is_again(&error) => {}
                Err(error) => return Err(ffi::fail("decode", &self.path, error)),
            }
            loop {
                let mut packet = ffmpeg::Packet::empty();
                match packet.read(&mut self.input) {
                    Ok(()) => {
                        if packet.stream() != self.stream {
                            continue;
                        }
                        match self.decoder.send_packet(&packet) {
                            Ok(()) => break,
                            Err(error) if ffi::is_again(&error) => break,
                            Err(error) => return Err(ffi::fail("send packet", &self.path, error)),
                        }
                    }
                    Err(ffmpeg::Error::Eof) => {
                        let _ = self.decoder.send_eof();
                        break;
                    }
                    Err(error) => return Err(ffi::fail("read", &self.path, error)),
                }
            }
        }
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
        };
        assert_eq!(
            audio_filter(&options),
            "atrim=start=1.500000:duration=2.000000,asetpts=PTS-STARTPTS,atempo=2.000000,\
             aresample=16000,aformat=sample_fmts=flt:sample_rates=16000:channel_layouts=mono"
        );
    }

    #[test]
    fn a_whole_file_has_no_trim() {
        let graph = audio_filter(&AudioOptions::default());
        assert!(graph.starts_with("aresample=48000,"), "{graph}");
        assert!(graph.ends_with("channel_layouts=stereo"), "{graph}");
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        assert!(AudioDecoder::open("does-not-exist.mp3", &AudioOptions::default()).is_err());
    }
}
