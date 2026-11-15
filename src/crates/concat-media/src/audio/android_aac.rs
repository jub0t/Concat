// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Android's own AAC encoder, through the NDK's MediaCodec. FFmpeg wraps a
//! phone's video encoders and not its audio one, so a mix to AAC on a phone
//! is encoded here - Google's encoder, licensed with Android, rather than
//! FFmpeg's - and FFmpeg only muxes it.

use std::path::Path;
use std::time::Duration;

use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::encoder;
use ndk::media::media_codec::{
    DequeuedInputBufferResult, DequeuedOutputBufferInfoResult, MediaCodec, MediaCodecDirection,
};
use ndk::media::media_format::MediaFormat;

use super::{AudioBus, AudioClip, MIX_RATE, run_mix};
use crate::error::{Error, Result};
use crate::ffi;

const CHANNELS: usize = 2;
const BITRATE: i32 = 192_000;
/// Samples per channel in one AAC frame, and so in each block fed.
const FRAME: usize = 1024;
/// MediaCodec's BUFFER_FLAG_CODEC_CONFIG and BUFFER_FLAG_END_OF_STREAM.
const CODEC_CONFIG: u32 = 2;
const END_OF_STREAM: u32 = 4;
/// How long a dequeue waits before the loop looks the other way.
const WAIT: Duration = Duration::from_millis(10);
/// How many samples Android's encoder holds its sound back by, which its
/// timestamps do not say: its packets are stamped from the first sample
/// fed, as if the encoder had no delay, and a click fed at 1.000 s decodes
/// 2048 samples late. Nor does it flush them at the end of the stream, so
/// the mix is followed by that much silence and a frame more, which the
/// cut to the mix's length then takes off. Measured on Google's encoder,
/// which is asked for by name so that this is the encoder it holds for.
const DELAY: i64 = 2048;

/// Google's own AAC encoder, the one every phone carries - Codec 2's since
/// Android 10, OMX's before - and failing both, whichever the phone offers.
const ENCODERS: [&str; 2] = ["c2.android.aac.encoder", "OMX.google.aac.encoder"];

/// The AudioSpecificConfig of AAC-LC at `rate` in stereo: the two bytes an
/// MP4 carries for the decoder, the same the encoder's own csd-0 holds.
fn audio_specific_config(rate: u32) -> [u8; 2] {
    const RATES: [u32; 13] = [
        96_000, 88_200, 64_000, 48_000, 44_100, 32_000, 24_000, 22_050, 16_000, 12_000, 11_025,
        8_000, 7_350,
    ];
    let index = RATES.iter().position(|r| *r == rate).unwrap_or(3) as u16;
    let bits = (2u16 << 11) | (index << 7) | ((CHANNELS as u16) << 3);
    bits.to_be_bytes()
}

/// One encoded frame, stamped in samples from the start of the mix.
struct Packet {
    data: Vec<u8>,
    pts: i64,
}

/// Android's AAC encoder, configured for the mix.
struct Aac {
    codec: MediaCodec,
    /// Samples per channel fed so far, which stamps the next block.
    fed: u64,
    /// The encoder has handed back its end of stream.
    finished: bool,
}

impl Aac {
    fn open(destination: &Path) -> Result<Self> {
        let fail = |detail: String| Error::Ffi {
            operation: "android aac encoder",
            path: destination.to_path_buf(),
            detail,
        };
        let codec = ENCODERS
            .iter()
            .find_map(|name| MediaCodec::from_codec_name(name))
            .or_else(|| MediaCodec::from_encoder_type("audio/mp4a-latm"))
            .ok_or_else(|| fail("no AAC encoder on this phone".to_owned()))?;
        let mut format = MediaFormat::new();
        format.set_str("mime", "audio/mp4a-latm");
        format.set_i32("sample-rate", MIX_RATE as i32);
        format.set_i32("channel-count", CHANNELS as i32);
        format.set_i32("bitrate", BITRATE);
        // AAC-LC, the profile every player decodes.
        format.set_i32("aac-profile", 2);
        format.set_i32("max-input-size", (FRAME * CHANNELS * 2 * 4) as i32);
        codec
            .configure(&format, None, MediaCodecDirection::Encoder)
            .map_err(|error| fail(format!("configure: {error}")))?;
        codec
            .start()
            .map_err(|error| fail(format!("start: {error}")))?;
        Ok(Aac {
            codec,
            fed: 0,
            finished: false,
        })
    }

    /// Feeds `pcm`, interleaved 16-bit stereo, and hands every packet the
    /// encoder has ready to `out`. With `pcm` empty and `last`, says the
    /// stream has ended and waits for the rest.
    fn feed(&mut self, mut pcm: &[u8], last: bool, out: &mut Vec<Packet>) -> Result<()> {
        let frame_bytes = CHANNELS * 2;
        let mut ended = false;
        while !pcm.is_empty() || (last && !ended) {
            if let DequeuedInputBufferResult::Buffer(mut buffer) = self
                .codec
                .dequeue_input_buffer(WAIT)
                .map_err(|error| self.fail(format!("input: {error}")))?
            {
                let room = buffer.buffer_mut();
                let take = pcm.len().min(room.len() / frame_bytes * frame_bytes);
                for (slot, byte) in room.iter_mut().zip(&pcm[..take]) {
                    slot.write(*byte);
                }
                let time = self.fed * 1_000_000 / u64::from(MIX_RATE);
                let flags = if take == pcm.len() && last {
                    ended = true;
                    END_OF_STREAM
                } else {
                    0
                };
                self.codec
                    .queue_input_buffer(buffer, 0, take, time, flags)
                    .map_err(|error| self.fail(format!("queue: {error}")))?;
                self.fed += (take / frame_bytes) as u64;
                pcm = &pcm[take..];
            }
            self.drain(out, false)?;
        }
        if last {
            self.drain(out, true)?;
        }
        Ok(())
    }

    /// Takes the packets that are ready, or with `to_end` every one up to
    /// the encoder's end of stream.
    fn drain(&mut self, out: &mut Vec<Packet>, to_end: bool) -> Result<()> {
        while !self.finished {
            match self
                .codec
                .dequeue_output_buffer(WAIT)
                .map_err(|error| self.fail(format!("output: {error}")))?
            {
                DequeuedOutputBufferInfoResult::Buffer(buffer) => {
                    let flags = buffer.info().flags();
                    let pts =
                        buffer.info().presentation_time_us() * i64::from(MIX_RATE) / 1_000_000;
                    // The buffer is the codec's whole allocation; the frame is
                    // the part the info names.
                    let offset = buffer.info().offset().max(0) as usize;
                    let size = buffer.info().size().max(0) as usize;
                    let frame = buffer
                        .buffer()
                        .get(offset..offset + size)
                        .unwrap_or_default();
                    if flags & CODEC_CONFIG == 0 && !frame.is_empty() {
                        out.push(Packet {
                            data: frame.to_vec(),
                            pts: pts - DELAY,
                        });
                    }
                    self.codec
                        .release_output_buffer(buffer, false)
                        .map_err(|error| self.fail(format!("release: {error}")))?;
                    if flags & END_OF_STREAM != 0 {
                        self.finished = true;
                    }
                }
                DequeuedOutputBufferInfoResult::TryAgainLater if !to_end => return Ok(()),
                _ => {}
            }
        }
        Ok(())
    }

    fn fail(&self, detail: String) -> Error {
        Error::Ffi {
            operation: "android aac encoder",
            path: Default::default(),
            detail,
        }
    }
}

impl Drop for Aac {
    fn drop(&mut self) {
        let _ = self.codec.stop();
    }
}

/// [`super::mix_to_file_with`] for an AAC file on a phone: the mix through
/// Android's encoder into the file FFmpeg muxes, cut to the mix's length to
/// the sample as the FFmpeg path cuts it.
pub(super) fn mix(
    clips: &[AudioClip],
    buses: &[AudioBus],
    duration: f64,
    destination: &Path,
) -> Result<()> {
    let mut aac = Aac::open(destination)?;
    let mut output = ffmpeg::format::output(destination)
        .map_err(|error| ffi::fail("create", destination, error))?;
    {
        let mut stream = output
            .add_stream(encoder::find(ffmpeg::codec::Id::None))
            .map_err(|error| ffi::fail("add stream", destination, error))?;
        stream.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
        let config = audio_specific_config(MIX_RATE);
        // SAFETY: the stream was just made and owns its parameters; the
        // extradata is allocated by FFmpeg with the padding it requires,
        // and the parameters free it with the stream.
        unsafe {
            let parameters = (*stream.as_mut_ptr()).codecpar;
            (*parameters).codec_type = ffmpeg::media::Type::Audio.into();
            (*parameters).codec_id = ffmpeg::codec::Id::AAC.into();
            (*parameters).sample_rate = MIX_RATE as i32;
            (*parameters).frame_size = FRAME as i32;
            (*parameters).bit_rate = i64::from(BITRATE);
            ffmpeg::sys::av_channel_layout_default(&mut (*parameters).ch_layout, CHANNELS as i32);
            let extradata = ffmpeg::sys::av_mallocz(
                config.len() + ffmpeg::sys::AV_INPUT_BUFFER_PADDING_SIZE as usize,
            ) as *mut u8;
            if extradata.is_null() {
                return Err(Error::Ffi {
                    operation: "extradata",
                    path: destination.to_path_buf(),
                    detail: "out of memory".to_owned(),
                });
            }
            std::ptr::copy_nonoverlapping(config.as_ptr(), extradata, config.len());
            (*parameters).extradata = extradata;
            (*parameters).extradata_size = config.len() as i32;
        }
    }
    output
        .write_header()
        .map_err(|error| ffi::fail("write header", destination, error))?;
    let stream_time_base = output
        .stream(0)
        .map(|stream| stream.time_base())
        .unwrap_or(ffmpeg::Rational::new(1, MIX_RATE as i32));
    let samples = (duration.max(0.0) * f64::from(MIX_RATE)).round() as i64;
    let write = |packets: &mut Vec<Packet>, output: &mut ffmpeg::format::context::Output| {
        for packet in packets.drain(..) {
            // Padding alone past the last sample is dropped, and the frame
            // across it cut there, as the FFmpeg path does.
            if packet.pts >= samples {
                continue;
            }
            let mut written = ffmpeg::Packet::copy(&packet.data);
            written.set_pts(Some(packet.pts));
            written.set_dts(Some(packet.pts));
            written.set_duration((FRAME as i64).min(samples - packet.pts));
            written.set_stream(0);
            written.rescale_ts(ffmpeg::Rational::new(1, MIX_RATE as i32), stream_time_base);
            written
                .write_interleaved(output)
                .map_err(|error| ffi::fail("write packet", destination, error))?;
        }
        Ok::<(), Error>(())
    };

    let mut packets = Vec::new();
    run_mix(
        clips,
        buses,
        (0.0, duration),
        "s16",
        Some(FRAME as u32),
        destination,
        |mixed| {
            let bytes = mixed.samples() * CHANNELS * 2;
            aac.feed(&mixed.data(0)[..bytes], false, &mut packets)?;
            write(&mut packets, &mut output)
        },
    )?;
    // The encoder's delay and a frame more of silence, so the mix's last
    // samples come out of it before the end of the stream.
    let tail = vec![0u8; (DELAY as usize + FRAME) * CHANNELS * 2];
    aac.feed(&tail, false, &mut packets)?;
    aac.feed(&[], true, &mut packets)?;
    write(&mut packets, &mut output)?;
    output
        .write_trailer()
        .map_err(|error| ffi::fail("write trailer", destination, error))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Six seconds of stereo silence with one loud millisecond at exactly
    /// 1.000 s, as a WAV file.
    fn click_wav(path: &Path) {
        let samples = MIX_RATE as usize * 6;
        let mut data = Vec::with_capacity(44 + samples * 4);
        let bytes = (samples * 4) as u32;
        data.extend_from_slice(b"RIFF");
        data.extend_from_slice(&(36 + bytes).to_le_bytes());
        data.extend_from_slice(b"WAVEfmt ");
        data.extend_from_slice(&16u32.to_le_bytes());
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&2u16.to_le_bytes());
        data.extend_from_slice(&MIX_RATE.to_le_bytes());
        data.extend_from_slice(&(MIX_RATE * 4).to_le_bytes());
        data.extend_from_slice(&4u16.to_le_bytes());
        data.extend_from_slice(&16u16.to_le_bytes());
        data.extend_from_slice(b"data");
        data.extend_from_slice(&bytes.to_le_bytes());
        let click = MIX_RATE as usize..MIX_RATE as usize + 48;
        for index in 0..samples {
            let value: i16 = if click.contains(&index) { 20_000 } else { 0 };
            data.extend_from_slice(&value.to_le_bytes());
            data.extend_from_slice(&value.to_le_bytes());
        }
        std::fs::write(path, data).expect("writes the click");
    }

    /// Android's encoder writes the mix the length it is, to the sample,
    /// with its sound where it was: a click at 1.000 s decodes at 1.000 s.
    #[test]
    fn a_phone_mix_keeps_its_length_and_its_timing() {
        let dir = std::env::temp_dir().join(format!("concat-android-aac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let wav = dir.join("click.wav");
        let m4a = dir.join("click.m4a");
        click_wav(&wav);
        let clip = AudioClip {
            path: wav.clone(),
            stream: None,
            start: 0.0,
            duration: 6.0,
            source_start: 0.0,
            speed: 1.0,
            tempo: Vec::new(),
            preserve_pitch: true,
            volume: 1.0,
            volume_curve: Default::default(),
            fade_in: 0.0,
            fade_out: 0.0,
            filter_chain: String::new(),
        };
        super::super::mix_to_file(&[clip], 6.0, &m4a).expect("mixes");

        let info = crate::probe(&m4a).expect("probes");
        let seconds = info.duration.expect("a duration").as_f64();

        let mut reader = crate::samples::AudioDecoder::open(
            &m4a,
            &crate::samples::AudioOptions {
                channels: 1,
                format: crate::samples::SampleFormat::F32,
                ..Default::default()
            },
        )
        .expect("reads it back");
        let samples = reader.collect_f32().expect("decodes");
        let peak = (0..samples.len())
            .max_by(|a, b| samples[*a].abs().total_cmp(&samples[*b].abs()))
            .expect("samples");
        let _ = std::fs::remove_dir_all(&dir);
        eprintln!(
            "peak at sample {peak}, the click at 48000 to 48048; {seconds} s, {} samples decoded",
            samples.len()
        );
        assert!((seconds - 6.0).abs() < 0.001, "{seconds} s");
        assert!(
            (MIX_RATE as usize - 48..MIX_RATE as usize + 96).contains(&peak),
            "the click decodes at sample {peak}, not 48000"
        );
    }
}
