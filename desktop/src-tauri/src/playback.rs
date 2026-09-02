// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Engine-owned audio playback.
//!
//! This replaces the webview audio preview (see the superseded
//! docs/decisions/0004): the engine decodes, mixes and clocks all audible
//! clips itself, and the UI is a controller that follows.
//!
//! The shape of it:
//!
//! - Every audible clip's span is decoded **once** by FFmpeg straight to raw
//!   PCM - the clip's filter chain and speed baked in with the same filters
//!   the exporter uses, so the preview and the export cannot disagree. The
//!   PCM lands as a file in the project's cache and is memory-mapped, so
//!   memory stays bounded however long the material is, and a reopened
//!   project pays nothing.
//!
//! - One cpal output stream mixes the mapped clips sample by sample, with
//!   volume and fades applied at mix time. Gain-only edits therefore never
//!   re-decode, and what a fade sounds like has exactly one definition: this
//!   file's `gain_at`.
//!
//! - The playback clock is the audio device's own sample counter. The UI
//!   receives `transport` position events and interpolates between them;
//!   there is no second clock to drift against.
//!
//! - The stream is *supervised*: pulled headphones, a changed default
//!   device, or a device that was missing at launch all rebuild the stream
//!   rather than leaving the session silent until restart. The supervisor
//!   re-seeds the new stream from the shared clock and the last clip set,
//!   so a rebuild sounds like a hiccup, not a reset.
//!
//! The mix callback owns its state and receives changes as messages; the
//! only lock it touches is an uncontended try-lock on the message channel
//! (contended solely during a rebuild, when the callback is being replaced
//! anyway). Decodes run on a small worker pool, newest request first - a
//! scrubbed trim queues many spans and the one under the playhead matters
//! most - and the clip set is re-sent to the callback as each one lands.
//!
//! Both caches are bounded. In memory, mappings not referenced by the
//! current clip set are dropped beyond a cap, oldest use first. On disk,
//! `cache/audio` keeps a byte budget: least-recently-modified files beyond
//! it are removed, never one the current clip set is using.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};

use serde::Deserialize;
use tauri::Emitter;

/// Every clip is decoded to this rate; the callback resamples to the device.
const PCM_RATE: u32 = 48_000;

/// Decoded spans kept mapped beyond the current clip set. Enough that
/// undo/redo across a few edits never re-decodes; small enough that a long
/// session cannot accumulate hundreds of mappings.
const MEMORY_CACHE_CAP: usize = 64;

/// Byte budget for `cache/audio` on disk. Stereo 16-bit 48k is ~11 MB a
/// minute, so this keeps hours of decoded material before anything is
/// evicted.
const DISK_CACHE_BUDGET: u64 = 2 * 1024 * 1024 * 1024;

/// Concurrent FFmpeg decode processes. Two keeps a dual-span edit prompt
/// without letting a scrubbed trim fan out a dozen full-file decodes.
const DECODE_WORKERS: usize = 2;

/// One audible clip, as the frontend describes it.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipSpec {
    pub path: String,
    /// Timeline seconds.
    pub start: f64,
    pub duration: f64,
    /// Seconds into the source file.
    pub source_start: f64,
    pub volume: f32,
    pub fade_in: f64,
    pub fade_out: f64,
    pub speed: f64,
    pub preserve_pitch: bool,
    /// FFmpeg audio filter chain, or empty.
    #[serde(default)]
    pub chain: String,
}

/// Decoded audio: 16-bit little-endian stereo at [`PCM_RATE`] inside a WAV
/// container, memory-mapped from the cache file so the OS decides what stays
/// resident.
///
/// WAV rather than raw PCM because the bundled FFmpeg is a trimmed build, and
/// the raw `s16le` muxer is one of the things trimmed away - the `wav` muxer
/// is not. The header costs a one-time chunk walk here; the samples inside
/// are the identical bytes.
struct Pcm {
    map: memmap2::Mmap,
    /// Byte offset of the first sample - just past the `data` chunk header.
    start: usize,
    frames: u64,
}

impl Pcm {
    fn open(path: &Path) -> Result<Pcm, String> {
        let file = std::fs::File::open(path)
            .map_err(|error| format!("could not open {}: {error}", path.display()))?;
        let map = unsafe { memmap2::Mmap::map(&file) }
            .map_err(|error| format!("could not map {}: {error}", path.display()))?;
        let (start, length) = wav_data_range(&map)
            .ok_or_else(|| format!("{} is not the wav this cache writes", path.display()))?;
        let frames = (length / 4) as u64;
        Ok(Pcm { map, start, frames })
    }

    /// One sample as a float in [-1, 1]. Out of range reads are silence.
    fn sample(&self, frame: u64, channel: usize) -> f32 {
        if frame >= self.frames {
            return 0.0;
        }
        let at = self.start + (frame as usize * 2 + channel) * 2;
        let raw = i16::from_le_bytes([self.map[at], self.map[at + 1]]);
        f32::from(raw) / 32768.0
    }
}

/// Finds the samples inside a WAV file: byte offset of the `data` chunk's
/// payload and its usable length.
///
/// A plain chunk walk, not a format library: the file was written by our own
/// FFmpeg invocation two functions down, so the only variability is which
/// bookkeeping chunks precede `data`. Sizes written to a pipe are `u32::MAX`
/// placeholders - the muxer could not seek back to patch them - in which case
/// the payload is simply the rest of the file.
fn wav_data_range(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }

    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes([bytes[at + 4], bytes[at + 5], bytes[at + 6], bytes[at + 7]]);
        let payload = at + 8;

        if id == b"data" {
            let available = bytes.len() - payload;
            let length = if size == u32::MAX { available } else { (size as usize).min(available) };
            return Some((payload, length));
        }

        // A placeholder size on any other chunk means the walk cannot
        // continue - refuse rather than misread samples as headers.
        if size == u32::MAX {
            return None;
        }
        // Chunks are word-aligned; an odd size is followed by a pad byte.
        at = payload + size as usize + (size as usize & 1);
    }
    None
}

/// A clip the mix callback can play: spec fields it needs, plus the audio.
/// Clone is cheap - the samples are behind an `Arc` - and the supervisor
/// uses it to seed a rebuilt stream with the set the old one was playing.
#[derive(Clone)]
struct ActiveClip {
    start: f64,
    duration: f64,
    volume: f32,
    fade_in: f64,
    fade_out: f64,
    pcm: Arc<Pcm>,
}

/// The one definition of a clip's gain envelope. It used to mirror a
/// `clipGainAt` in the frontend; that copy died with the webview audio path
/// (decision 0005), which is the better arrangement - nothing to drift.
fn gain_at(clip: &ActiveClip, local: f64) -> f32 {
    if local < 0.0 || local > clip.duration {
        return 0.0;
    }
    let mut gain = f64::from(clip.volume);
    if clip.fade_in > 0.0 {
        gain *= (local / clip.fade_in).min(1.0);
    }
    if clip.fade_out > 0.0 {
        gain *= ((clip.duration - local) / clip.fade_out).min(1.0);
    }
    gain.max(0.0) as f32
}

enum Msg {
    SetClips(Vec<ActiveClip>),
    Play(f64),
    Pause,
    Seek(f64),
}

struct Shared {
    /// Timeline position in microseconds, written by the mix callback.
    position_micros: AtomicU64,
    playing: AtomicBool,
}

/// One decode waiting for a worker.
struct DecodeJob {
    spec: ClipSpec,
    project: PathBuf,
    key: String,
}

/// The decode queue: newest job first, because a scrub over a trim handle
/// enqueues a span per pause and the latest is the one under the playhead.
struct DecodeQueue {
    jobs: Mutex<VecDeque<DecodeJob>>,
    available: Condvar,
}

/// A mapped span plus when it was last part of the clip set, for eviction.
struct CacheEntry {
    pcm: Arc<Pcm>,
    last_used: u64,
}

pub struct Playback {
    tx: mpsc::Sender<Msg>,
    shared: Arc<Shared>,
    /// For surfacing failures; a mute app must never be a mystery.
    app: tauri::AppHandle,
    /// Decoded clips by decode key. Bounded: see [`MEMORY_CACHE_CAP`].
    cache: Mutex<HashMap<String, CacheEntry>>,
    /// Monotonic use counter feeding `CacheEntry::last_used`.
    tick: AtomicU64,
    decoding: Mutex<HashSet<String>>,
    /// What the timeline currently wants audible.
    specs: Mutex<Vec<ClipSpec>>,
    /// The project whose cache the disk GC sweeps; set by `set_clips`.
    project: Mutex<Option<PathBuf>>,
    /// True while a disk sweep runs, so sweeps never pile up.
    sweeping: AtomicBool,
    queue: Arc<DecodeQueue>,
    /// The set most recently sent to the callback, for re-seeding a rebuilt
    /// stream. The supervisor reads it; `resync` writes it.
    last_active: Arc<Mutex<Vec<ActiveClip>>>,
}

/// A playback failure the user would otherwise experience as unexplained
/// silence. Logged for the dev console, emitted for the UI's toast - the
/// difference between "audio is broken" and a bug report that names a file.
fn report(app: &tauri::AppHandle, message: String) {
    eprintln!("wolfcut: {message}");
    let _ = app.emit("audio://error", message);
}

impl Playback {
    pub fn start(app: tauri::AppHandle) -> Arc<Playback> {
        let (tx, rx) = mpsc::channel::<Msg>();
        let shared = Arc::new(Shared {
            position_micros: AtomicU64::new(0),
            playing: AtomicBool::new(false),
        });
        let last_active: Arc<Mutex<Vec<ActiveClip>>> = Arc::new(Mutex::new(Vec::new()));

        {
            let shared = Arc::clone(&shared);
            let app = app.clone();
            let last_active = Arc::clone(&last_active);
            std::thread::Builder::new()
                .name("audio-output".into())
                .spawn(move || supervise_stream(rx, shared, app, last_active))
                .expect("could not spawn the audio thread");
        }

        // Position events at ~30Hz while playing. The UI interpolates between
        // them, so this cadence bounds correction error, not smoothness.
        {
            let shared = Arc::clone(&shared);
            let app = app.clone();
            std::thread::Builder::new()
                .name("transport-events".into())
                .spawn(move || loop {
                    std::thread::sleep(std::time::Duration::from_millis(33));
                    if shared.playing.load(Ordering::Relaxed) {
                        let position =
                            shared.position_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;
                        let _ = app.emit("transport", position);
                    }
                })
                .expect("could not spawn the transport event thread");
        }

        let playback = Arc::new(Playback {
            tx,
            shared,
            app,
            cache: Mutex::new(HashMap::new()),
            tick: AtomicU64::new(0),
            decoding: Mutex::new(HashSet::new()),
            specs: Mutex::new(Vec::new()),
            project: Mutex::new(None),
            sweeping: AtomicBool::new(false),
            queue: Arc::new(DecodeQueue {
                jobs: Mutex::new(VecDeque::new()),
                available: Condvar::new(),
            }),
            last_active,
        });

        for index in 0..DECODE_WORKERS {
            let this = Arc::clone(&playback);
            std::thread::Builder::new()
                .name(format!("audio-decode-{index}"))
                .spawn(move || decode_worker(&this))
                .expect("could not spawn a decode worker");
        }

        playback
    }

    pub fn play(&self, position: f64) {
        self.shared.playing.store(true, Ordering::Relaxed);
        self.shared
            .position_micros
            .store((position.max(0.0) * 1_000_000.0) as u64, Ordering::Relaxed);
        let _ = self.tx.send(Msg::Play(position.max(0.0)));
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Ordering::Relaxed);
        let _ = self.tx.send(Msg::Pause);
    }

    pub fn seek(&self, position: f64) {
        let clamped = position.max(0.0);
        // The callback owns the clock while playing, but a paused callback
        // returns before storing - so a paused seek must land the shared
        // position itself, or the atomic keeps serving the pre-seek time.
        self.shared
            .position_micros
            .store((clamped * 1_000_000.0) as u64, Ordering::Relaxed);
        let _ = self.tx.send(Msg::Seek(clamped));
    }

    /// Replaces the audible clip set.
    ///
    /// Anything already decoded plays immediately; anything new joins the
    /// decode queue and joins the mix when it lands. `project` is the
    /// project folder, whose cache holds the PCM files.
    pub fn set_clips(self: &Arc<Self>, project: PathBuf, specs: Vec<ClipSpec>) {
        *self.specs.lock().unwrap() = specs.clone();
        *self.project.lock().unwrap() = Some(project.clone());

        for spec in specs {
            let key = decode_key(&spec);
            if self.cache.lock().unwrap().contains_key(&key) {
                continue;
            }
            if !self.decoding.lock().unwrap().insert(key.clone()) {
                continue;
            }
            let mut jobs = self.queue.jobs.lock().unwrap();
            jobs.push_back(DecodeJob { spec, project: project.clone(), key });
            self.queue.available.notify_one();
        }

        self.resync();
        self.evict_memory();
        self.sweep_disk(project);
    }

    /// Sends the mix callback everything currently wanted *and* decoded, and
    /// remembers the set so a rebuilt stream can be seeded with it.
    fn resync(&self) {
        let specs = self.specs.lock().unwrap();
        let now = self.tick.fetch_add(1, Ordering::Relaxed) + 1;
        let mut cache = self.cache.lock().unwrap();
        let active: Vec<ActiveClip> = specs
            .iter()
            .filter_map(|spec| {
                let entry = cache.get_mut(&decode_key(spec))?;
                entry.last_used = now;
                Some(ActiveClip {
                    start: spec.start,
                    duration: spec.duration,
                    volume: spec.volume,
                    fade_in: spec.fade_in,
                    fade_out: spec.fade_out,
                    pcm: Arc::clone(&entry.pcm),
                })
            })
            .collect();
        drop(cache);
        *self.last_active.lock().unwrap() = active.clone();
        let _ = self.tx.send(Msg::SetClips(active));
    }

    /// Drops mapped spans beyond [`MEMORY_CACHE_CAP`], oldest use first,
    /// never one the current clip set references. The mapping is address
    /// space and page cache, not copies - but a long session accumulates a
    /// mapping per edit of every trimmed span, and those add up.
    fn evict_memory(&self) {
        let live: HashSet<String> =
            self.specs.lock().unwrap().iter().map(decode_key).collect();
        let mut cache = self.cache.lock().unwrap();
        while cache.len() > MEMORY_CACHE_CAP {
            let doomed = cache
                .iter()
                .filter(|(key, _)| !live.contains(*key))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            match doomed {
                Some(key) => cache.remove(&key),
                // Everything over the cap is live: a genuinely enormous clip
                // set. Keeping it is correct; evicting live spans would just
                // re-decode them on the next resync.
                None => break,
            };
        }
    }

    /// Sweeps `cache/audio` down to [`DISK_CACHE_BUDGET`] on a worker
    /// thread: least-recently-modified first, never a file the current clip
    /// set is using. Old `.decoding` leftovers from crashed runs go too.
    fn sweep_disk(self: &Arc<Self>, project: PathBuf) {
        if self.sweeping.swap(true, Ordering::Relaxed) {
            return;
        }
        let this = Arc::clone(self);
        std::thread::Builder::new()
            .name("audio-cache-sweep".into())
            .spawn(move || {
                let live: HashSet<String> =
                    this.specs.lock().unwrap().iter().map(decode_key).collect();
                let directory = project.join("cache").join("audio");
                if let Some(doomed) = sweep_plan(&directory, &live, DISK_CACHE_BUDGET) {
                    for path in doomed {
                        let _ = std::fs::remove_file(path);
                    }
                }
                this.sweeping.store(false, Ordering::Relaxed);
            })
            .expect("could not spawn the cache sweep thread");
    }
}

/// One decode worker: takes the newest queued job, skips jobs whose span the
/// timeline no longer wants, decodes, lands the result, resyncs.
fn decode_worker(playback: &Arc<Playback>) {
    loop {
        let job = {
            let mut jobs = playback.queue.jobs.lock().unwrap();
            loop {
                // Newest first: pop from the back where set_clips pushes.
                match jobs.pop_back() {
                    Some(job) => break job,
                    None => jobs = playback.queue.available.wait(jobs).unwrap(),
                }
            }
        };

        // A span queued by an edit that has since been edited again is dead
        // weight; skip it before paying for FFmpeg.
        let wanted = playback
            .specs
            .lock()
            .unwrap()
            .iter()
            .any(|spec| decode_key(spec) == job.key);
        if !wanted {
            playback.decoding.lock().unwrap().remove(&job.key);
            continue;
        }

        match decode(&job.spec, &job.project, &job.key) {
            Ok(pcm) => {
                let now = playback.tick.fetch_add(1, Ordering::Relaxed) + 1;
                playback.cache.lock().unwrap().insert(
                    job.key.clone(),
                    CacheEntry { pcm: Arc::new(pcm), last_used: now },
                );
            }
            Err(error) => report(
                &playback.app,
                format!("audio decode failed for {}: {error}", job.spec.path),
            ),
        }
        playback.decoding.lock().unwrap().remove(&job.key);
        playback.resync();
    }
}

/// Which files a sweep of `directory` should delete to fit `budget` bytes:
/// least-recently-modified first, never a `.wav` whose key is in `live`,
/// plus any `.decoding` leftover older than a day. Returns `None` when the
/// directory cannot be read (not created yet - nothing to sweep).
fn sweep_plan(
    directory: &Path,
    live: &HashSet<String>,
    budget: u64,
) -> Option<Vec<PathBuf>> {
    let entries = std::fs::read_dir(directory).ok()?;
    let now = std::time::SystemTime::now();

    let mut doomed: Vec<PathBuf> = Vec::new();
    let mut candidates: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    let mut total: u64 = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Ok(meta) = entry.metadata() else { continue };

        // A `.decoding` file is a crashed or torn run once it is old enough
        // that no live decode can still be writing it.
        if name.ends_with(".decoding") {
            let stale = meta
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age.as_secs() > 24 * 3600);
            if stale {
                doomed.push(path);
            }
            continue;
        }

        let Some(key) = name.strip_suffix(".wav") else { continue };
        total += meta.len();
        if live.contains(key) {
            continue;
        }
        let modified = meta.modified().unwrap_or(now);
        candidates.push((modified, meta.len(), path));
    }

    if total > budget {
        candidates.sort_by_key(|(modified, ..)| *modified);
        for (_, size, path) in candidates {
            if total <= budget {
                break;
            }
            total = total.saturating_sub(size);
            doomed.push(path);
        }
    }
    Some(doomed)
}

/// Identity of one decoded span: everything that changes the samples.
/// Volume, fades and timeline position are deliberately absent - they apply
/// at mix time.
///
/// FNV-1a rather than the standard library's hasher, because these keys name
/// files that outlive the process: `DefaultHasher` is free to change between
/// Rust releases, and a changed hash would silently orphan every project's
/// audio cache on a toolchain upgrade.
fn decode_key(spec: &ClipSpec) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &byte in bytes {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(spec.path.as_bytes());
    eat(&spec.source_start.to_bits().to_le_bytes());
    eat(&spec.duration.to_bits().to_le_bytes());
    eat(&spec.speed.to_bits().to_le_bytes());
    eat(&[u8::from(spec.preserve_pitch)]);
    eat(spec.chain.as_bytes());
    format!("{hash:016x}")
}

/// Decodes one clip span to the cache and maps it.
///
/// The FFmpeg invocation mirrors the exporter's treatment of filters and
/// speed, so what the preview mixes is what the export renders. A file
/// already in the cache is reused as is - that is the point of the cache.
fn decode(spec: &ClipSpec, project: &Path, key: &str) -> Result<Pcm, String> {
    wolfcut_media::audio::validate_chain(&spec.chain).map_err(|error| error.to_string())?;

    let directory = project.join("cache").join("audio");
    // `.wav`, and `.pcm` before the muxer change - old raw caches are simply
    // orphaned and re-decoded, because a cache must never need migrating.
    let destination = directory.join(format!("{key}.wav"));
    if destination.is_file() {
        return Pcm::open(&destination);
    }
    std::fs::create_dir_all(&directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))?;

    // Speed first, then the clip's own filters - the same order the export
    // graph applies, because these are one definition, not two. The engine
    // owns what speed *means*; this function only decodes.
    let mut filters: Vec<String> =
        wolfcut_media::audio::speed_filters(spec.speed, spec.preserve_pitch);
    if !spec.chain.is_empty() {
        filters.push(spec.chain.clone());
    }
    filters.push(format!("aresample={PCM_RATE}"));
    filters.push("aformat=sample_fmts=s16:channel_layouts=stereo".to_owned());

    // A sped-up clip reads further into the file than it occupies on the
    // timeline: the source window is `duration * speed`, exactly the window
    // the exporter trims.
    let window = spec.duration * wolfcut_media::audio::clamp_speed(spec.speed);

    let temporary = directory.join(format!("{key}.wav.decoding"));
    let file = std::fs::File::create(&temporary)
        .map_err(|error| format!("could not create {}: {error}", temporary.display()))?;

    let status = wolfcut_media::command(wolfcut_media::ffmpeg())
        .args(["-hide_banner", "-nostdin", "-loglevel", "error"])
        .args(["-ss", &format!("{:.6}", spec.source_start)])
        .args(["-t", &format!("{window:.6}")])
        .args(["-i", &spec.path])
        .args(["-vn", "-af", &filters.join(",")])
        // `wav`, not `s16le`: the bundled FFmpeg is a trimmed build and the
        // raw muxer is not in it. `Pcm::open` skips the header.
        .args(["-f", "wav", "pipe:1"])
        .stdout(file)
        .status()
        .map_err(|error| format!("could not run ffmpeg: {error}"))?;

    if !status.success() {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("ffmpeg exited with {status}"));
    }

    std::fs::rename(&temporary, &destination)
        .map_err(|error| format!("could not commit {}: {error}", destination.display()))?;
    Pcm::open(&destination)
}

/// Owns the output stream for the life of the app, rebuilding it whenever
/// it dies or the default device changes.
///
/// Setup failure is a retry with backoff, not a permanent return: a machine
/// that boots WolfCut before its audio device is ready, or loses its only
/// device mid-session, gets sound back when the device does. A rebuilt
/// stream is seeded from the shared clock and the last clip set, so playback
/// carries straight on.
fn supervise_stream(
    rx: mpsc::Receiver<Msg>,
    shared: Arc<Shared>,
    app: tauri::AppHandle,
    last_active: Arc<Mutex<Vec<ActiveClip>>>,
) {
    use cpal::traits::{DeviceTrait, HostTrait};

    // The callback drains this through a try-lock: uncontended in steady
    // state (the supervisor only takes it between streams), and a missed
    // drain on a contended tick is caught on the next callback.
    let rx = Arc::new(Mutex::new(rx));
    let mut backoff = std::time::Duration::from_secs(1);
    let mut reported_missing = false;

    loop {
        let host = cpal::default_host();
        let built = host
            .default_output_device()
            .ok_or_else(|| "no audio output device".to_owned())
            .and_then(|device| {
                let name = device.name().unwrap_or_default();
                build_stream(&device, &rx, &shared, &app, &last_active).map(|s| (s, name))
            });

        match built {
            Ok((stream, device_name)) => {
                backoff = std::time::Duration::from_secs(1);
                reported_missing = false;

                // Watch for the two reasons to rebuild: the stream reported
                // an error, or the default device is no longer the one the
                // stream was built on (headphones unplugged, output switched
                // in system settings).
                let failed = stream.failed;
                let _stream = stream.stream;
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(1000));
                    if failed.load(Ordering::Relaxed) {
                        report(&app, "audio stream failed; rebuilding".to_owned());
                        break;
                    }
                    let current = cpal::default_host()
                        .default_output_device()
                        .and_then(|device| device.name().ok());
                    if let Some(current) = current {
                        if current != device_name {
                            // Informational only - switching outputs is a
                            // normal act, not an error toast.
                            eprintln!("wolfcut: audio device changed to {current}; rebuilding");
                            break;
                        }
                    }
                }
                // `_stream` drops here, releasing the device before rebuild.
            }
            Err(error) => {
                // Once, not every retry: a machine with no sound card would
                // otherwise toast every backoff tick forever.
                if !reported_missing {
                    report(&app, format!("audio output unavailable: {error}; will keep trying"));
                    reported_missing = true;
                }
                std::thread::sleep(backoff);
                backoff = (backoff * 2).min(std::time::Duration::from_secs(30));
            }
        }
    }
}

/// A built, playing stream plus its error flag.
struct BuiltStream {
    stream: cpal::Stream,
    failed: Arc<AtomicBool>,
}

/// Builds and starts one output stream on `device`, seeded with the current
/// clip set and clock so a rebuild resumes where the last stream stopped.
fn build_stream(
    device: &cpal::Device,
    rx: &Arc<Mutex<mpsc::Receiver<Msg>>>,
    shared: &Arc<Shared>,
    app: &tauri::AppHandle,
    last_active: &Arc<Mutex<Vec<ActiveClip>>>,
) -> Result<BuiltStream, String> {
    use cpal::traits::DeviceTrait;

    let config = device
        .default_output_config()
        .map_err(|error| format!("no default output config: {error}"))?;
    let sample_format = config.sample_format();
    let config: cpal::StreamConfig = config.into();

    match sample_format {
        cpal::SampleFormat::F32 => stream_for::<f32>(device, &config, rx, shared, app, last_active),
        cpal::SampleFormat::I16 => stream_for::<i16>(device, &config, rx, shared, app, last_active),
        cpal::SampleFormat::U16 => stream_for::<u16>(device, &config, rx, shared, app, last_active),
        other => Err(format!("unsupported sample format {other:?}")),
    }
}

fn stream_for<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    rx: &Arc<Mutex<mpsc::Receiver<Msg>>>,
    shared: &Arc<Shared>,
    app: &tauri::AppHandle,
    last_active: &Arc<Mutex<Vec<ActiveClip>>>,
) -> Result<BuiltStream, String>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    use cpal::traits::{DeviceTrait, StreamTrait};

    let channels = config.channels as usize;
    let step = 1.0 / f64::from(config.sample_rate.0);

    // Seed from the world as it is, not from zero: a stream rebuilt mid-
    // playback resumes the same clips at the shared clock's position.
    let mut clips: Vec<ActiveClip> = last_active.lock().unwrap().clone();
    let mut playing = shared.playing.load(Ordering::Relaxed);
    let mut position = shared.position_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;

    let failed = Arc::new(AtomicBool::new(false));
    let rx = Arc::clone(rx);
    let shared = Arc::clone(shared);

    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let mut sought = false;
                // try_lock, not lock: the audio callback must never block.
                // Contention only exists while the supervisor swaps streams,
                // and then this callback is on its way out anyway.
                if let Ok(receiver) = rx.try_lock() {
                    while let Ok(message) = receiver.try_recv() {
                        match message {
                            Msg::SetClips(next) => clips = next,
                            Msg::Play(at) => {
                                position = at;
                                playing = true;
                            }
                            Msg::Pause => playing = false,
                            Msg::Seek(at) => {
                                position = at;
                                sought = true;
                            }
                        }
                    }
                }

                if !playing {
                    // A paused seek must reach the shared clock - the playing
                    // store below never runs. Only on an actual seek, so this
                    // silent branch cannot overwrite a fresher play() store
                    // with its own stale idea of the position.
                    if sought {
                        shared
                            .position_micros
                            .store((position * 1_000_000.0) as u64, Ordering::Relaxed);
                    }
                    data.fill(T::from_sample(0.0));
                    return;
                }

                for frame in data.chunks_mut(channels) {
                    let mut left = 0.0f32;
                    let mut right = 0.0f32;

                    for clip in &clips {
                        let local = position - clip.start;
                        if local < 0.0 || local >= clip.duration {
                            continue;
                        }
                        let gain = gain_at(clip, local);
                        if gain <= 0.0 {
                            continue;
                        }

                        // Linear interpolation across the 48k source grid -
                        // exact when the device runs at 48k, inaudibly close
                        // when it does not.
                        let read = local * f64::from(PCM_RATE);
                        let index = read.floor() as u64;
                        let fraction = (read - read.floor()) as f32;
                        let pcm = &clip.pcm;
                        left += gain
                            * (pcm.sample(index, 0) * (1.0 - fraction)
                                + pcm.sample(index + 1, 0) * fraction);
                        right += gain
                            * (pcm.sample(index, 1) * (1.0 - fraction)
                                + pcm.sample(index + 1, 1) * fraction);
                    }

                    // Hard limit. A mix of boosted clips can exceed full
                    // scale; wrapping would be far worse than flattening.
                    frame[0] = T::from_sample(left.clamp(-1.0, 1.0));
                    if channels > 1 {
                        frame[1] = T::from_sample(right.clamp(-1.0, 1.0));
                    }
                    for extra in frame.iter_mut().skip(2) {
                        *extra = T::from_sample(0.0);
                    }

                    position += step;
                }

                shared
                    .position_micros
                    .store((position * 1_000_000.0) as u64, Ordering::Relaxed);
            },
            {
                let app = app.clone();
                let failed = Arc::clone(&failed);
                move |error| {
                    // The supervisor rebuilds on this flag; the report says
                    // why the sound hiccuped.
                    report(&app, format!("audio stream error: {error}"));
                    failed.store(true, Ordering::Relaxed);
                }
            },
            None,
        )
        .map_err(|error| format!("could not build the output stream: {error}"))?;

    stream
        .play()
        .map_err(|error| format!("could not start the output stream: {error}"))?;

    Ok(BuiltStream { stream, failed })
}

#[cfg(test)]
mod tests {
    use super::{sweep_plan, wav_data_range};

    /// A minimal RIFF/WAVE: fmt chunk, then a data chunk with `samples`.
    fn wav(data_size: u32, samples: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&u32::MAX.to_le_bytes()); // pipe placeholder
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 16]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_size.to_le_bytes());
        bytes.extend_from_slice(samples);
        bytes
    }

    #[test]
    fn finds_the_data_chunk_past_the_headers() {
        let bytes = wav(8, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let (start, length) = wav_data_range(&bytes).expect("valid wav");
        assert_eq!(length, 8);
        assert_eq!(&bytes[start..start + length], &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn a_placeholder_data_size_means_the_rest_of_the_file() {
        // What the wav muxer writes to a pipe: it cannot seek back to patch.
        let bytes = wav(u32::MAX, &[9, 9, 9, 9]);
        let (start, length) = wav_data_range(&bytes).expect("valid wav");
        assert_eq!(length, 4);
        assert_eq!(start, bytes.len() - 4);
    }

    #[test]
    fn a_stated_size_is_clamped_to_the_file() {
        // A truncated decode must read short, not out of bounds.
        let bytes = wav(1000, &[1, 2]);
        assert_eq!(wav_data_range(&bytes).map(|(_, length)| length), Some(2));
    }

    #[test]
    fn refuses_files_that_are_not_wav() {
        assert_eq!(wav_data_range(b""), None);
        assert_eq!(wav_data_range(b"RIFFxxxxJUNK"), None);
        assert_eq!(wav_data_range(&[0u8; 64]), None);
    }

    #[test]
    fn the_sweep_removes_the_oldest_dead_files_first_and_never_live_ones() {
        use std::collections::HashSet;

        let directory = std::env::temp_dir().join(format!(
            "wolfcut-sweep-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&directory).expect("scratch dir");

        // Three 4-byte caches, written oldest-first so mtimes order them.
        for name in ["old", "live", "new"] {
            std::fs::write(directory.join(format!("{name}.wav")), [0u8; 4]).expect("writes");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        let live: HashSet<String> = ["live".to_owned()].into();

        // Budget for two files: only the oldest dead file goes; the live one
        // survives despite being older than "new".
        let doomed = sweep_plan(&directory, &live, 8).expect("plans");
        let names: Vec<_> = doomed
            .iter()
            .filter_map(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, vec!["old.wav"]);

        // Inside budget: nothing goes.
        assert_eq!(sweep_plan(&directory, &live, 1024).expect("plans"), Vec::<std::path::PathBuf>::new());

        // A directory that does not exist yet is nothing to sweep, not a
        // panic - the first decode has simply not happened.
        assert!(sweep_plan(&directory.join("missing"), &live, 8).is_none());

        let _ = std::fs::remove_dir_all(&directory);
    }
}
