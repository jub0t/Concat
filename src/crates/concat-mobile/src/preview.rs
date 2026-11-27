// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The phone app's picture and sound of the project its API has open.
//!
//! [`crate::EditorApi`] edits; a [`Preview`] shows. It reads the session the
//! API holds rather than opening the project again - a second session on
//! the folder would fight the first for its lock and drift from its edits -
//! and the app calls [`Preview::refresh`] after each edit it sends. The
//! drawing is the spike's: a thread of its own owns the GPU device and the
//! Surface, draws the monitor's texture straight to it, and paces playback
//! from the audio device's clock.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use concat_api::Api;
use concat_host::playback::Playback;

use crate::{Clock, MobileError, Stats, audio_specs, render};

/// The picture and sound of one open project.
#[derive(uniffi::Object)]
pub struct Preview {
    api: Arc<Mutex<Api>>,
    path: String,
    playback: Arc<Playback>,
    renderer: render::Handle,
    /// Where the last clip ends, in seconds, as of the last refresh: read
    /// on every frame of the transport, so never by locking the API.
    duration_bits: AtomicU64,
}

impl Preview {
    /// A preview of `path`, which `api` must have open.
    pub(crate) fn open(api: Arc<Mutex<Api>>, path: String) -> Result<Arc<Preview>, MobileError> {
        {
            let api = lock(&api);
            api.open_session(&path)
                .map_err(|error| MobileError::Failed {
                    reason: error.message,
                })?;
        }
        let clock = Arc::new(Clock {
            position_bits: AtomicU64::new(0f64.to_bits()),
        });
        let playback = Playback::start(clock)?;
        let renderer = render::Handle::start(Arc::clone(&playback));
        let preview = Arc::new(Preview {
            api,
            path,
            playback,
            renderer,
            duration_bits: AtomicU64::new(0f64.to_bits()),
        });
        preview.refresh()?;
        Ok(preview)
    }
}

#[uniffi::export]
impl Preview {
    /// Reads the edit again from the API and hands it to playback and the
    /// picture: call it after every edit sent through the API.
    pub fn refresh(&self) -> Result<(), MobileError> {
        let api = lock(&self.api);
        let session = api
            .open_session(&self.path)
            .map_err(|error| MobileError::Failed {
                reason: error.message,
            })?;
        // The titles, captions and shapes too, which the flattening leaves
        // out: painted once at the timeline's size and held in memory.
        let mut clips = session.flattened_clips();
        clips.extend(
            api.title_pictures(&self.path)
                .map_err(|error| MobileError::Failed {
                    reason: error.message,
                })?,
        );
        let clips = Arc::new(clips);
        let specs = audio_specs(&clips);
        self.playback
            .set_clips(std::path::PathBuf::from(session.path()), specs);
        self.renderer.edit(clips, session.frame());
        let duration = session.document().showing_end().display_seconds();
        self.duration_bits
            .store(duration.to_bits(), Ordering::Relaxed);
        drop(api);
        // The frame at the playhead, as the edit now is.
        self.renderer.seek(self.playback.position_now());
        Ok(())
    }

    /// Where the last clip ends, in seconds, as of the last refresh.
    pub fn duration(&self) -> f64 {
        f64::from_bits(self.duration_bits.load(Ordering::Relaxed))
    }

    /// The playhead, in seconds: the audio device's clock while playing.
    pub fn position(&self) -> f64 {
        self.playback.position_now()
    }

    /// Whether playback is running.
    pub fn is_playing(&self) -> bool {
        self.playback.is_playing()
    }

    /// Starts playback from the playhead (from the start, at the end).
    pub fn play(&self) {
        let duration = self.duration();
        let mut at = self.playback.position_now();
        if at >= duration - 0.01 {
            at = 0.0;
        }
        self.playback.play(at);
        self.renderer.playing(true, duration);
    }

    /// Stops playback where it is.
    pub fn pause(&self) {
        self.playback.pause();
        self.renderer.playing(false, self.duration());
        self.renderer.seek(self.playback.position_now());
    }

    /// Moves the playhead and shows the frame there: a scrub.
    pub fn seek(&self, seconds: f64) {
        let seconds = seconds.clamp(0.0, self.duration().max(0.0));
        self.playback.seek(seconds);
        self.renderer.seek(seconds);
    }

    /// Takes the ANativeWindow of the preview's Surface (from
    /// `ConcatNative.windowFromSurface`, whose reference this now owns) and
    /// draws on it: `mode` "vulkan" (falling back to OpenGL ES), "gl" or
    /// "cpu". Which renderer took it.
    pub fn attach_surface(
        &self,
        window: u64,
        width: u32,
        height: u32,
        mode: String,
    ) -> Result<String, MobileError> {
        let described = self.renderer.attach(window, width, height, &mode)?;
        self.renderer.seek(self.playback.position_now());
        Ok(described)
    }

    /// Lets the Surface go; returns once nothing draws on it, as
    /// `surfaceDestroyed` requires.
    pub fn detach_surface(&self) {
        self.renderer.detach();
    }

    /// What the render thread measured.
    pub fn stats(&self) -> Stats {
        self.renderer.stats()
    }
}

/// The API, whatever an earlier panic left of its lock.
fn lock(api: &Mutex<Api>) -> std::sync::MutexGuard<'_, Api> {
    api.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
