// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Keeping a codec fed: the send-packet, receive-frame loop every decoder
//! in this crate runs, written once.
//!
//! libavcodec's contract is that `avcodec_send_packet` may refuse a packet
//! with `EAGAIN` when the codec has frames to hand back first, and that the
//! refused packet is to be sent again once they have been taken. FFmpeg's
//! own software decoders almost never say it, so the loops here used to
//! read past the refusal and move on to the next packet - dropping the
//! refused one. A codec that wraps the platform's decoder says it all the
//! time: MediaCodec's input queue holds a handful of packets, and every
//! packet dropped when it was full was a hole in the stream, a smear of
//! grey blocks until the next keyframe. [`next_frame`] keeps the packet
//! and sends it again after draining what the codec had.

use ffmpeg_the_third as ffmpeg;
use ffmpeg_the_third::codec::decoder;
use ffmpeg_the_third::util::frame::{audio::Audio, video::Video};

use std::path::Path;

use crate::error::Error;
use crate::ffi;

/// What goes into a codec: a packet, or the end of the stream.
#[derive(Debug)]
pub(crate) enum Input<P> {
    Packet(P),
    End,
}

/// A packet the codec refused with `EAGAIN`, kept to be sent again; or
/// nothing. Cleared whenever the codec is flushed or replaced, since the
/// packet belonged to the stream position before.
pub(crate) type Held<P> = Option<Input<P>>;

/// The two halves of a codec's API, as [`next_frame`] drives them.
pub(crate) trait Codec {
    type Packet;
    type Frame;
    /// Hands the codec a packet, or tells it the stream is over.
    fn send(&mut self, input: &Input<Self::Packet>) -> Result<(), ffmpeg::Error>;
    /// Takes a decoded frame out of the codec.
    fn receive(&mut self) -> Result<Self::Frame, ffmpeg::Error>;
}

/// Why [`next_frame`] stopped without a frame or the end.
#[derive(Debug)]
pub(crate) enum Stall<E> {
    /// The codec refused to hand back a frame.
    Receive(ffmpeg::Error),
    /// The codec refused a packet, for a reason other than being full.
    Send(ffmpeg::Error),
    /// Reading the next packet failed.
    Read(E),
    /// The codec refused every packet and gave nothing back, round after
    /// round. libavcodec promises that never happens; a broken platform
    /// decoder is the one way it could, and a stuck reader is worse than an
    /// error the caller can fall back from.
    Stuck,
}

/// Rounds in a row with the codec neither taking a packet nor giving a
/// frame before it counts as stuck. MediaCodec's receive waits for its
/// output on each round, so this is seconds, not a busy spin.
const STUCK_AFTER: u32 = 1000;

/// Damaged packets or pictures in a row the codec may refuse before the
/// stream counts as unreadable rather than damaged in places.
const DAMAGED_AFTER: u32 = 64;

/// The next frame out of `codec`, or `None` once it has given its last.
///
/// Feeds it from `held` first, then from `read`, and when the codec says
/// `EAGAIN` to a packet, keeps that packet in `held` and takes frames out
/// before trying it again. `read` gives [`Input::End`] at the end of the
/// file, as often as it is asked.
///
/// A packet the codec calls damaged (`AVERROR_INVALIDDATA`) is passed over,
/// as every player passes over it, and so is a picture it could not
/// rebuild: the frames around it still decode. One used to end the read,
/// and with it the export; and the first audio packet after a seek into an
/// MPEG-PS file is always one, cut in half by the seek, so every export of
/// a .mpg clip with an in-point failed (#457). Only [`DAMAGED_AFTER`] in a
/// row is a stall, a stream with nothing readable left in it.
pub(crate) fn next_frame<C: Codec, E>(
    codec: &mut C,
    held: &mut Held<C::Packet>,
    mut read: impl FnMut() -> Result<Input<C::Packet>, E>,
) -> Result<Option<C::Frame>, Stall<E>> {
    let mut refused = 0;
    let mut damaged = 0;
    loop {
        match codec.receive() {
            Ok(frame) => return Ok(Some(frame)),
            Err(ffmpeg::Error::Eof) => return Ok(None),
            Err(error) if ffi::is_again(&error) => {}
            Err(ffmpeg::Error::InvalidData) if damaged + 1 < DAMAGED_AFTER => {
                damaged += 1;
                continue;
            }
            Err(error) => return Err(Stall::Receive(error)),
        }
        let input = match held.take() {
            Some(input) => input,
            None => read().map_err(Stall::Read)?,
        };
        match codec.send(&input) {
            Ok(()) => {
                refused = 0;
                damaged = 0;
            }
            // Told twice the stream is over: it heard the first time.
            Err(ffmpeg::Error::Eof) if matches!(input, Input::End) => {}
            Err(ffmpeg::Error::InvalidData) if damaged + 1 < DAMAGED_AFTER => damaged += 1,
            Err(error) if ffi::is_again(&error) => {
                *held = Some(input);
                refused += 1;
                if refused >= STUCK_AFTER {
                    return Err(Stall::Stuck);
                }
            }
            Err(error) => return Err(Stall::Send(error)),
        }
    }
}

/// The error a stall is for a reader of `path`, named the way the loops
/// this replaced named theirs.
pub(crate) fn fail(stall: Stall<ffmpeg::Error>, path: &Path) -> Error {
    match stall {
        Stall::Receive(error) => ffi::fail("decode", path, error),
        Stall::Send(error) => ffi::fail("send packet", path, error),
        Stall::Read(error) => ffi::fail("read", path, error),
        Stall::Stuck => Error::Ffi {
            operation: "decode",
            path: path.to_path_buf(),
            detail: "the codec stopped taking packets and giving frames".to_owned(),
        },
    }
}

/// The next packet of `stream` in `input`, or [`Input::End`] at the end of
/// the file. Packets of every other stream are skipped.
pub(crate) fn read_packet(
    input: &mut ffmpeg::format::context::Input,
    stream: usize,
) -> Result<Input<ffmpeg::Packet>, ffmpeg::Error> {
    loop {
        let mut packet = ffmpeg::Packet::empty();
        match packet.read(input) {
            Ok(()) if packet.stream() == stream => return Ok(Input::Packet(packet)),
            Ok(()) => {}
            Err(ffmpeg::Error::Eof) => return Ok(Input::End),
            Err(error) => return Err(error),
        }
    }
}

fn send(codec: &mut decoder::Opened, input: &Input<ffmpeg::Packet>) -> Result<(), ffmpeg::Error> {
    match input {
        Input::Packet(packet) => codec.send_packet(packet),
        Input::End => codec.send_eof(),
    }
}

impl Codec for decoder::Video {
    type Packet = ffmpeg::Packet;
    type Frame = Video;

    fn send(&mut self, input: &Input<ffmpeg::Packet>) -> Result<(), ffmpeg::Error> {
        send(self, input)
    }

    fn receive(&mut self) -> Result<Video, ffmpeg::Error> {
        let mut frame = Video::empty();
        self.receive_frame(&mut frame).map(|()| frame)
    }
}

impl Codec for decoder::Audio {
    type Packet = ffmpeg::Packet;
    type Frame = Audio;

    fn send(&mut self, input: &Input<ffmpeg::Packet>) -> Result<(), ffmpeg::Error> {
        send(self, input)
    }

    fn receive(&mut self) -> Result<Audio, ffmpeg::Error> {
        let mut frame = Audio::empty();
        self.receive_frame(&mut frame).map(|()| frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    fn again() -> ffmpeg::Error {
        ffmpeg::Error::Other {
            errno: libc::EAGAIN,
        }
    }

    /// A codec shaped like MediaCodec: an input queue of `room` packets,
    /// one frame out per packet in, refusing packets while the queue is
    /// full, and giving a frame back only once it has refused one (or the
    /// stream is over) - the worst case for a loop that drops refusals.
    struct Queue {
        room: usize,
        queue: VecDeque<u32>,
        blocked: bool,
        ended: bool,
        refusals: usize,
    }

    impl Queue {
        fn new(room: usize) -> Self {
            Self {
                room,
                queue: VecDeque::new(),
                blocked: false,
                ended: false,
                refusals: 0,
            }
        }
    }

    impl Codec for Queue {
        type Packet = u32;
        type Frame = u32;

        fn send(&mut self, input: &Input<u32>) -> Result<(), ffmpeg::Error> {
            match input {
                _ if self.queue.len() >= self.room => {
                    self.blocked = true;
                    self.refusals += 1;
                    Err(again())
                }
                Input::Packet(_) if self.ended => Err(ffmpeg::Error::Eof),
                Input::Packet(packet) => {
                    self.queue.push_back(*packet);
                    Ok(())
                }
                Input::End if self.ended => Err(ffmpeg::Error::Eof),
                Input::End => {
                    self.ended = true;
                    Ok(())
                }
            }
        }

        fn receive(&mut self) -> Result<u32, ffmpeg::Error> {
            if !self.blocked && !self.ended {
                return Err(again());
            }
            match self.queue.pop_front() {
                Some(frame) => {
                    self.blocked = false;
                    Ok(frame)
                }
                None if self.ended => Err(ffmpeg::Error::Eof),
                None => Err(again()),
            }
        }
    }

    /// A codec that, like a full MediaCodec queue, refuses a packet once
    /// and hands back a frame first. Every packet must still come out.
    struct Refuses {
        sent: Vec<u32>,
        ready: VecDeque<u32>,
        refuse_next: bool,
        ended: bool,
    }

    impl Codec for Refuses {
        type Packet = u32;
        type Frame = u32;

        fn send(&mut self, input: &Input<u32>) -> Result<(), ffmpeg::Error> {
            if self.refuse_next {
                self.refuse_next = false;
                return Err(again());
            }
            match input {
                Input::Packet(packet) => {
                    self.sent.push(*packet);
                    self.ready.push_back(*packet);
                    // Every other packet is refused the next time round.
                    self.refuse_next = packet % 2 == 0;
                }
                Input::End => self.ended = true,
            }
            Ok(())
        }

        fn receive(&mut self) -> Result<u32, ffmpeg::Error> {
            match self.ready.pop_front() {
                Some(frame) => Ok(frame),
                None if self.ended => Err(ffmpeg::Error::Eof),
                None => Err(again()),
            }
        }
    }

    fn drain<C: Codec<Packet = u32, Frame = u32>>(codec: &mut C, packets: u32) -> Vec<u32> {
        let mut source = 0..packets;
        let mut held = None;
        let mut out = Vec::new();
        loop {
            let read = || -> Result<Input<u32>, ()> {
                Ok(source.next().map_or(Input::End, Input::Packet))
            };
            match next_frame(codec, &mut held, read) {
                Ok(Some(frame)) => out.push(frame),
                Ok(None) => return out,
                Err(stall) => panic!("stalled: {stall:?}"),
            }
        }
    }

    #[test]
    fn a_refused_packet_is_sent_again_not_dropped() {
        let mut codec = Refuses {
            sent: Vec::new(),
            ready: VecDeque::new(),
            refuse_next: false,
            ended: false,
        };
        let out = drain(&mut codec, 10);
        assert_eq!(out, (0..10).collect::<Vec<_>>());
        assert_eq!(codec.sent, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn a_full_input_queue_loses_nothing_and_drains_at_the_end() {
        for room in [1, 2, 4, 8] {
            let mut codec = Queue::new(room);
            let out = drain(&mut codec, 30);
            assert_eq!(out, (0..30).collect::<Vec<_>>(), "room {room}");
            assert!(codec.refusals > 0, "room {room} was never full");
        }
    }

    #[test]
    fn a_codec_that_takes_nothing_and_gives_nothing_is_stuck_not_spinning() {
        struct Wedged;
        impl Codec for Wedged {
            type Packet = u32;
            type Frame = u32;
            fn send(&mut self, _: &Input<u32>) -> Result<(), ffmpeg::Error> {
                Err(again())
            }
            fn receive(&mut self) -> Result<u32, ffmpeg::Error> {
                Err(again())
            }
        }
        let mut held = None;
        let mut reads = 0;
        let result = next_frame(&mut Wedged, &mut held, || -> Result<Input<u32>, ()> {
            reads += 1;
            Ok(Input::Packet(7))
        });
        assert!(matches!(result, Err(Stall::Stuck)));
        // The one packet it kept refusing was read once and is still held.
        assert_eq!(reads, 1);
        assert!(matches!(held, Some(Input::Packet(7))));
    }

    /// A damaged packet is passed over and the stream read on, as a
    /// player reads on; a stream of nothing but damage is still a stall.
    #[test]
    fn a_damaged_packet_is_passed_over() {
        struct Scarred {
            sent: Vec<u32>,
        }
        impl Codec for Scarred {
            type Packet = u32;
            type Frame = u32;
            fn send(&mut self, input: &Input<u32>) -> Result<(), ffmpeg::Error> {
                match input {
                    Input::Packet(2 | 3) => Err(ffmpeg::Error::InvalidData),
                    Input::Packet(packet) => {
                        self.sent.push(*packet);
                        Ok(())
                    }
                    Input::End => Ok(()),
                }
            }
            fn receive(&mut self) -> Result<u32, ffmpeg::Error> {
                match self.sent.pop() {
                    Some(5) => Err(ffmpeg::Error::InvalidData),
                    Some(frame) => Ok(frame),
                    None => Err(again()),
                }
            }
        }
        let mut codec = Scarred { sent: Vec::new() };
        let mut held = None;
        let mut next = 0;
        let mut frames = Vec::new();
        while next < 7 {
            let read = || -> Result<Input<u32>, ()> {
                next += 1;
                Ok(Input::Packet(next))
            };
            match next_frame(&mut codec, &mut held, read) {
                Ok(Some(frame)) => frames.push(frame),
                other => panic!("stopped at packet {next}: {other:?}"),
            }
        }
        assert_eq!(frames, [1, 4, 6, 7]);
    }

    #[test]
    fn errors_say_which_side_failed() {
        struct Broken;
        impl Codec for Broken {
            type Packet = u32;
            type Frame = u32;
            fn send(&mut self, _: &Input<u32>) -> Result<(), ffmpeg::Error> {
                Err(ffmpeg::Error::InvalidData)
            }
            fn receive(&mut self) -> Result<u32, ffmpeg::Error> {
                Err(again())
            }
        }
        let mut held = None;
        let sent = next_frame(&mut Broken, &mut held, || -> Result<Input<u32>, ()> {
            Ok(Input::Packet(1))
        });
        assert!(matches!(sent, Err(Stall::Send(ffmpeg::Error::InvalidData))));
        let read = next_frame(&mut Broken, &mut held, || -> Result<Input<u32>, &str> {
            Err("disk")
        });
        assert!(matches!(read, Err(Stall::Read("disk"))));
    }
}
