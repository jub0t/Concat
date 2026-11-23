// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! What an AAC encoder does with a sound's first samples, measured.
//!
//! AAC codes overlapping blocks, so a decoder gives a frame back whole only
//! when it has the packet before it. An encoder therefore starts a stream
//! with a packet of priming, and says how much of the decoded sound is that
//! priming, for the file to tell a player to skip. FFmpeg's encoder and
//! Apple's both do. Windows' does neither: it throws its priming packet
//! away and stamps the next one nought, so the first frame of a sound comes
//! back faded in from silence - a hit on a film's first frame lost half its
//! length - and Microsoft calls it by design. How many packets it throws
//! away has changed between Windows versions, and may again.
//!
//! So an encoder that does not declare its priming is not trusted to a
//! rule: once a run it is handed a sound it cannot mistake, and what comes
//! back says how much silence to feed it ahead of a mix and how far to move
//! its packets' stamps, so the silence is the priming and the mix's first
//! sample is the first one heard.

use std::sync::OnceLock;

use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::encoder;
use ffmpeg_the_third::util::channel_layout::{ChannelLayout, ChannelLayoutMask};
use ffmpeg_the_third::util::format::Sample;
use ffmpeg_the_third::util::format::sample::Type;
use ffmpeg_the_third::util::frame::audio::Audio;

use super::MIX_RATE;
use crate::ffi;

/// How an encoder is fed so a sound's first sample is its first heard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Lead {
    /// Samples of silence fed ahead of the sound: whole frames, as many as
    /// the encoder cannot give back whole.
    pub silence: i64,
    /// Samples taken off every packet's stamp, so the sound's first sample
    /// is stamped nought and what is ahead of it is priming.
    pub shift: i64,
}

/// How many times a flushed encoder that says "nothing yet" is asked
/// again, a millisecond apart, before it is taken to have finished without
/// saying so. FFmpeg's and Apple's answer with their packets or their end
/// of stream at once; an encoder behind Media Foundation may still be
/// working.
pub(crate) const FLUSH_WAITS: u32 = 500;

/// The frame an encoder that takes any size is fed: an AAC frame.
pub(crate) const FRAME: u32 = 1024;

/// The frame size to feed an encoder that states `stated`: its own, or
/// [`FRAME`] where it states none because it takes any.
pub(crate) fn frame_size(stated: u32) -> u32 {
    if stated == 0 { FRAME } else { stated }
}

/// The frames of the measuring sound.
const FRAMES: usize = 8;

/// A frame is given back whole when what differs from what went in carries
/// under this share of its energy. A frame the decoder had no packet before
/// loses about half; one coded at the mix's bit rate, a thousandth or so.
const WHOLE: f64 = 0.1;

/// The measurement is believed when the sound that came back is this like
/// the one that went in, of 1, at the lag it is most like it.
const ALIKE: f64 = 0.8;

/// The lead `codec_name` needs, measured the first time it is asked for.
/// An encoder that declares its priming needs none, and is not measured.
pub(crate) fn of(codec_name: &str, format: Sample) -> Lead {
    static WINDOWS: OnceLock<Lead> = OnceLock::new();
    #[cfg(test)]
    if let Some(forced) = FORCED.get() {
        return forced;
    }
    if codec_name != "aac_mf" {
        return Lead::default();
    }
    *WINDOWS.get_or_init(|| match measure(codec_name, format) {
        Ok(lead) => {
            log::info!(
                "{codec_name}: {} samples of silence ahead of a mix, its packets stamped {} earlier",
                lead.silence,
                lead.shift
            );
            lead
        }
        Err(why) => {
            log::warn!("{codec_name}: its start could not be measured ({why}); fed as it is");
            Lead::default()
        }
    })
}

#[cfg(test)]
thread_local! {
    /// A lead a test has the mix on its thread use, whatever the encoder:
    /// the feeding and restamping run where Windows' encoder is not.
    pub(crate) static FORCED: std::cell::Cell<Option<Lead>> = const { std::cell::Cell::new(None) };
}

/// A stereo frame of `samples`, the same in both channels, in `format`:
/// packed 16-bit or planar float, the two an AAC encoder here takes.
pub(crate) fn frame(format: Sample, samples: &[f32]) -> Option<Audio> {
    let mut frame = Audio::new(format, samples.len(), ChannelLayoutMask::STEREO);
    frame.set_ch_layout(ChannelLayout::STEREO);
    frame.set_rate(MIX_RATE);
    match format {
        Sample::I16(Type::Packed) => {
            let bytes = frame.data_mut(0);
            for (pair, sample) in bytes.chunks_exact_mut(4).zip(samples) {
                let value = ((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes();
                pair[..2].copy_from_slice(&value);
                pair[2..].copy_from_slice(&value);
            }
        }
        Sample::F32(Type::Planar) => {
            for plane in 0..frame.planes() {
                frame.plane_mut::<f32>(plane).copy_from_slice(samples);
            }
        }
        _ => return None,
    }
    Some(frame)
}

/// The measuring sound: a sweep from 300 Hz to 6 kHz at half scale, from
/// its first sample to its last. It is like itself at no lag but nought,
/// and all tone, which a codec keeps the shape of where it would only keep
/// the loudness of noise.
fn sweep(length: usize) -> Vec<f32> {
    let (low, high) = (300.0f64, 6000.0f64);
    let rate = f64::from(MIX_RATE);
    (0..length)
        .map(|n| {
            let n = n as f64;
            let turns = (low * n + (high - low) * n * n / (2.0 * length as f64)) / rate;
            (0.5 * (2.0 * std::f64::consts::PI * turns).sin()) as f32
        })
        .collect()
}

/// Runs the sweep through `codec_name` as the mix sets it up, decodes what
/// comes out and judges it.
fn measure(codec_name: &str, format: Sample) -> Result<Lead, String> {
    ffi::init();
    let codec = encoder::find_by_name(codec_name).ok_or("no such encoder")?;
    let mut audio = ffmpeg::codec::Context::new_with_codec(codec)
        .encoder()
        .audio()
        .map_err(|error| format!("encoder: {error}"))?;
    audio.set_rate(MIX_RATE as i32);
    audio.set_format(format);
    audio.set_ch_layout(ChannelLayout::STEREO);
    audio.set_bit_rate(192_000);
    audio.set_time_base(ffmpeg::Rational::new(1, MIX_RATE as i32));
    // As in an MP4, where the mix goes: the decoder's setup in the
    // parameters, not in every packet.
    audio.set_flags(ffmpeg::codec::Flags::GLOBAL_HEADER);
    ffi::begin();
    let mut encoder = audio.open().map_err(|error| format!("open: {error}"))?;
    let size = frame_size(encoder.frame_size()) as usize;
    let source = sweep(FRAMES * size);

    let mut packets = Vec::new();
    let mut drain = |encoder: &mut encoder::audio::Encoder, flushing: bool| -> Result<(), String> {
        let mut waits = 0;
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match encoder.receive_packet(&mut packet) {
                Ok(()) => packets.push(packet),
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(error) if ffi::is_again(&error) => {
                    if !flushing || waits >= FLUSH_WAITS {
                        return Ok(());
                    }
                    waits += 1;
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                Err(error) => return Err(format!("encode: {error}")),
            }
        }
    };
    for (index, samples) in source.chunks(size).enumerate() {
        let mut frame = frame(format, samples).ok_or("a sample format the measure cannot write")?;
        frame.set_pts(Some((index * size) as i64));
        encoder
            .send_frame(&frame)
            .map_err(|error| format!("encode: {error}"))?;
        drain(&mut encoder, false)?;
    }
    encoder
        .send_eof()
        .map_err(|error| format!("flush: {error}"))?;
    drain(&mut encoder, true)?;

    // FFmpeg's own decoder, as a file's reader on Windows uses: what it
    // cannot give back whole is what the lead is for.
    // Told of no priming, so it gives back every frame under its packet's
    // own stamp: what an encoder declares, the decoder would cut off
    // itself, and stamp what is left as if it had not.
    let mut parameters = ffmpeg::codec::Parameters::from(&encoder);
    // SAFETY: the parameters are owned here and not yet shared; a plain
    // integer field.
    unsafe {
        (*parameters.as_mut_ptr()).initial_padding = 0;
    }
    let mut decoder = ffmpeg::codec::Context::from_parameters(parameters)
        .and_then(|context| context.decoder().audio())
        .map_err(|error| format!("decoder: {error}"))?;
    let mut heard: Vec<(i64, Vec<f32>)> = Vec::new();
    let mut take = |decoder: &mut ffmpeg::codec::decoder::Audio| -> Result<(), String> {
        loop {
            let mut decoded = Audio::empty();
            match decoder.receive_frame(&mut decoded) {
                Ok(()) => {
                    if decoded.format() != Sample::F32(Type::Planar) {
                        return Err(format!("decoded as {:?}", decoded.format()));
                    }
                    let at = decoded.pts().ok_or("a decoded frame with no stamp")?;
                    heard.push((at, decoded.plane::<f32>(0).to_vec()));
                }
                Err(ffmpeg::Error::Eof) => return Ok(()),
                Err(error) if ffi::is_again(&error) => return Ok(()),
                Err(error) => return Err(format!("decode: {error}")),
            }
        }
    };
    for packet in &packets {
        decoder
            .send_packet(packet)
            .map_err(|error| format!("decode: {error}"))?;
        take(&mut decoder)?;
    }
    decoder
        .send_eof()
        .map_err(|error| format!("decode: {error}"))?;
    take(&mut decoder)?;

    // One run of samples by stamp: `decoded[n]` is stamped `origin + n`.
    let origin = heard
        .iter()
        .map(|(at, _)| *at)
        .min()
        .ok_or("nothing came out")?;
    let end = heard
        .iter()
        .map(|(at, samples)| at + samples.len() as i64)
        .max()
        .unwrap_or(origin);
    let mut decoded = vec![0.0f32; (end - origin).max(0) as usize];
    for (at, samples) in &heard {
        let from = (at - origin) as usize;
        decoded[from..from + samples.len()].copy_from_slice(samples);
    }
    judge(&source, &decoded, origin, size).ok_or_else(|| {
        // Every packet as stamp, length and bytes, and every decoded frame
        // as stamp and length: what a report from a Windows this was never
        // run on needs to say what its encoder does.
        let came: Vec<String> = packets
            .iter()
            .map(|packet| {
                format!(
                    "{}+{}/{}b",
                    packet.pts().unwrap_or(-1),
                    packet.duration(),
                    packet.size()
                )
            })
            .collect();
        let back: Vec<String> = heard
            .iter()
            .map(|(at, samples)| format!("{at}+{}", samples.len()))
            .collect();
        format!(
            "{} samples fed in frames of {size}; packets {}; decoded {}",
            source.len(),
            came.join(" "),
            back.join(" ")
        )
    })
}

/// The lead that makes `decoded` - what came back, its first sample stamped
/// `origin` - start with `source`'s first sample, when `source` was fed
/// stamped from nought in frames of `size`. None when what came back is not
/// the source at any lag within four frames, or no frame of it is whole.
fn judge(source: &[f32], decoded: &[f32], origin: i64, size: usize) -> Option<Lead> {
    let frames = source.len() / size;
    if size == 0 || frames < 6 {
        return None;
    }
    let at = |stamp: i64| -> f64 {
        usize::try_from(stamp - origin)
            .ok()
            .and_then(|index| decoded.get(index))
            .map_or(0.0, |sample| f64::from(*sample))
    };
    // How late the source is heard, by its middle: clear of a start the
    // encoder may have mangled and of the end's padding.
    let middle = 3 * size..(frames - 1) * size;
    let reach = 4 * size as i64;
    let alike = |lag: i64| -> f64 {
        middle
            .clone()
            .map(|n| f64::from(source[n]) * at(n as i64 + lag))
            .sum()
    };
    let (lag, most) =
        (-reach..=reach)
            .map(|lag| (lag, alike(lag)))
            .fold(
                (0, f64::MIN),
                |best, next| if next.1 > best.1 { next } else { best },
            );
    let fed: f64 = middle.clone().map(|n| f64::from(source[n]).powi(2)).sum();
    let back: f64 = middle.clone().map(|n| at(n as i64 + lag).powi(2)).sum();
    if fed <= 0.0 || back <= 0.0 || most / (fed * back).sqrt() < ALIKE {
        return None;
    }
    // The first frame from which every frame came back whole. The last is
    // left out: its end runs into the encoder's padding.
    let whole = |frame: usize| -> bool {
        let span = frame * size..(frame + 1) * size;
        let energy: f64 = span.clone().map(|n| f64::from(source[n]).powi(2)).sum();
        let off: f64 = span
            .map(|n| (at(n as i64 + lag) - f64::from(source[n])).powi(2))
            .sum();
        off < WHOLE * energy
    };
    let first = (0..frames - 1)
        .rev()
        .take_while(|frame| whole(*frame))
        .last()?;
    let silence = (first * size) as i64;
    // Fed after the silence, the sound's sample `n` is stamped
    // `silence + n + lag`: that much comes off for it to be stamped `n`.
    let shift = silence + lag;
    (shift >= 0).then_some(Lead { silence, shift })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: usize = 1024;

    fn source() -> Vec<f32> {
        sweep(FRAMES * SIZE)
    }

    /// A first frame as a decoder gives it with no packet before it: silent,
    /// then rising into the sound.
    fn faded(samples: &mut [f32]) {
        for (index, sample) in samples.iter_mut().enumerate() {
            *sample *= (index as f32 / SIZE as f32).powi(2);
        }
    }

    #[test]
    fn a_sound_given_back_as_it_went_in_needs_no_lead() {
        let source = source();
        assert_eq!(judge(&source, &source, 0, SIZE), Some(Lead::default()));
    }

    /// FFmpeg's encoder: a packet of priming stamped a frame before nought.
    #[test]
    fn priming_stamped_before_nought_needs_no_lead() {
        let source = source();
        let mut decoded = vec![0.0; SIZE];
        decoded.extend_from_slice(&source);
        assert_eq!(
            judge(&source, &decoded, -(SIZE as i64), SIZE),
            Some(Lead::default())
        );
    }

    /// Windows' encoder as CI's runner has it: the priming packet thrown
    /// away, so the first frame fades in, on time.
    #[test]
    fn a_first_frame_that_fades_in_is_fed_a_frame_of_silence() {
        let source = source();
        let mut decoded = source.clone();
        faded(&mut decoded[..SIZE]);
        assert_eq!(
            judge(&source, &decoded, 0, SIZE),
            Some(Lead {
                silence: 1024,
                shift: 1024
            })
        );
    }

    /// Two packets thrown away and the third stamped nought: a frame of the
    /// sound gone, the next faded in, and all of it a frame early.
    #[test]
    fn two_packets_thrown_away_are_two_frames_of_silence_and_one_of_shift() {
        let source = source();
        let mut decoded = source[SIZE..].to_vec();
        faded(&mut decoded[..SIZE]);
        assert_eq!(
            judge(&source, &decoded, 0, SIZE),
            Some(Lead {
                silence: 2048,
                shift: 1024
            })
        );
    }

    /// An encoder that primes and does not say so: whole, and late by its
    /// priming.
    #[test]
    fn priming_that_is_not_declared_is_taken_off_the_stamps() {
        let source = source();
        let mut decoded = vec![0.0; 2112];
        decoded.extend_from_slice(&source);
        assert_eq!(
            judge(&source, &decoded, 0, SIZE),
            Some(Lead {
                silence: 0,
                shift: 2112
            })
        );
    }

    #[test]
    fn a_sound_that_is_not_the_one_fed_is_not_judged() {
        let source = source();
        let other: Vec<f32> = (0..source.len())
            .map(|n| if (n / 37) % 2 == 0 { 0.4 } else { -0.4 })
            .collect();
        assert_eq!(judge(&source, &other, 0, SIZE), None);
        assert_eq!(judge(&source, &vec![0.0; source.len()], 0, SIZE), None);
        assert_eq!(judge(&source, &[], 0, SIZE), None);
    }

    /// The measure itself, on the encoders this machine has. Each declares
    /// its priming, so each comes back whole and on time: what the measure
    /// must say of an encoder with nothing to correct.
    #[test]
    fn an_encoder_that_declares_its_priming_measures_as_needing_no_lead() {
        ffi::init();
        let mut measured = 0;
        for (name, format) in [
            ("aac", Sample::F32(Type::Planar)),
            ("aac_at", Sample::I16(Type::Packed)),
        ] {
            if encoder::find_by_name(name).is_none() {
                continue;
            }
            assert_eq!(measure(name, format), Ok(Lead::default()), "{name}");
            measured += 1;
        }
        assert!(measured > 0, "no AAC encoder here to measure");
    }

    /// Windows' own encoder, where there is one: whatever it does with a
    /// sound's start, the measure finds a lead for it.
    #[test]
    fn windows_encoder_is_measured() {
        ffi::init();
        if encoder::find_by_name("aac_mf").is_none() {
            return;
        }
        let lead = measure("aac_mf", Sample::I16(Type::Packed)).expect("measures");
        eprintln!("aac_mf: {lead:?}");
        assert!(lead.shift > 0, "{lead:?}");
        assert_eq!(lead.silence % 1024, 0, "{lead:?}");
    }
}
