// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Spike: the engine behind a native phone shell.
//!
//! A narrow surface over `concat-host`, exported through UniFFI so a
//! Kotlin + Jetpack Compose app can drive it: open a project in the app's
//! storage, import a video onto the timeline, show the frame at an instant
//! on an Android `Surface`, play and pause with sound, and say how long the
//! edit is and where the playhead stands.
//!
//! Nothing here decides what an edit means. Edits are `concat_project`
//! commands applied by the host's [`Session`]; pictures come from the
//! host's [`concat_host::preview::Monitor`]; sound and the clock from
//! [`concat_host::playback::Playback`]. What this crate adds is the thread
//! that owns the GPU device and the Surface ([`render`]), and the JNI the
//! Surface and the app's `Context` arrive through ([`jni`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use concat_host::Session;
use concat_host::playback::{ClipSpec, Playback, PlaybackEvents};
use concat_project::Command;

mod render;

#[cfg(target_os = "android")]
mod jni;

uniffi::setup_scaffolding!();

/// Everything that can go wrong, as one message the shell shows.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    /// The engine refused, with its reason. (Not `message`: Kotlin's
    /// Throwable already has one, and UniFFI's class would clash with it.)
    #[error("{reason}")]
    Failed {
        /// What happened, in words.
        reason: String,
    },
}

impl From<String> for MobileError {
    fn from(reason: String) -> Self {
        MobileError::Failed { reason }
    }
}

/// One clip on the timeline, as the strip draws it.
#[derive(Clone, Debug, uniffi::Record)]
pub struct ClipView {
    /// The clip's id in the edit.
    pub id: String,
    /// The bin item's name.
    pub name: String,
    /// Which lane, counted from the top.
    pub lane: u32,
    /// Timeline seconds where it starts.
    pub start: f64,
    /// Timeline seconds it lasts.
    pub duration: f64,
}

/// The timeline's frame and rate.
#[derive(Clone, Copy, Debug, uniffi::Record)]
pub struct VideoInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frames per second.
    pub fps: f64,
}

/// What the render thread measured, for the spike's numbers.
#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct Stats {
    /// "gpu: <adapter> (<backend>)" or "cpu", or "" before a surface.
    pub renderer: String,
    /// Frames presented since the surface was attached.
    pub frames: u64,
    /// Decode and placement of the last frame (frame_sources), ms.
    pub last_decode_ms: f64,
    /// Composite and present of the last frame, ms.
    pub last_draw_ms: f64,
    /// Frames presented per second over the last second of playback.
    pub play_fps: f64,
    /// Frames playback wanted and the render thread could not deliver in
    /// time (a late frame is skipped, not queued), since play.
    pub late_frames: u64,
    /// From a seek() call to its frame presented, the last one, ms.
    pub last_scrub_ms: f64,
    /// The same, averaged over every seek since the surface was attached.
    pub mean_scrub_ms: f64,
    /// The largest scrub latency seen, ms.
    pub max_scrub_ms: f64,
    /// Seeks presented (seeks superseded before drawing are not counted).
    pub scrubs: u64,
    /// The loudest sample the mixer played since play, linear (1.0 full
    /// scale): proof the sound reached the device, short of listening.
    pub audio_peak: f64,
    /// The last error the render thread hit, or "".
    pub last_error: String,
}

/// The clock playback reports, kept for the shell to read.
struct Clock {
    position_bits: AtomicU64,
}

impl PlaybackEvents for Clock {
    fn position(&self, seconds: f64) {
        self.position_bits
            .store(seconds.to_bits(), Ordering::Relaxed);
    }
    fn error(&self, message: String) {
        log::warn!("playback: {message}");
    }
}

/// The engine, as the shell holds it: one open project.
#[derive(uniffi::Object)]
pub struct Engine {
    session: Mutex<Session>,
    playback: Arc<Playback>,
    renderer: render::Handle,
}

#[uniffi::export]
impl Engine {
    /// Opens the spike's project under `storage_dir` (the app's files
    /// folder), creating it the first time. Also points the host's
    /// directories there, as the Slint activity does.
    #[uniffi::constructor]
    pub fn new(storage_dir: String) -> Result<Arc<Self>, MobileError> {
        let storage = std::path::PathBuf::from(&storage_dir);
        // SAFETY: the engine's threads do not exist yet; the JVM's do, and
        // none of them reads the C environment.
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", &storage);
            std::env::set_var("XDG_DATA_HOME", &storage);
            std::env::set_var("HOME", &storage);
        }
        // As the Slint window's settings pane does on open (panes/settings.rs):
        // decode on MediaCodec, a file it will not take in software.
        concat_media::set_hardware_decode(true);
        let projects = storage.join("projects");
        std::fs::create_dir_all(&projects)
            .map_err(|error| format!("could not create {}: {error}", projects.display()))?;
        let folder = projects.join(concat_host::projects::folder_name("Spike"));
        let info = if concat_host::projects::is_project(&folder) {
            concat_host::projects::open(&folder.to_string_lossy())?
        } else {
            concat_host::projects::create(&projects.to_string_lossy(), "Spike", 1080, 1920, 30, 1)?
        };
        let session = Session::open_info(&info)?;
        let clock = Arc::new(Clock {
            position_bits: AtomicU64::new(0f64.to_bits()),
        });
        let playback = Playback::start(clock)?;
        let renderer = render::Handle::start(Arc::clone(&playback));
        let engine = Arc::new(Engine {
            session: Mutex::new(session),
            playback,
            renderer,
        });
        engine.edited();
        log::info!("spike engine open on {}", info.path);
        Ok(engine)
    }

    /// The project folder.
    pub fn project_path(&self) -> String {
        self.session().path().to_owned()
    }

    /// Imports one file and puts it at the end of the timeline. The first
    /// video imported sets the timeline's frame and rate to its own.
    pub fn import_video(&self, path: String) -> Result<Vec<ClipView>, MobileError> {
        let started = std::time::Instant::now();
        let summary = concat_host::media::probe(&path)?;
        let probed_ms = started.elapsed().as_secs_f64() * 1000.0;
        {
            let mut session = self.session();
            let first = session.project().active().clips.is_empty();
            let added = session.apply(Command::AddMedia {
                item: summary.to_new_media(),
            })?;
            let media_id = match added.created_id {
                Some(id) => id,
                None => session
                    .project()
                    .media
                    .iter()
                    .find(|item| item.path == path)
                    .map(|item| item.id.clone())
                    .ok_or_else(|| "the bin did not take the file".to_owned())?,
            };
            if first && let Some(video) = summary.video.as_ref() {
                let (rate_num, rate_den) = rate_of(&video.frame_rate_fraction);
                let mut settings = session.video();
                settings.width = video.width;
                settings.height = video.height;
                settings.rate_num = rate_num;
                settings.rate_den = rate_den;
                session.set_video(settings)?;
            }
            let end = session.project().active().duration();
            session.apply(Command::AddClipAtFirstFree {
                media_id,
                start: end,
            })?;
            session.save(None)?;
        }
        log::info!(
            "imported {path}: probed in {probed_ms:.1} ms, placed in {:.1} ms",
            started.elapsed().as_secs_f64() * 1000.0
        );
        self.edited();
        Ok(self.clips())
    }

    /// The clips on the timeline, in time order.
    pub fn clips(&self) -> Vec<ClipView> {
        let session = self.session();
        let timeline = session.project().active();
        let mut clips: Vec<ClipView> = timeline
            .clips
            .iter()
            .map(|clip| ClipView {
                id: clip.id.clone(),
                name: clip.name.clone(),
                lane: timeline
                    .tracks
                    .iter()
                    .position(|track| track.id == clip.track_id)
                    .unwrap_or(0) as u32,
                start: clip.start,
                duration: clip.duration,
            })
            .collect();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        clips
    }

    /// The timeline's frame and rate.
    pub fn video(&self) -> VideoInfo {
        let video = self.session().video();
        VideoInfo {
            width: video.width,
            height: video.height,
            fps: video.rate_num as f64 / video.rate_den.max(1) as f64,
        }
    }

    /// Where the last clip ends, in seconds.
    pub fn duration(&self) -> f64 {
        self.session().project().active().duration()
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
    /// draws on it. `mode` is "vulkan" (wgpu on Vulkan, falling back to
    /// OpenGL ES), "gl" (wgpu on OpenGL ES) or "cpu" (the CPU compositor
    /// into the window's locked buffer). Returns which renderer took it.
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

    /// Lets the Surface go. Returns once nothing draws on it any more, as
    /// `surfaceDestroyed` requires.
    pub fn detach_surface(&self) {
        self.renderer.detach();
    }

    /// What the render thread measured.
    pub fn stats(&self) -> Stats {
        self.renderer.stats()
    }

    /// Clears the measured numbers, for a fresh run.
    pub fn reset_stats(&self) {
        self.renderer.reset_stats();
    }
}

impl Engine {
    fn session(&self) -> std::sync::MutexGuard<'_, Session> {
        self.session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Hands the new edit to playback and the monitor.
    fn edited(&self) {
        let session = self.session();
        let clips = Arc::new(session.flattened_clips());
        let specs = audio_specs(&clips);
        self.playback
            .set_clips(std::path::PathBuf::from(session.path()), specs);
        let video = session.video();
        self.renderer
            .edit(clips, session.settings(), video.color_space);
    }
}

/// "30000/1001" as a fraction, 30/1 for anything unreadable.
fn rate_of(fraction: &str) -> (i64, i64) {
    let mut parts = fraction.split('/');
    let num = parts
        .next()
        .and_then(|part| part.trim().parse::<i64>().ok());
    let den = parts
        .next()
        .map_or(Some(1), |part| part.trim().parse::<i64>().ok());
    match (num, den) {
        (Some(num), Some(den)) if num > 0 && den > 0 => (num, den),
        _ => (30, 1),
    }
}

/// The audible clips of a flattened edit, as playback takes them - the
/// host's one definition, which the desktop window plays too.
fn audio_specs(clips: &[concat_export::ExportClip]) -> Vec<ClipSpec> {
    concat_host::playback::audible(clips)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_rate_reads_as_a_fraction() {
        assert_eq!(super::rate_of("30000/1001"), (30000, 1001));
        assert_eq!(super::rate_of("25"), (25, 1));
        assert_eq!(super::rate_of("x"), (30, 1));
    }
}
