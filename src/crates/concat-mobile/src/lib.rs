// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Spike: the engine behind a native phone shell.
//!
//! A narrow surface over `concat-host`, exported through UniFFI so a
//! Kotlin + Jetpack Compose app can drive it: open a project in the app's
//! storage, import a video onto the timeline, show the frame at an instant
//! on an Android `Surface`, play and pause with sound, and say how long the
//! edit is and where the playhead stands.
//!
//! Nothing here decides what an edit means. Edits are `concat_doc`
//! commands applied by the host's [`Session`]; pictures come from the
//! host's [`concat_host::preview::Monitor`]; sound and the clock from
//! [`concat_host::playback::Playback`]. What this crate adds is the thread
//! that owns the GPU device and the Surface ([`render`]), and the JNI the
//! Surface and the app's `Context` arrive through ([`jni`]).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use concat_doc::commands::Command;
use concat_doc::model::{Frame, LaneId, Media, MediaId, Model, SequenceId};
use concat_doc::{Rate, TickDur};
use concat_host::Session;
use concat_host::playback::{ClipSpec, Playback, PlaybackEvents};

mod api;
mod preview;
mod render;
mod view;

pub use api::{ApiEvents, EditorApi};
pub use preview::Preview;

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
    /// How many times the preview moved a rung down - Vulkan to OpenGL ES
    /// to the CPU - because its device was lost or kept failing. Above
    /// nought, the preview is drawing the slower way and the app says so.
    pub fallbacks: u64,
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
            concat_host::projects::create(
                &projects.to_string_lossy(),
                "Spike",
                1080,
                1920,
                30,
                1,
                [0, 0, 0, 255],
            )?
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
            place_at_end(&mut session, summary.to_new_media())?;
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
        clips_of(&session.document(), session.showing())
    }

    /// The timeline's frame and rate.
    pub fn video(&self) -> VideoInfo {
        let frame = self.session().frame();
        VideoInfo {
            width: frame.width.get(),
            height: frame.height.get(),
            fps: frame.rate.display_per_second(),
        }
    }

    /// Where the last clip ends, in seconds.
    pub fn duration(&self) -> f64 {
        self.session().document().showing_end().display_seconds()
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
        self.renderer.edit(clips, session.frame());
    }
}

/// Adds a probed file to the bin and puts it at the end of the timeline
/// showing. The first file placed there that has a picture gives the
/// timeline its frame; see [`framed_by`]. A file already in the bin is
/// cut from again, not added a second time.
fn place_at_end(session: &mut Session, media: Media) -> Result<(), String> {
    let sequence = session
        .showing()
        .ok_or_else(|| "the project has no timeline to put the file on".to_owned())?;
    let document = session.document();
    let first = document
        .lanes
        .index()
        .of(sequence)
        .all(|(lane, _)| document.items.index().on_lane(lane).next().is_none());
    let end = document.end_of(sequence);
    let frame = if first {
        framed_by(&media, session.frame())
    } else {
        None
    };
    let added = session.apply(&Command::AddMedia { media })?;
    let media: MediaId = added
        .applied
        .created
        .and_then(|id| id.typed())
        .ok_or_else(|| "the bin did not take the file".to_owned())?;
    if let Some(frame) = frame {
        session.apply(&Command::SetTimelineVideo { sequence, frame })?;
    }
    session.apply(&Command::AddClipAtFirstFree { media, start: end })?;
    Ok(())
}

/// `frame` with the size and rate of a file's picture, 30 a second when
/// the probe found no rate: what the first file on a timeline makes of
/// it. None for a file with no picture.
fn framed_by(media: &Media, frame: Frame) -> Option<Frame> {
    let size = media.size?;
    Some(Frame {
        width: size.width,
        height: size.height,
        rate: media.rate.unwrap_or(Rate::FPS_30),
        ..frame
    })
}

/// The clips of the sequence showing, in time order, each with its lane
/// counted from the top of the stack.
fn clips_of(document: &Model, showing: Option<SequenceId>) -> Vec<ClipView> {
    let Some(sequence) = showing else {
        return Vec::new();
    };
    let mut lanes: Vec<LaneId> = document
        .lanes
        .index()
        .of(sequence)
        .map(|(lane, _)| lane)
        .collect();
    // The index walks the stack from the bottom.
    lanes.reverse();
    let mut clips = Vec::new();
    for (from_top, lane) in lanes.into_iter().enumerate() {
        for (id, start, _) in document.items.index().on_lane(lane) {
            let Some(item) = document.items.get(id) else {
                continue;
            };
            clips.push(ClipView {
                id: id.to_string(),
                name: item.name.clone(),
                lane: u32::try_from(from_top).unwrap_or(u32::MAX),
                start: start.display_seconds(),
                duration: TickDur::from(item.len).display_seconds(),
            });
        }
    }
    clips.sort_by(|a, b| a.start.total_cmp(&b.start));
    clips
}

/// The audible clips of a flattened edit, as playback takes them - the
/// host's one definition, which the desktop window plays too.
fn audio_specs(clips: &[concat_export::ExportClip]) -> Vec<ClipSpec> {
    concat_host::playback::audible(clips)
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_doc::model::{MediaKind, Size};
    use concat_doc::{Tick, TickLen};
    use concat_host::media::{FileKind, MediaSummary, VideoStreamInfo};
    use concat_host::session::ProjectSettings;

    /// A session on a project never saved: one timeline, 1920 by 1080 at
    /// 30 a second, in a folder that is not there.
    fn session() -> Session {
        let nowhere = std::env::temp_dir().join(format!("concat-mobile-{}", std::process::id()));
        let settings = ProjectSettings {
            name: "Spike".to_owned(),
            frame: Frame::hd(),
        };
        Session::open(&nowhere.join("unsaved").to_string_lossy(), settings).expect("opens")
    }

    /// A probed video of four seconds with a picture of this size and rate.
    fn video(path: &str, width: u32, height: u32, rate: Option<Rate>) -> Media {
        Media {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            kind: MediaKind::Video,
            length: TickLen::try_from(Tick::from_whole_seconds(4).expect("a place").get()).ok(),
            rate,
            size: Size::new(width, height),
            ..Media::default()
        }
    }

    /// What the probe of a video hands the bin, its rate written this way.
    fn probed(rate: &str) -> Media {
        MediaSummary {
            path: "/footage/a.mp4".to_owned(),
            duration: Some(4.0),
            kind: FileKind::Video,
            video: Some(VideoStreamInfo {
                index: 0,
                codec: "h264".to_owned(),
                width: 1280,
                height: 720,
                frame_rate: 0.0,
                frame_rate_fraction: rate.to_owned(),
                color_range: None,
                color_space: Default::default(),
            }),
            audio: None,
            audio_tracks: Vec::new(),
        }
        .to_new_media()
    }

    #[test]
    fn a_rate_reads_as_a_fraction() {
        let rate = |written: &str| {
            framed_by(&probed(written), Frame::hd())
                .expect("it has a picture")
                .rate
        };
        assert_eq!(Ok(rate("30000/1001")), Rate::new(30000, 1001));
        assert_eq!(Ok(rate("25/1")), Rate::new(25, 1));
        assert_eq!(rate("x"), Rate::FPS_30);
    }

    #[test]
    fn the_first_file_with_a_picture_gives_the_timeline_its_size_and_rate() {
        let mut session = session();
        let ntsc = Rate::new(30000, 1001).ok();
        place_at_end(&mut session, video("/footage/a.mp4", 1280, 720, ntsc)).expect("placed");
        let frame = session.frame();
        assert_eq!((frame.width.get(), frame.height.get()), (1280, 720));
        assert_eq!(Some(frame.rate), ntsc);

        let pal = Rate::new(25, 1).ok();
        place_at_end(&mut session, video("/footage/b.mp4", 640, 360, pal)).expect("placed");
        assert_eq!(session.frame(), frame, "only the first sets it");
    }

    #[test]
    fn a_file_with_no_picture_leaves_the_frame_as_it_was() {
        let mut session = session();
        let song = Media {
            kind: MediaKind::Audio,
            size: None,
            rate: None,
            has_sound: true,
            ..video("/footage/song.wav", 1, 1, None)
        };
        place_at_end(&mut session, song).expect("placed");
        assert_eq!(session.frame(), Frame::hd());
        assert_eq!(clips_of(&session.document(), session.showing()).len(), 1);
    }

    #[test]
    fn a_file_already_in_the_bin_is_not_added_twice() {
        let mut session = session();
        for _ in 0..2 {
            place_at_end(&mut session, video("/footage/a.mp4", 1280, 720, None)).expect("placed");
        }
        let document = session.document();
        assert_eq!(document.media.len(), 1);
        let clips = clips_of(&document, session.showing());
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[1].start, clips[0].start + clips[0].duration);
        assert_eq!(document.showing_end().display_seconds(), 8.0);
    }

    #[test]
    fn clips_list_in_time_order_with_the_lane_counted_from_the_top() {
        let mut session = session();
        for _ in 0..2 {
            place_at_end(&mut session, video("/footage/a.mp4", 1280, 720, None)).expect("placed");
        }
        // Over the first clip, so it goes one lane up.
        let media = session.document().media.ids().next().expect("the file");
        let start = Tick::from_whole_seconds(1).expect("a place");
        session
            .apply(&Command::AddClipAtFirstFree { media, start })
            .expect("placed");

        let document = session.document();
        let showing = session.showing().expect("a timeline shows");
        let bottom = u32::try_from(document.lanes.index().of(showing).count()).expect("few") - 1;
        let clips = clips_of(&document, session.showing());
        let listed: Vec<(f64, u32)> = clips.iter().map(|clip| (clip.start, clip.lane)).collect();
        assert_eq!(listed, [(0.0, bottom), (1.0, bottom - 1), (4.0, bottom)]);
        assert!(clips.iter().all(|clip| clip.name == "a.mp4"));
    }
}
