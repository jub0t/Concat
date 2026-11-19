// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Sound made longer or shorter without changing its pitch.
//!
//! A clip at half speed has to last twice as long and still sound in
//! tune. The sound is cut into grains about 10 ms apart, each analysed and
//! laid down again at its new place (Bungee, a phase vocoder, under
//! `third_party/`). What this crate adds is where each grain comes from:
//! output frame `k` is the source frame the clip's time map says it is,
//! grain by grain, so a speed curve or a jump in speed lands every sound
//! where the picture is, with no drift and no lag to correct for. A
//! window-based stretcher that only knows a rate places a drum hit up to a
//! window early; this one is held to a millisecond by its tests.
//!
//! The source is read in order from the clip's in-point. Nothing before
//! the in-point or past the out-point is heard: the grains that reach over
//! either edge hear silence there, so a cut stays a cut.

use std::collections::VecDeque;
use std::ffi::c_void;

/// Where a clip's source is at each output frame: the clip's time map.
pub trait TimeMap {
    /// The source frame, counted from the in-point, that output frame
    /// `output` (counted from the clip's start) plays. Never decreasing.
    fn source_at(&self, output: f64) -> f64;
}

impl<F: Fn(f64) -> f64> TimeMap for F {
    fn source_at(&self, output: f64) -> f64 {
        self(output)
    }
}

/// A constant rate: source frames per output frame.
#[derive(Clone, Copy, Debug)]
pub struct Constant(pub f64);

impl TimeMap for Constant {
    fn source_at(&self, output: f64) -> f64 {
        output * self.0
    }
}

/// Where the source's samples come from: called for more, it appends
/// interleaved frames to the buffer and says whether there were any. The
/// first frame it gives is the clip's in-point.
pub trait Source {
    /// Appends the next frames, interleaved. False at the end of the
    /// source, with nothing appended.
    fn read(&mut self, into: &mut Vec<f32>) -> bool;
}

impl<F: FnMut(&mut Vec<f32>) -> bool> Source for F {
    fn read(&mut self, into: &mut Vec<f32>) -> bool {
        self(into)
    }
}

/// How many output frames apart Bungee lays its grains at `rate`: a
/// sixty-fourth of the rate, rounded down to a power of two, scaled by
/// [`GRAIN_ADJUST`]: 256 at 44.1 and 48 kHz. As `Timing` in Bungee's
/// source works it out.
pub fn hop(rate: u32) -> usize {
    let log2 = 31 - rate.max(64).leading_zeros();
    1 << (log2 as i32 - 6 + GRAIN_ADJUST)
}

/// Bungee's grain size, in powers of two from its default: half, a hop of
/// 256 frames (5.3 ms) at 48 kHz. Measured on pips and a held chord
/// (tests.rs): at the default, a pip slowed to a quarter starts 20 ms late
/// and one at 4x 3.9 ms early; at half, every speed from 0.25x to 2x is
/// within 0.54 ms and 4x within 2.4 ms, while a held chord's energy off
/// its partials stays near -40 dB. A quarter warbles (-20 dB).
const GRAIN_ADJUST: i32 = -1;

/// The first grain asked for, in hops before the clip's start: the grains
/// before frame nought only fill the stretcher's overlap, and are heard as
/// the silence before the in-point.
const PREROLL_GRAINS: i64 = 4;

/// A clip's sound, stretched to its time map, read out frame by frame.
pub struct Stretch<S, M> {
    raw: Raw,
    channels: usize,
    hop: usize,
    map: M,
    source: S,
    /// Source frames past this one are not heard: the out-point.
    limit: Option<u64>,
    source_done: bool,
    /// The source frames read and still needed, planar, from
    /// `window_begin`.
    window: Vec<VecDeque<f32>>,
    window_begin: i64,
    /// Interleaved frames just read, before they are split into `window`.
    staging: Vec<f32>,
    /// One grain's input, planar, contiguous, as Bungee reads it.
    scratch: Vec<f32>,
    next_grain: i64,
    /// Grains asked for whose output has not come back, oldest first.
    pending: VecDeque<(i64, f64)>,
    /// Stretched frames ready, interleaved, from output frame `ready_at`.
    ready: VecDeque<f32>,
    ready_at: i64,
}

// SAFETY: the Bungee instance is owned by this value alone and is only
// called through `&mut self`; it holds no thread-local state.
unsafe impl<S: Send, M: Send> Send for Stretch<S, M> {}

impl<S: Source, M: TimeMap> Stretch<S, M> {
    /// A stretch of `channels` channels at `rate` frames a second, reading
    /// `source` from the in-point, through `map`, and hearing nothing past
    /// source frame `limit` when there is one.
    pub fn new(rate: u32, channels: usize, map: M, source: S, limit: Option<u64>) -> Self {
        assert!((1..=8).contains(&channels), "1 to 8 channels");
        let raw = Raw::new(rate, channels);
        Self {
            raw,
            channels,
            hop: hop(rate),
            map,
            source,
            limit,
            source_done: false,
            window: (0..channels).map(|_| VecDeque::new()).collect(),
            window_begin: 0,
            staging: Vec::new(),
            scratch: Vec::new(),
            next_grain: -PREROLL_GRAINS,
            pending: VecDeque::new(),
            ready: VecDeque::new(),
            ready_at: 0,
        }
    }

    /// Fills `out`, interleaved, with the next frames of the stretched
    /// sound. Past the end of the source it is silence, for as long as it
    /// is read.
    pub fn render(&mut self, out: &mut [f32]) {
        debug_assert_eq!(out.len() % self.channels, 0);
        let wanted = out.len();
        while self.ready.len() < wanted {
            self.grain();
        }
        for (slot, sample) in out.iter_mut().zip(self.ready.drain(..wanted)) {
            *slot = sample;
        }
        self.ready_at += (wanted / self.channels) as i64;
    }

    /// The source position grain `grain` is centred on: the time map at
    /// its output frame, and before the clip's start the map's first rate
    /// carried back.
    fn position(&self, grain: i64) -> (f64, f64) {
        let hop = self.hop as f64;
        let at = grain as f64 * hop;
        let first = self.map.source_at(0.0);
        let rate = |from: f64| (self.map.source_at(from + hop) - self.map.source_at(from)) / hop;
        if at < 0.0 {
            let speed = rate(0.0);
            (first + at * speed, speed)
        } else {
            (self.map.source_at(at), rate(at))
        }
    }

    /// Asks for, feeds and lays down one grain, keeping what it gave.
    fn grain(&mut self) {
        let grain = self.next_grain;
        self.next_grain += 1;
        let (position, speed) = self.position(grain);
        let request = Request {
            position,
            speed: speed.max(f64::MIN_POSITIVE),
            pitch: 1.0,
            reset: grain == -PREROLL_GRAINS,
            resample_mode: 0,
        };
        let chunk = self.raw.specify(&request);
        let (head, tail) = self.gather(chunk);
        let length = (chunk.end - chunk.begin).max(0) as usize;
        self.raw.analyse(&self.scratch, length, head, tail);
        self.pending.push_back((grain, position));
        let output = self.raw.synthesise();
        self.keep(&output);
        self.forget_before(i64::from(chunk.begin));
    }

    /// Puts source frames `chunk` into `scratch`, planar, and says how
    /// many at its head and tail are outside what is heard.
    fn gather(&mut self, chunk: InputChunk) -> (i32, i32) {
        let begin = i64::from(chunk.begin);
        let end = i64::from(chunk.end);
        let length = (end - begin).max(0) as usize;
        self.scratch.clear();
        self.scratch.resize(length * self.channels, 0.0);
        if length == 0 {
            return (0, 0);
        }
        while !self.source_done && self.window_end() < end {
            self.staging.clear();
            if self.source.read(&mut self.staging) {
                for frame in self.staging.chunks_exact(self.channels) {
                    for (channel, sample) in self.window.iter_mut().zip(frame) {
                        channel.push_back(*sample);
                    }
                }
            } else {
                self.source_done = true;
            }
        }
        // What is heard: from the in-point to the out-point or the end of
        // the source, whichever is first.
        let mut heard_end = self.window_end();
        if let Some(limit) = self.limit {
            heard_end = heard_end.min(limit as i64);
        }
        let from = begin.max(0).max(self.window_begin);
        let to = end.min(heard_end);
        if from >= to {
            // Nothing of the source in this grain: all of it is silence.
            return (length as i32, 0);
        }
        for (channel, samples) in self.window.iter().enumerate() {
            let row = &mut self.scratch[channel * length..(channel + 1) * length];
            for frame in from..to {
                row[(frame - begin) as usize] = samples[(frame - self.window_begin) as usize];
            }
        }
        ((from - begin) as i32, (end - to) as i32)
    }

    fn window_end(&self) -> i64 {
        self.window_begin + self.window[0].len() as i64
    }

    /// Lets go of source frames before `frame`, which no later grain asks
    /// for: grains only move on.
    fn forget_before(&mut self, frame: i64) {
        let drop = (frame - self.window_begin).clamp(0, self.window[0].len() as i64) as usize;
        if drop > 0 {
            for channel in &mut self.window {
                channel.drain(..drop);
            }
            self.window_begin += drop as i64;
        }
    }

    /// Takes the frames a grain gave back, if they are of a grain at or
    /// after the clip's start, and appends them, interleaved, to `ready`.
    fn keep(&mut self, output: &Output) {
        let Some(begin) = output.begin else {
            return;
        };
        // The oldest grain still pending whose position this is: Bungee
        // hands back the requests it was given, so the match is exact.
        let Some(index) = self.pending.iter().position(|&(_, at)| at == begin) else {
            debug_assert!(false, "Bungee gave back a grain it was not asked for");
            return;
        };
        let (grain, _) = self.pending[index];
        self.pending.drain(..=index);
        let start = grain * self.hop as i64;
        debug_assert_eq!(output.frames, self.hop, "a grain's output is one hop");
        let produced_end = self.ready_at + (self.ready.len() / self.channels) as i64;
        for frame in 0..output.frames {
            let at = start + frame as i64;
            if at < produced_end {
                continue;
            }
            for channel in 0..self.channels {
                // SAFETY: Bungee's output chunk holds `frames` frames of
                // each channel, `stride` apart, until the next call.
                let sample = unsafe {
                    *output
                        .data
                        .offset(channel as isize * output.stride + frame as isize)
                };
                self.ready.push_back(sample);
            }
        }
    }
}

/// What one grain gave back.
struct Output {
    /// The source position of the grain it starts at, or none while the
    /// stretcher is still filling.
    begin: Option<f64>,
    data: *const f32,
    frames: usize,
    stride: isize,
}

/// One Bungee stretcher, called only in the order it demands: specify,
/// analyse, synthesise. Out of order it aborts the process, so the order
/// is this type's to keep.
struct Raw {
    functions: &'static Functions,
    state: *mut c_void,
}

impl Raw {
    fn new(rate: u32, channels: usize) -> Self {
        // SAFETY: the table is a static in the library, valid for the life
        // of the process.
        let functions = unsafe { &*getFunctionsBungeeBasic() };
        let rates = SampleRates {
            input: rate as i32,
            output: rate as i32,
        };
        // SAFETY: a plain constructor; it returns an owned instance.
        let state = unsafe { (functions.create)(rates, channels as i32, GRAIN_ADJUST) };
        assert!(!state.is_null(), "Bungee made no stretcher");
        Self { functions, state }
    }

    fn specify(&mut self, request: &Request) -> InputChunk {
        // SAFETY: `state` is live; the request is read during the call.
        unsafe { (self.functions.specify_grain)(self.state, request, 0.0) }
    }

    /// `scratch` holds `length` frames of each channel, planar.
    fn analyse(&mut self, scratch: &[f32], length: usize, head: i32, tail: i32) {
        // SAFETY: the chunk's frames are all in `scratch`, `length` apart
        // per channel; muted frames are zeros Bungee does not read.
        unsafe {
            (self.functions.analyse_grain)(
                self.state,
                scratch.as_ptr(),
                length as isize,
                head,
                tail,
            );
        }
    }

    fn synthesise(&mut self) -> Output {
        let mut chunk = OutputChunk {
            data: std::ptr::null_mut(),
            frame_count: 0,
            channel_stride: 0,
            request: [std::ptr::null(); 2],
        };
        // SAFETY: follows a specify and an analyse of the same grain.
        unsafe { (self.functions.synthesise_grain)(self.state, &mut chunk) };
        // SAFETY: when set, the request pointers point at the stretcher's
        // own copies of earlier requests.
        let begin = unsafe { chunk.request[0].as_ref() }
            .map(|request| request.position)
            .filter(|position| !position.is_nan());
        Output {
            begin,
            data: chunk.data,
            frames: chunk.frame_count.max(0) as usize,
            stride: chunk.channel_stride,
        }
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        // SAFETY: made by `create`, destroyed once.
        unsafe { (self.functions.destroy)(self.state) }
    }
}

#[repr(C)]
struct Request {
    position: f64,
    speed: f64,
    pitch: f64,
    reset: bool,
    resample_mode: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct InputChunk {
    begin: i32,
    end: i32,
}

#[repr(C)]
struct OutputChunk {
    data: *mut f32,
    frame_count: i32,
    channel_stride: isize,
    request: [*const Request; 2],
}

#[repr(C)]
struct SampleRates {
    input: i32,
    output: i32,
}

/// Bungee's C function table, as `bungee/Bungee.h` declares it.
#[repr(C)]
struct Functions {
    edition: unsafe extern "C" fn() -> *const std::ffi::c_char,
    version: unsafe extern "C" fn() -> *const std::ffi::c_char,
    create: unsafe extern "C" fn(SampleRates, i32, i32) -> *mut c_void,
    destroy: unsafe extern "C" fn(*mut c_void),
    enable_instrumentation: unsafe extern "C" fn(*mut c_void, i32),
    max_input_frame_count: unsafe extern "C" fn(*const c_void) -> i32,
    preroll: unsafe extern "C" fn(*const c_void, *mut Request),
    next: unsafe extern "C" fn(*const c_void, *mut Request),
    specify_grain: unsafe extern "C" fn(*mut c_void, *const Request, f64) -> InputChunk,
    analyse_grain: unsafe extern "C" fn(*mut c_void, *const f32, isize, i32, i32),
    synthesise_grain: unsafe extern "C" fn(*mut c_void, *mut OutputChunk),
    is_flushed: unsafe extern "C" fn(*const c_void) -> bool,
}

unsafe extern "C" {
    fn getFunctionsBungeeBasic() -> *const Functions;
}

#[cfg(test)]
mod tests;
