// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The engine's services, started once, and the bridge between their
//! threads and the window's.
//!
//! Slint's models and properties may only be touched on the event-loop
//! thread. Everything slow - probes, decodes, renders, downloads - runs on
//! its own thread through [`spawn`], and hands its result back with
//! `slint::invoke_from_event_loop`, where [`Shell::with`] reaches the window
//! and its state again. The state itself never leaves the event-loop thread,
//! which is why it can live in a plain `RefCell` with no lock.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use concat_host::export::Exporter;
use concat_host::playback::{Playback, PlaybackEvents};
use concat_host::preview::Monitor;
use concat_host::{AppDirs, media};
use concat_speech::{Speech, Transcriber};

use crate::gpu::Gpu;
use crate::studio::{Models, Studio};
use crate::ui::App;

/// The engine's long-lived services.
pub struct Host {
    /// Where recents, preferences and models live on this machine.
    pub dirs: AppDirs,
    /// The audio engine and the clock.
    pub playback: Arc<Playback>,
    /// The monitor's reader pool.
    pub monitor: Monitor,
    /// The one-export-at-a-time slot.
    pub exporter: Exporter,
    /// whisper.cpp, and its model downloads.
    pub transcriber: Arc<Transcriber>,
    /// Kokoro, and its model downloads.
    pub speech: Arc<Speech>,
    /// Titles painted to pictures, and the cache of them.
    pub titles: concat_host::Titles,
    /// The cutout models, and the masks they find for the project's media.
    pub cutouts: Arc<concat_host::Cutouts>,
    /// The brush model, and the regions it reads under smart strokes.
    pub brushes: Arc<concat_host::Brushes>,
}

impl Host {
    /// Starts every service. The audio device may not be there yet; playback
    /// keeps trying on its own thread and says so through a toast. `gpu` is
    /// the window's device; with it the monitor composites where the window
    /// draws.
    pub fn start(gpu: Option<Gpu>) -> Result<Host, String> {
        let dirs = AppDirs::locate()?;
        let _ = std::fs::create_dir_all(&dirs.config);
        Ok(Host {
            titles: concat_host::Titles::new(&dirs),
            cutouts: Arc::new(concat_host::Cutouts::new(&dirs.data)),
            brushes: Arc::new(concat_host::Brushes::new(&dirs.data)),
            dirs,
            playback: Playback::start(Arc::new(Events))?,
            monitor: match gpu {
                Some(gpu) => Monitor::with_gpu(gpu.device, gpu.queue),
                None => Monitor::new(),
            },
            exporter: Exporter::new(),
            transcriber: Arc::new(Transcriber::new()),
            speech: Arc::new(Speech::new()),
        })
    }
}

/// Playback's way back to the window. The clock is polled by a timer while
/// playing, so only failures need to cross here.
struct Events;

impl PlaybackEvents for Events {
    fn position(&self, _seconds: f64) {}

    fn error(&self, message: String) {
        let _ = slint::invoke_from_event_loop(move || {
            Shell::with(|shell, app| {
                shell.studio.borrow_mut().notify(&message, true);
                shell.studio.borrow().publish(&app, &shell.models);
            });
        });
    }
}

/// Everything a handler needs: the window, the state and the models the
/// state publishes into. One per process, reachable from the event-loop
/// thread by [`Shell::with`].
pub struct Shell {
    /// The window, weakly: a callback that outlives it is a no-op.
    pub app: slint::Weak<App>,
    /// The window's state.
    pub studio: RefCell<Studio>,
    /// The live models, handed to Slint once and never replaced.
    pub models: Models,
}

thread_local! {
    static SHELL: RefCell<Option<Rc<Shell>>> = const { RefCell::new(None) };
}

impl Shell {
    /// Makes this the process's shell. Called once from `main`.
    pub fn install(shell: Rc<Shell>) {
        SHELL.with(|slot| *slot.borrow_mut() = Some(shell));
    }

    /// Runs `body` with the shell and a strong handle on the window, on the
    /// event-loop thread. Does nothing if the window is gone or the shell
    /// was never installed.
    pub fn with(body: impl FnOnce(&Shell, App)) {
        let shell = SHELL.with(|slot| slot.borrow().clone());
        if let Some(shell) = shell
            && let Some(app) = shell.app.upgrade()
        {
            body(&shell, app);
        }
    }
}

/// The workers every background job runs on: a few threads kept for the
/// life of the process, fed from one queue. A thread per job was a thread
/// per monitor frame, thirty a second during playback.
fn workers() -> &'static std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>> {
    static WORKERS: std::sync::OnceLock<std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>> =
        std::sync::OnceLock::new();
    WORKERS.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
        // Three: the monitor's frame, its prefetch, and whatever thumbnail
        // or analysis is running, without any of them queuing behind the
        // others.
        for index in 0..3 {
            let receiver = std::sync::Arc::clone(&receiver);
            std::thread::Builder::new()
                .name(format!("concat-worker-{index}"))
                .spawn(move || {
                    loop {
                        let job = receiver
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => return,
                        }
                    }
                })
                .expect("a worker thread");
        }
        sender
    })
}

/// Runs `work` on a worker with nothing to report: a prefetch, a warm-up.
pub fn spawn_detached(work: impl FnOnce() + Send + 'static) {
    let _ = workers().send(Box::new(work));
}

/// Runs `work` on a worker, then `then` on the event-loop thread with the
/// result, the state and the window - followed by a full publish, so a
/// completion never has to remember to redraw.
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    spawn_detached(move || deliver(work(), then));
}

/// Hands a worker's result to the event-loop thread: `then` with the state
/// and the window, and a full publish after it.
fn deliver<T: Send + 'static>(
    result: T,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, app| {
            {
                let mut studio = shell.studio.borrow_mut();
                then(&mut studio, &app, &shell.models, result);
            }
            shell.studio.borrow().publish(&app, &shell.models);
        });
    });
}

/// The bin's artwork lane: its own few threads, taking jobs off one queue.
///
/// Not the shared workers. Those are the monitor's: a frame, its prefetch,
/// and one more, and an import of twenty files asks for twenty thumbnails,
/// twenty filmstrips and twenty waveforms at once - on the shared workers
/// the monitor would wait behind all of it, and as one thread per job it
/// was every core at a hundred percent until the bin had its pictures, the
/// machine unusable for a minute (#52). The artwork is decoration; it can
/// arrive a few files at a time. So it queues here, on threads the monitor
/// never needs, and only [`art_workers`] decoders ever run together,
/// whatever the size of the import.
struct ArtLane {
    queue: Mutex<VecDeque<Box<dyn FnOnce() + Send>>>,
    ready: Condvar,
}

/// How many artwork decoders run at once: a quarter of the machine's
/// threads, one at least and three at most. A decoder is single-threaded,
/// so this is roughly the share of the machine the bin's pictures may take
/// while the window, playback and the monitor keep the rest.
fn art_workers() -> usize {
    std::thread::available_parallelism().map_or(1, |threads| (threads.get() / 4).clamp(1, 3))
}

/// Like [`spawn`], on the artwork lane: `work` waits its turn behind the
/// other artwork rather than starting a thread of its own.
pub fn spawn_art<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    static LANE: OnceLock<Arc<ArtLane>> = OnceLock::new();
    let lane = LANE.get_or_init(|| {
        let lane = Arc::new(ArtLane {
            queue: Mutex::new(VecDeque::new()),
            ready: Condvar::new(),
        });
        for index in 0..art_workers() {
            let lane = Arc::clone(&lane);
            let _ = std::thread::Builder::new()
                .name(format!("media-art-{index}"))
                .spawn(move || {
                    loop {
                        let job = {
                            let mut queue = lane.queue.lock().unwrap_or_else(|e| e.into_inner());
                            loop {
                                if let Some(job) = queue.pop_front() {
                                    break job;
                                }
                                queue = lane.ready.wait(queue).unwrap_or_else(|e| e.into_inner());
                            }
                        };
                        job();
                    }
                });
        }
        lane
    });
    lane.queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(Box::new(move || deliver(work(), then)));
    lane.ready.notify_one();
}

/// Runs `body` on the event-loop thread from anywhere, with a full publish
/// after. For progress reports from a worker.
pub fn on_ui(body: impl FnOnce(&mut Studio, &App, &Models) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, app| {
            {
                let mut studio = shell.studio.borrow_mut();
                body(&mut studio, &app, &shell.models);
            }
            shell.studio.borrow().publish(&app, &shell.models);
        });
    });
}

/// A decoded frame as a Slint image.
pub fn image_of(frame: &concat_core::frame::Frame) -> slint::Image {
    let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
        frame.pixels(),
        frame.width(),
        frame.height(),
    );
    slint::Image::from_rgba8(buffer)
}

/// The JPEG at `path`, if it can be read, as a Slint image.
pub fn image_at(path: &std::path::Path) -> Option<slint::Image> {
    slint::Image::load_from_path(path).ok()
}

/// A filmstrip restored from the project's artwork cache.
pub struct CachedStrip {
    /// The strip image.
    pub image: slint::Image,
    /// How many frames the picture holds.
    pub frames: u32,
    /// One frame's width in the picture's pixels.
    pub frame_width: u32,
    /// The picture's height in pixels.
    pub height: u32,
}

/// Artwork restored from the project's cache.
pub struct CachedMediaArt {
    /// Cached thumbnail, when present.
    pub thumbnail: Option<slint::Image>,
    /// Cached filmstrip, when present.
    pub strip: Option<CachedStrip>,
}

/// Loads a media item's thumbnail and filmstrip from the project's JPEG cache.
///
/// The cache key includes the media id plus the source file's size and mtime,
/// so replacing a file with the same project media id naturally misses and
/// regenerates fresh artwork.
pub fn cached_media_art(
    project: &str,
    id: &str,
    path: &str,
    kind: concat_project::model::MediaKind,
) -> CachedMediaArt {
    use concat_project::model::MediaKind;

    if kind == MediaKind::Audio {
        return CachedMediaArt {
            thumbnail: None,
            strip: None,
        };
    }

    let dir = art_cache_dir(project);
    let stem = art_cache_stem(id, path);

    let thumbnail = image_at(&dir.join(format!("{stem}.thumb.jpg")));

    let strip = std::fs::read_dir(&dir).ok().and_then(|entries| {
        let prefix = format!("{stem}.strip.");
        entries.filter_map(Result::ok).find_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            let rest = name.strip_prefix(&prefix)?.strip_suffix(".jpg")?;
            let mut parts = rest.split('.');
            let frames: u32 = parts.next()?.parse().ok()?;
            let frame_width: u32 = parts.next()?.parse().ok()?;
            let height: u32 = parts.next()?.parse().ok()?;
            if parts.next().is_some() {
                return None;
            }
            let image = image_at(&entry.path())?;
            Some(CachedStrip {
                image,
                frames,
                frame_width,
                height,
            })
        })
    });

    CachedMediaArt { thumbnail, strip }
}

fn art_cache_dir(project: &str) -> std::path::PathBuf {
    std::path::Path::new(project).join("cache").join("art")
}

fn art_cache_stem(id: &str, path: &str) -> String {
    format!("{}-{}", clean_cache_part(id), source_stamp(path))
}

fn clean_cache_part(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn source_stamp(path: &str) -> String {
    let Ok(meta) = std::fs::metadata(path) else {
        return "missing".to_owned();
    };
    let len = meta.len();
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{len:x}-{modified:x}")
}

fn save_media_art_cache(
    project: &str,
    id: &str,
    path: &str,
    thumbnail: Option<&concat_core::frame::Frame>,
    strip: Option<(&concat_core::frame::Frame, u32)>,
) {
    let dir = art_cache_dir(project);
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let stem = art_cache_stem(id, path);

    if let Some(frame) = thumbnail {
        let _ = save_frame_jpeg(&dir.join(format!("{stem}.thumb.jpg")), frame, 82);
    }

    if let Some((frame, frames)) = strip {
        let frames = frames.max(1);
        let frame_width = frame.width() / frames;
        let name = format!("{stem}.strip.{frames}.{frame_width}.{}.jpg", frame.height());
        let _ = save_frame_jpeg(&dir.join(name), frame, 78);
    }
}

fn save_frame_jpeg(
    path: &std::path::Path,
    frame: &concat_core::frame::Frame,
    quality: u8,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut rgb = Vec::with_capacity(frame.width() as usize * frame.height() as usize * 3);
    for pixel in frame.pixels().chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }

    let file = std::fs::File::create(path)?;
    let mut writer = std::io::BufWriter::new(file);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut writer, quality);
    encoder.encode(
        &rgb,
        frame.width(),
        frame.height(),
        image::ColorType::Rgb8.into(),
    )?;
    Ok(())
}

/// A probe's error, in the words a toast can use.
pub fn probe_error(error: &str) -> String {
    format!("Could not import: {error}")
}

/// The still and the peaks for one media item, decoded on a worker.
pub struct MediaArt {
    /// The media id the art belongs to.
    pub id: String,
    /// Which audio stream the peaks are of: `None` for the file's first,
    /// which is the media's own art, or a named one a clip plays. Named
    /// streams get peaks only, never a picture.
    pub stream: Option<u32>,
    /// A small first frame, for footage and stills. A frame rather than an
    /// image: a Slint image cannot cross a thread, and this is made on one.
    pub thumbnail: Option<concat_core::frame::Frame>,
    /// The waveform, for anything with sound.
    pub peaks: Option<Arc<concat_media::Peaks>>,
    /// Frames sampled evenly across the footage, side by side in one
    /// picture, and how many there are. A still is a strip of one.
    pub strip: Option<(concat_core::frame::Frame, u32)>,
}

/// Frames in a filmstrip, and the strip's height in logical pixels.
///
/// Twenty-four across a file is enough that a clip a few seconds long
/// shows different pictures along its length; the height is the tallest
/// lane's body, so the lanes never upscale it.
const STRIP_FRAMES: u32 = 24;
const STRIP_HEIGHT: u32 = 64;

/// Decodes the art for one media item. `project` is where the peaks cache
/// lives. With a `stream` named, only that stream's peaks: the pictures are
/// the media's, made once with the default stream's peaks.
pub fn media_art(
    id: String,
    path: String,
    kind: concat_project::model::MediaKind,
    has_audio: bool,
    duration: Option<f64>,
    project: String,
    stream: Option<u32>,
    pictures: bool,
) -> MediaArt {
    use concat_project::model::MediaKind;
    if stream.is_some() {
        return MediaArt {
            id,
            stream,
            thumbnail: None,
            peaks: media::peaks(&path, stream, Some(&project))
                .ok()
                .map(Arc::new),
            strip: None,
        };
    }
    let thumbnail = if pictures {
        match kind {
            MediaKind::Video | MediaKind::Image => {
                // Within the first two seconds, so the walk from the keyframe
                // before it is at most those two seconds of frames.
                let at = duration.map_or(0.0, |seconds| (seconds * 0.25).min(2.0));
                media::still_at(&path, if kind == MediaKind::Image { 0.0 } else { at }, 160).ok()
            }
            MediaKind::Audio => None,
        }
    } else {
        None
    };
    let peaks = (kind == MediaKind::Audio || has_audio)
        .then(|| media::peaks(&path, None, Some(&project)).ok().map(Arc::new))
        .flatten();
    let strip = if pictures {
        match kind {
            MediaKind::Video => media::filmstrip(&path, STRIP_FRAMES, STRIP_HEIGHT)
                .ok()
                .map(|frame| (frame, STRIP_FRAMES)),
            MediaKind::Image => thumbnail.clone().map(|frame| (frame, 1)),
            MediaKind::Audio => None,
        }
    } else {
        None
    };

    if pictures {
        save_media_art_cache(
            &project,
            &id,
            &path,
            thumbnail.as_ref(),
            strip.as_ref().map(|(frame, frames)| (frame, *frames)),
        );
    }

    MediaArt {
        id,
        stream: None,
        thumbnail,
        peaks,
        strip,
    }
}
