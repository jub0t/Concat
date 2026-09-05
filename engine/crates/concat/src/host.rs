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
use std::rc::Rc;
use std::sync::Arc;

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
    /// The cutout model, and the masks it finds for the project's media.
    pub cutouts: Arc<concat_host::Cutouts>,
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
            dirs,
            playback: Playback::start(Arc::new(Events))?,
            monitor: match gpu {
                Some(gpu) => Monitor::with_gpu(gpu.device, gpu.queue),
                None => Monitor::new(),
            },
            exporter: Exporter::new(),
            transcriber: Arc::new(Transcriber::new()),
            speech: Arc::new(Speech::new()),
            cutouts: Arc::new(concat_host::Cutouts::new()),
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

/// Runs `work` on its own thread, then `then` on the event-loop thread with
/// the result, the state and the window - followed by a full publish, so a
/// completion never has to remember to redraw.
pub fn spawn<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
    then: impl FnOnce(&mut Studio, &App, &Models, T) + Send + 'static,
) {
    std::thread::spawn(move || {
        let result = work();
        let _ = slint::invoke_from_event_loop(move || {
            Shell::with(|shell, app| {
                {
                    let mut studio = shell.studio.borrow_mut();
                    then(&mut studio, &app, &shell.models, result);
                }
                shell.studio.borrow().publish(&app, &shell.models);
            });
        });
    });
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

/// A probe's error, in the words a toast can use.
pub fn probe_error(error: &str) -> String {
    format!("Could not import: {error}")
}

/// The still and the peaks for one media item, decoded on a worker.
pub struct MediaArt {
    /// The media id the art belongs to.
    pub id: String,
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
/// lives.
pub fn media_art(
    id: String,
    path: String,
    kind: concat_project::model::MediaKind,
    has_audio: bool,
    duration: Option<f64>,
    project: String,
) -> MediaArt {
    use concat_project::model::MediaKind;
    let thumbnail = match kind {
        MediaKind::Video | MediaKind::Image => {
            let at = duration.map_or(0.0, |seconds| (seconds * 0.25).min(2.0));
            media::still_at(&path, if kind == MediaKind::Image { 0.0 } else { at }, 160).ok()
        }
        MediaKind::Audio => None,
    };
    let peaks = (kind == MediaKind::Audio || has_audio)
        .then(|| media::peaks(&path, Some(&project)).ok().map(Arc::new))
        .flatten();
    let strip = match kind {
        MediaKind::Video => media::filmstrip(&path, STRIP_FRAMES, STRIP_HEIGHT)
            .ok()
            .map(|frame| (frame, STRIP_FRAMES)),
        MediaKind::Image => thumbnail.clone().map(|frame| (frame, 1)),
        MediaKind::Audio => None,
    };
    MediaArt {
        id,
        thumbnail,
        peaks,
        strip,
    }
}
