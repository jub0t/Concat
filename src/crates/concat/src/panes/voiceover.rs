// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The voiceover: words recorded from the microphone over the picture,
//! and laid on the timeline where they were spoken.
//!
//! The tray's microphone opens a panel that floats over the editor - a
//! sheet at the foot of a phone's screen - and stays up while the cut is
//! worked on around it. The panel opens the microphone at once, so its
//! meter shows the room before a word is said: which microphone, how loud,
//! whether it clips. Record counts three in, then rolls the picture from
//! the playhead with the timeline's own sound held silent, unless asked
//! otherwise, so the speakers do not bleed into the take. Pause holds the
//! picture and the take together, and Resume carries both on from there:
//! one take is one clip, its pauses cut out, because the picture paused
//! with them. Stop lands the take on a lane under the picture, at the
//! playhead plus the moment the microphone actually began; the next take
//! goes on the same lane when it is free there. Retake throws the last
//! take away - the one running, or the one just landed - and goes again
//! from where it began.
//!
//! The file goes into the project's audio folder and is whole on disk as
//! it is written (see concat_host::record). Closing the panel mid-take
//! stops it and keeps it.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use concat_host::media;
use concat_host::record::{self, Microphone};
use concat_project::Command;
use slint::ComponentHandle;

use crate::host::{on_ui, spawn_in_project};
use crate::i18n::{t, tf};
use crate::panes::Msg;
use crate::studio::Studio;
use crate::ui::{Editor, VoiceoverData};

/// Everything that can happen to the panel and its takes.
pub enum VoiceoverMsg {
    /// The tray's button: open the panel, or stop the take it is running.
    Toggle,
    /// The panel's close button. A take running stops and is kept.
    Close,
    /// The microphone is open, with the list it was chosen from, or why
    /// it is not. The number is the opening it answers; see `opening`.
    Opened(u32, Box<Result<(Microphone, Vec<String>), String>>),
    /// Record: a take from the playhead, counted in or not.
    Record,
    /// A second of the count-in has passed.
    Tick,
    Pause,
    Resume,
    Stop,
    /// Throw the last take away and go again from where it began.
    Retake,
    /// Open the microphone again, after it would not open.
    Retry,
    /// The device failed after it opened: unplugged, taken by another app.
    Lost(String),
    /// A microphone from the list, by its place in it.
    Device(i32),
    Countdown(bool),
    Mute(bool),
    /// The input gain, in decibels.
    Gain(f32),
    /// The teleprompter's script, as typed.
    Prompt(String),
    PromptSpeed(i32),
    /// The take is written and probed: onto the timeline with it.
    Finished(Box<Landed>),
}

/// A finished take on its way to the timeline.
pub struct Landed {
    result: Result<media::MediaSummary, String>,
    /// Where on the timeline its first sample belongs.
    landing: f64,
    /// Its length, in seconds.
    seconds: f64,
    /// Where the playhead was when it began: where a retake goes back to.
    start: f64,
}

impl std::fmt::Debug for VoiceoverMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Toggle => write!(f, "Toggle"),
            Self::Close => write!(f, "Close"),
            Self::Opened(token, result) => match result.as_ref() {
                Ok((mic, _)) => write!(f, "Opened({token}, {})", mic.name()),
                Err(error) => write!(f, "Opened({token}, error {error:?})"),
            },
            Self::Record => write!(f, "Record"),
            Self::Tick => write!(f, "Tick"),
            Self::Pause => write!(f, "Pause"),
            Self::Resume => write!(f, "Resume"),
            Self::Stop => write!(f, "Stop"),
            Self::Retake => write!(f, "Retake"),
            Self::Retry => write!(f, "Retry"),
            Self::Lost(error) => write!(f, "Lost({error:?})"),
            Self::Device(index) => write!(f, "Device({index})"),
            Self::Countdown(on) => write!(f, "Countdown({on})"),
            Self::Mute(on) => write!(f, "Mute({on})"),
            Self::Gain(db) => write!(f, "Gain({db})"),
            Self::Prompt(text) => write!(f, "Prompt({} chars)", text.chars().count()),
            Self::PromptSpeed(index) => write!(f, "PromptSpeed({index})"),
            Self::Finished(landed) => match &landed.result {
                Ok(summary) => write!(f, "Finished({})", summary.path),
                Err(error) => write!(f, "Finished(error {error:?})"),
            },
        }
    }
}

/// Where a take is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    #[default]
    Ready,
    /// Counting in, this many seconds to go.
    Counting(u8),
    Recording,
    Paused,
}

/// What a transport verb from anywhere but the panel - Space, the
/// monitor's Play, the keymap's play, a click on the ruler - becomes while
/// a take is under way. The picture and the take are one run: a verb that
/// moved one without the other would leave the words out of step with
/// the frames they were spoken to, so it moves both or neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    /// No take: the verb does what it always does.
    Free,
    /// Pause the take, and the picture with it.
    Pause,
    /// Carry the take on, and the picture with it.
    Resume,
    /// Call off the count-in.
    CancelCount,
    /// Not now: the playhead belongs to the take until it stops.
    Refuse,
}

impl Phase {
    /// Play or pause, toggled.
    fn on_play(self) -> Hold {
        match self {
            Phase::Ready => Hold::Free,
            Phase::Counting(_) => Hold::CancelCount,
            Phase::Recording => Hold::Pause,
            Phase::Paused => Hold::Resume,
        }
    }

    /// The picture stopped from elsewhere - before a seek, a step, an
    /// edit. At the end of the cut it stops by itself and the take records
    /// on, with nothing more to be out of step with.
    fn on_pause(self, at_end: bool) -> Hold {
        match self {
            Phase::Recording if !at_end => Hold::Pause,
            _ => Hold::Free,
        }
    }

    /// The playhead moved from elsewhere. A paused take resumes from where
    /// it was held, so moving the playhead meanwhile would only be undone.
    fn on_seek(self) -> Hold {
        match self {
            Phase::Recording | Phase::Paused => Hold::Refuse,
            _ => Hold::Free,
        }
    }
}

impl Hold {
    fn msg(self) -> Option<VoiceoverMsg> {
        match self {
            Hold::Free | Hold::Refuse => None,
            Hold::Pause => Some(VoiceoverMsg::Pause),
            Hold::Resume => Some(VoiceoverMsg::Resume),
            Hold::CancelCount => Some(VoiceoverMsg::Stop),
        }
    }
}

/// The take being recorded.
struct Running {
    path: PathBuf,
    /// Where the playhead was when it began.
    start: f64,
    /// When the picture started rolling under it.
    rolled: Instant,
    /// Where the playhead was when it was paused: where the picture
    /// carries on from, wherever the playhead was moved meanwhile, so the
    /// take and the picture stay one continuous run.
    held: f32,
}

/// The take that landed last, for Retake.
struct Last {
    media_id: String,
    start: f64,
}

/// The panel, its microphone and its take.
#[derive(Default)]
pub struct VoiceoverPane {
    open: bool,
    mic: Option<Microphone>,
    /// When the microphone opened: a microphone that has said nothing but
    /// silence for a while is one the system is keeping from the app.
    opened: Option<Instant>,
    /// Counts openings, so the answer to one the panel has since moved on
    /// from - closed, another microphone picked - is let go.
    opening: u32,
    /// An opening is on its way.
    waiting: bool,
    devices: Vec<String>,
    /// Which of `devices` is open.
    device: usize,
    /// Why there is no microphone, or what last went wrong.
    error: Option<String>,
    phase: Phase,
    take: Option<Running>,
    last: Option<Last>,
    /// The lane the last take went on: the next goes there too when it
    /// can, so a voiceover reads as one line under the picture.
    lane: Option<String>,
    prompt: String,
    /// Feeds the meter while the panel is open.
    meter: Option<slint::Timer>,
    /// Beats the count-in.
    countdown: Option<slint::Timer>,
    /// A seek this take refused has been explained already: a drag along
    /// the ruler asks dozens of times, and once is enough to be told.
    told: bool,
}

impl VoiceoverPane {
    /// Whether a take is under way, counted in or paused: the tray's
    /// button reads Stop.
    /// Whether the panel is up, for the tray button that opens and closes it.
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn recording(&self) -> bool {
        self.phase != Phase::Ready
    }

    /// What Play becomes while a take is under way: the panel's own verb,
    /// or None to play as usual. See [`Hold`].
    pub fn on_play(&self) -> Option<VoiceoverMsg> {
        self.phase.on_play().msg()
    }

    /// What stopping the picture becomes; `at_end` is the playhead at the
    /// end of the cut.
    pub fn on_pause(&self, at_end: bool) -> Option<VoiceoverMsg> {
        self.phase.on_pause(at_end).msg()
    }

    /// Whether a seek is refused now, and whether to say why: Some(true)
    /// the first time in a take, Some(false) after.
    pub fn on_seek(&mut self) -> Option<bool> {
        if self.phase.on_seek() != Hold::Refuse {
            return None;
        }
        Some(!std::mem::replace(&mut self.told, true))
    }

    /// Applies one message.
    pub fn update(&mut self, msg: VoiceoverMsg, studio: &mut Studio) {
        match msg {
            VoiceoverMsg::Toggle => match self.phase {
                Phase::Counting(_) => self.cancel_count(),
                Phase::Recording | Phase::Paused => self.stop(studio),
                Phase::Ready if self.open => self.close(studio),
                Phase::Ready => self.open(studio),
            },
            VoiceoverMsg::Close => self.close(studio),
            VoiceoverMsg::Opened(token, result) => {
                self.opened(token, *result);
                self.apply_gain(studio);
            }
            VoiceoverMsg::Record => self.record(studio),
            VoiceoverMsg::Tick => match self.phase {
                Phase::Counting(left) if left > 1 => self.phase = Phase::Counting(left - 1),
                Phase::Counting(_) => {
                    self.countdown = None;
                    self.begin(studio);
                }
                _ => self.countdown = None,
            },
            VoiceoverMsg::Pause => self.pause(studio),
            VoiceoverMsg::Resume => self.resume(studio),
            VoiceoverMsg::Stop => match self.phase {
                Phase::Counting(_) => self.cancel_count(),
                _ => self.stop(studio),
            },
            VoiceoverMsg::Retake => self.retake(studio),
            VoiceoverMsg::Retry => self.reopen(studio),
            VoiceoverMsg::Lost(error) => {
                log::warn!("voiceover: the microphone was lost: {error}");
                self.stop(studio);
                self.mic = None;
                self.error = Some(error);
            }
            VoiceoverMsg::Device(index) => {
                let Some((index, name)) = usize::try_from(index)
                    .ok()
                    .and_then(|index| Some((index, self.devices.get(index)?)))
                else {
                    return;
                };
                if self.phase != Phase::Ready || (self.mic.is_some() && self.device == index) {
                    return;
                }
                studio.prefs.voiceover.device = Some(name.clone());
                studio.prefs.save(&studio.host.dirs);
                self.reopen(studio);
            }
            VoiceoverMsg::Countdown(on) => {
                studio.prefs.voiceover.countdown = on;
                studio.prefs.save(&studio.host.dirs);
            }
            VoiceoverMsg::Mute(on) => {
                studio.prefs.voiceover.mute = on;
                studio.prefs.save(&studio.host.dirs);
                if matches!(self.phase, Phase::Recording | Phase::Paused) {
                    studio.host.playback.set_muted(on);
                }
            }
            VoiceoverMsg::Gain(db) => {
                let db = db.clamp(GAIN_FLOOR, GAIN_CEILING);
                studio.prefs.voiceover.gain_db = db;
                if let Some(mic) = self.mic.as_ref() {
                    mic.set_gain(record::gain_of(db));
                }
                studio.prefs.save(&studio.host.dirs);
            }
            VoiceoverMsg::Prompt(text) => self.prompt = text,
            VoiceoverMsg::PromptSpeed(index) => {
                studio.prefs.voiceover.prompt_speed = index.clamp(0, 2) as u8;
                studio.prefs.save(&studio.host.dirs);
            }
            VoiceoverMsg::Finished(landed) => self.land(*landed, studio),
        }
    }

    /// What the panel shows.
    pub fn data(&self, studio: &Studio) -> VoiceoverData {
        let prefs = &studio.prefs.voiceover;
        let (phase, count) = match self.phase {
            Phase::Ready => (0, 0),
            Phase::Counting(left) => (1, i32::from(left)),
            Phase::Recording => (2, 0),
            Phase::Paused => (3, 0),
        };
        VoiceoverData {
            open: self.open,
            phase,
            count,
            waiting: self.waiting,
            ready: self.mic.is_some(),
            device: self.device as i32,
            countdown: prefs.countdown,
            mute: prefs.mute,
            gain: prefs.gain_db,
            error: self.error.clone().unwrap_or_default().into(),
            can_retake: self.can_retake(studio),
            prompt: self.prompt.clone().into(),
            prompt_speed: i32::from(prefs.prompt_speed),
        }
    }

    /// The microphones the panel lists.
    pub fn device_rows(&self) -> Vec<slint::SharedString> {
        self.devices.iter().map(|name| name.into()).collect()
    }

    fn can_retake(&self, studio: &Studio) -> bool {
        match self.phase {
            Phase::Recording | Phase::Paused => true,
            Phase::Counting(_) => false,
            Phase::Ready => self.last.as_ref().is_some_and(|last| {
                studio
                    .project()
                    .media
                    .iter()
                    .any(|item| item.id == last.media_id)
            }),
        }
    }

    fn open(&mut self, studio: &mut Studio) {
        if studio.session.is_none() {
            return;
        }
        self.open = true;
        self.reopen(studio);
    }

    /// Opens the microphone the preferences name, off the window's thread:
    /// listing and opening devices can take a moment, and the panel is up
    /// meanwhile, saying so.
    fn reopen(&mut self, studio: &mut Studio) {
        self.mic = None;
        self.meter = None;
        self.opened = None;
        self.error = None;
        // An iPhone records only with its audio session set to play and
        // record together; left as it is, the microphone hands over
        // silence. Set before the device opens, and a failure said
        // plainly rather than a take of nothing.
        #[cfg(target_os = "ios")]
        if let Err(error) = crate::ios::record_alongside_playback() {
            log::warn!("voiceover: the audio session would not record: {error}");
            self.error = Some(tf("voiceover.failed", &[&error]));
            return;
        }
        if !crate::platform::microphone_allowed() {
            self.error = Some(t("voiceover.permission"));
            return;
        }
        self.opening = self.opening.wrapping_add(1);
        self.waiting = true;
        let token = self.opening;
        let wanted = studio.prefs.voiceover.device.clone();
        spawn_in_project(
            move || {
                let devices = record::microphones();
                Microphone::open(wanted.as_deref()).map(|mic| (mic, devices))
            },
            move |studio, _, _, result| {
                studio.handle(Msg::Voiceover(VoiceoverMsg::Opened(
                    token,
                    Box::new(result),
                )));
            },
        );
    }

    fn opened(&mut self, token: u32, result: Result<(Microphone, Vec<String>), String>) {
        if token != self.opening || !self.open {
            // Dropped here: the device closes again.
            return;
        }
        self.waiting = false;
        match result {
            Ok((mic, mut devices)) => {
                log::info!("voiceover: listening to {}", mic.name());
                // The one that opened is on the list whatever the list
                // says: some systems do not list their default.
                let device = match devices.iter().position(|name| name == mic.name()) {
                    Some(index) => index,
                    None => {
                        devices.insert(0, mic.name().to_owned());
                        0
                    }
                };
                self.devices = devices;
                self.device = device;
                self.mic = Some(mic);
                self.opened = Some(Instant::now());
                self.meter = Some(meter());
                self.error = None;
            }
            Err(error) => {
                log::warn!("voiceover: the microphone would not open: {error}");
                self.error = Some(tf("voiceover.failed", &[&error]));
            }
        }
    }

    /// Applies the gain the preferences hold to the open microphone.
    fn apply_gain(&self, studio: &Studio) {
        if let Some(mic) = self.mic.as_ref() {
            mic.set_gain(record::gain_of(studio.prefs.voiceover.gain_db));
        }
    }

    fn close(&mut self, studio: &mut Studio) {
        self.cancel_count();
        self.stop(studio);
        self.open = false;
        self.waiting = false;
        self.mic = None;
        self.meter = None;
        self.opened = None;
        self.error = None;
        self.last = None;
    }

    fn record(&mut self, studio: &mut Studio) {
        if self.phase != Phase::Ready || self.mic.is_none() || studio.session.is_none() {
            return;
        }
        if studio.prefs.voiceover.countdown {
            self.phase = Phase::Counting(COUNT_IN);
            let timer = slint::Timer::default();
            timer.start(slint::TimerMode::Repeated, Duration::from_secs(1), || {
                on_ui(|studio, _, _| studio.handle(Msg::Voiceover(VoiceoverMsg::Tick)))
            });
            self.countdown = Some(timer);
        } else {
            self.begin(studio);
        }
    }

    fn cancel_count(&mut self) {
        self.countdown = None;
        if matches!(self.phase, Phase::Counting(_)) {
            self.phase = Phase::Ready;
        }
    }

    /// Starts writing and rolls the picture from the playhead.
    fn begin(&mut self, studio: &mut Studio) {
        self.phase = Phase::Ready;
        let Some(mic) = self.mic.as_ref() else {
            return;
        };
        let Some(project) = studio
            .session
            .as_ref()
            .map(|session| PathBuf::from(session.path()))
        else {
            return;
        };
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or_default();
        let path = project.join("audio").join(format!("voiceover-{stamp}.wav"));
        self.apply_gain(studio);
        if let Err(error) = mic.begin(path.clone()) {
            self.error = Some(tf("voiceover.failed", &[&error]));
            return;
        }
        log::info!("voiceover: recording from {:.3}s", studio.playhead);
        self.error = None;
        self.told = false;
        let start = f64::from(studio.playhead.max(0.0));
        // The picture rolls under the words, without its sound unless the
        // panel says otherwise. At the end of the cut there is nothing to
        // roll, and the take just records.
        studio.host.playback.set_muted(studio.prefs.voiceover.mute);
        if !studio.playing && studio.playhead < studio.duration() {
            studio.play_toggle();
        }
        self.take = Some(Running {
            path,
            start,
            rolled: Instant::now(),
            held: studio.playhead,
        });
        self.phase = Phase::Recording;
    }

    fn pause(&mut self, studio: &mut Studio) {
        if self.phase != Phase::Recording {
            return;
        }
        if let Some(mic) = self.mic.as_ref() {
            mic.pause();
        }
        if studio.playing {
            studio.pause();
        }
        if let Some(take) = self.take.as_mut() {
            take.held = studio.playhead;
        }
        self.phase = Phase::Paused;
    }

    fn resume(&mut self, studio: &mut Studio) {
        if self.phase != Phase::Paused {
            return;
        }
        let Some(mic) = self.mic.as_ref() else {
            return;
        };
        if let Some(held) = self.take.as_ref().map(|take| take.held)
            && (studio.playhead - held).abs() > 1e-4
        {
            studio.seek(held);
        }
        mic.resume();
        if !studio.playing && studio.playhead < studio.duration() {
            studio.play_toggle();
        }
        self.phase = Phase::Recording;
    }

    /// Stops the take and the picture, and sends the file to be probed.
    fn stop(&mut self, studio: &mut Studio) {
        self.cancel_count();
        let Some(take) = self.take.take() else {
            self.phase = Phase::Ready;
            return;
        };
        self.phase = Phase::Ready;
        if studio.playing {
            studio.pause();
        }
        studio.host.playback.set_muted(false);
        let Some(mic) = self.mic.as_ref() else {
            return;
        };
        match mic.end() {
            Ok(done) => {
                log::info!("voiceover: {:.2}s to {}", done.seconds, done.path.display());
                let landing = record::landing(take.start, take.rolled, done.first);
                let seconds = done.seconds;
                let start = take.start;
                spawn_in_project(
                    move || media::probe(&done.path.to_string_lossy()),
                    move |studio, _, _, result| {
                        studio.handle(Msg::Voiceover(VoiceoverMsg::Finished(Box::new(Landed {
                            result,
                            landing,
                            seconds,
                            start,
                        }))));
                    },
                );
            }
            Err(error) => {
                let _ = std::fs::remove_file(&take.path);
                studio.notify(&tf("voiceover.failed", &[&error]), true);
            }
        }
    }

    /// Throws the last take away and goes again from where it began: the
    /// one running, its file deleted, or the one that landed last, taken
    /// off the timeline and out of the bin as one step that undo brings
    /// back - so its file stays on disk.
    fn retake(&mut self, studio: &mut Studio) {
        let start = match self.phase {
            Phase::Counting(_) => return,
            Phase::Recording | Phase::Paused => {
                let Some(take) = self.take.take() else {
                    return;
                };
                self.phase = Phase::Ready;
                if studio.playing {
                    studio.pause();
                }
                // Heard again until the next take begins: a count-in
                // cancelled from here must not leave the timeline silent.
                studio.host.playback.set_muted(false);
                if let Some(mic) = self.mic.as_ref() {
                    let _ = mic.end();
                }
                let _ = std::fs::remove_file(&take.path);
                take.start
            }
            Phase::Ready => {
                if !self.can_retake(studio) {
                    return;
                }
                let Some(last) = self.last.take() else {
                    return;
                };
                studio.apply(Command::RemoveMedia {
                    media_id: last.media_id,
                });
                last.start
            }
        };
        log::info!("voiceover: retake from {start:.3}s");
        studio.seek(start as f32);
        self.record(studio);
    }

    /// Puts a probed take into the bin and on the timeline.
    fn land(&mut self, landed: Landed, studio: &mut Studio) {
        let summary = match landed.result {
            Ok(summary) => summary,
            Err(error) => {
                studio.notify(&tf("voiceover.failed", &[&error]), true);
                return;
            }
        };
        // The user's own media, like an import: shelved under Audio, not
        // under Generated.
        let created = studio.apply(Command::AddMedia {
            item: summary.to_new_media(),
        });
        let Some(media_id) = created.or_else(|| {
            studio
                .project()
                .media
                .iter()
                .find(|item| item.path == summary.path)
                .map(|item| item.id.clone())
        }) else {
            return;
        };
        // The lane the last take went on, when it is free over this one.
        let lane = self.lane.clone().filter(|lane| {
            let timeline = studio.project().active();
            timeline.tracks.iter().any(|track| track.id == *lane)
                && timeline.is_free(lane, landed.landing, landed.seconds, &[])
        });
        let clip = match lane {
            Some(track_id) => studio.apply(Command::AddClip {
                media_id: media_id.clone(),
                track_id,
                start: landed.landing,
                ripple: false,
            }),
            None => studio.apply(Command::AddClipAtFirstFree {
                media_id: media_id.clone(),
                start: landed.landing,
            }),
        };
        if let Some(clip) = clip {
            self.lane = studio
                .project()
                .active()
                .clips
                .iter()
                .find(|placed| placed.id == clip)
                .map(|placed| placed.track_id.clone());
            self.last = Some(Last {
                media_id,
                start: landed.start,
            });
            studio.notify(&t("voiceover.added"), false);
        }
    }
}

/// Seconds counted in before a take.
const COUNT_IN: u8 = 3;

/// The input gain's range, in decibels.
const GAIN_FLOOR: f32 = -12.0;
const GAIN_CEILING: f32 = 18.0;

/// How long the clip light stays on after a sample reached the top: long
/// enough to be seen, short enough to say when it stopped.
const CLIP_HOLD: Duration = Duration::from_millis(1500);

/// How long a microphone may say nothing but silence before the panel
/// says the system may be keeping it from the app.
const SILENT_AFTER: Duration = Duration::from_secs(2);

/// A timer that shows the microphone on the panel's meter thirty times a
/// second, straight onto the Editor's properties, with no publish around
/// it - and says so if the device went away.
fn meter() -> slint::Timer {
    let timer = slint::Timer::default();
    let mut shown = 0.0f32;
    let mut clipped_until: Option<Instant> = None;
    let mut reported = false;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(33),
        move || {
            crate::host::Shell::with(|shell, app| {
                // A native dialog can run this under a handler that holds
                // the studio; the meter skips a beat rather than panic.
                let lost = {
                    let Ok(studio) = shell.studio.try_borrow() else {
                        return;
                    };
                    let pane = &studio.voiceover;
                    let Some(mic) = pane.mic.as_ref() else {
                        return;
                    };
                    let level = mic.level();
                    let now = Instant::now();
                    // Up at once, down gently: a meter that falls as fast as
                    // it rises flickers.
                    shown = record::meter_position(level.peak).max(shown * 0.85);
                    if level.clipped {
                        clipped_until = Some(now + CLIP_HOLD);
                    }
                    let editor = app.global::<Editor>();
                    editor.set_record_level(shown);
                    editor.set_record_rms(record::meter_position(level.rms));
                    editor.set_record_clipped(clipped_until.is_some_and(|until| now < until));
                    editor.set_record_seconds(mic.seconds() as f32);
                    editor.set_record_silent(
                        !mic.heard()
                            && pane
                                .opened
                                .is_some_and(|opened| opened.elapsed() > SILENT_AFTER),
                    );
                    if reported { None } else { mic.failed() }
                };
                if let Some(error) = lost {
                    reported = true;
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Voiceover(VoiceoverMsg::Lost(error)));
                    });
                }
            });
        },
    );
    timer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn play_moves_the_take_and_the_picture_together() {
        assert_eq!(Phase::Ready.on_play(), Hold::Free);
        assert_eq!(Phase::Counting(2).on_play(), Hold::CancelCount);
        assert_eq!(Phase::Recording.on_play(), Hold::Pause);
        assert_eq!(Phase::Paused.on_play(), Hold::Resume);
    }

    #[test]
    fn a_pause_from_elsewhere_pauses_the_take_too_until_the_cut_ends() {
        assert_eq!(Phase::Recording.on_pause(false), Hold::Pause);
        // At the end the picture has stopped by itself; the take records on.
        assert_eq!(Phase::Recording.on_pause(true), Hold::Free);
        assert_eq!(Phase::Paused.on_pause(false), Hold::Free);
        assert_eq!(Phase::Ready.on_pause(false), Hold::Free);
        assert_eq!(Phase::Counting(3).on_pause(false), Hold::Free);
    }

    #[test]
    fn the_playhead_stays_put_while_a_take_holds_it() {
        assert_eq!(Phase::Recording.on_seek(), Hold::Refuse);
        assert_eq!(Phase::Paused.on_seek(), Hold::Refuse);
        // A count-in has nothing recorded yet: the take starts wherever
        // the playhead is when it ends.
        assert_eq!(Phase::Counting(1).on_seek(), Hold::Free);
        assert_eq!(Phase::Ready.on_seek(), Hold::Free);
    }

    #[test]
    fn a_refused_seek_is_explained_once_a_take() {
        let mut pane = VoiceoverPane::default();
        assert_eq!(pane.on_seek(), None);
        pane.phase = Phase::Recording;
        assert_eq!(pane.on_seek(), Some(true));
        assert_eq!(pane.on_seek(), Some(false));
        pane.phase = Phase::Paused;
        assert_eq!(pane.on_seek(), Some(false));
        assert!(matches!(pane.on_play(), Some(VoiceoverMsg::Resume)));
    }
}
