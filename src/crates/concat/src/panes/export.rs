// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The export sheet: the form, the running render, and the result.
//!
//! The first pane cut out of the window's controller, and the shape every
//! later one follows. The pane owns its state; every way it can change -
//! a field edited, a button pressed, a worker reporting - is one
//! [`ExportMsg`]; [`ExportPane::update`] is the only code that changes the
//! state; and [`ExportPane::data`] is the one place its Slint rows are
//! built. The controller it needs for context - the session, the output
//! size, the job slots - is handed to `update` as the studio, which the
//! pane reads and asks things of but never reaches into for its own
//! fields.

use concat_host::export::{self, AudioFormat, ExportSpec};
use concat_media::ColorRange;

use crate::format::{bytes, eta};
use crate::host::{on_ui, spawn};
use crate::i18n::{self, t, tf};
use crate::panes::Msg;
use crate::platform;
use crate::studio::{AUDIO_BPS, EXPORT_CRF, EXPORT_SHORT_SIDES, RATES, Studio, home_folder};
use crate::ui::{ExportData, ExportPhase};
use slint::{ModelRc, SharedString, VecModel};

/// Everything that can happen to the export sheet.
#[derive(Clone, Debug)]
pub enum ExportMsg {
    /// The sheet is asked for: the menu, the tray button or ⌘E.
    Open,
    /// The sheet is dismissed.
    Close,
    NameEdited(String),
    /// The picture and its sound (false), or the sound alone.
    AudioOnlyChanged(bool),
    /// Index into `AudioFormat::ALL`.
    AudioFormatChanged(i32),
    /// Index into `AudioFormat::BIT_RATES`, for the two that compress.
    AudioBitrateChanged(i32),
    RateChanged(i32),
    CodecChanged(i32),
    TenBitChanged(bool),
    /// For an HDR timeline: written HDR (true), or tone-mapped to SDR.
    HdrChanged(bool),
    /// The target bitrate field, in Mbps: the bitrate becomes a custom one.
    BitrateChanged(f32),
    /// Back to the form after a finished or failed render.
    Again,
    /// Pick the destination folder.
    Browse,
    /// The folder the picker answered with.
    FolderChosen(String),
    /// Show the finished file in the file manager.
    Reveal,
    Start,
    /// Start over a file already there: the confirmation sheet's answer.
    Overwrite,
    Cancel,
    /// The render's worker reporting where it is.
    Progress {
        /// Of the whole, `0..=1`.
        fraction: f32,
        /// What it is doing, in the person's language.
        stage: String,
    },
    /// The render's worker is done: the file written, or why not.
    Finished(Result<String, String>),
    /// The phone has moved the file to where it shows it - the path under
    /// the phone's storage - or could not, and the file is where it was.
    Published(Result<String, String>),
}

/// The export sheet's state.
pub struct ExportPane {
    pub open: bool,
    pub name: String,
    pub folder: String,
    pub resolution: usize,
    /// The file is the sound alone, written as `audio_format`.
    pub audio_only: bool,
    /// Index into `AudioFormat::ALL`.
    pub audio_format: usize,
    /// The bitrate picked for AAC or MP3, in bits a second; `None` until
    /// one is, and the format's own is written.
    pub audio_bitrate: Option<usize>,
    pub rate: usize,
    /// The bitrate is the typed one, not the recommended one for the
    /// frame and the rate.
    pub custom: bool,
    /// Index into `VideoCodec::ALL`.
    pub codec: usize,
    pub ten_bit: bool,
    /// An HDR timeline written tone-mapped to SDR rather than as it is.
    pub sdr: bool,
    /// The typed bitrate in kbps, the one written when `custom`.
    pub bitrate: u32,
    pub phase: ExportPhase,
    pub progress: f32,
    pub stage: String,
    pub message: String,
    /// Where the finished file is, for Reveal.
    pub written: String,
    /// When the render started, for a real ETA.
    started_at: Option<std::time::Instant>,
}

impl Default for ExportPane {
    fn default() -> Self {
        Self {
            open: false,
            name: "Untitled".into(),
            // An iPhone's sandbox has no Movies the app may make, as it has
            // no Desktop (#299); Documents is the folder Files shows, and a
            // finished export goes on to Photos from there.
            folder: home_folder(if cfg!(target_os = "ios") {
                "Documents/Exports"
            } else {
                "Movies"
            }),
            resolution: 2,
            audio_only: false,
            audio_format: 0,
            audio_bitrate: None,
            rate: 0,
            custom: false,
            codec: 0,
            ten_bit: false,
            sdr: false,
            bitrate: 8000,
            phase: ExportPhase::Idle,
            progress: 0.0,
            stage: String::new(),
            message: String::new(),
            written: String::new(),
            started_at: None,
        }
    }
}

impl ExportPane {
    /// Applies one message. The studio is the rest of the window, for
    /// what the sheet needs to know and to start; the pane's own state is
    /// `self`, and while this runs the studio's copy of it is a blank the
    /// pane must not read.
    pub fn update(&mut self, msg: ExportMsg, studio: &mut Studio) {
        match msg {
            ExportMsg::Open => {
                self.open = true;
                self.phase = ExportPhase::Idle;
                self.message.clear();
                // The timeline's own size: the one rung that can be picked;
                // and its own rate, first on the rate list.
                self.resolution = Self::own_rung(studio);
                self.rate = 0;
            }
            ExportMsg::Close => self.open = false,
            ExportMsg::NameEdited(name) => self.name = name,
            ExportMsg::AudioOnlyChanged(on) => self.audio_only = on,
            // A format this device cannot write is not taken, as a codec
            // is not: the picker greys it, and the choice before stands.
            ExportMsg::AudioFormatChanged(index) => {
                let index = (index.max(0) as usize).min(AudioFormat::ALL.len() - 1);
                if AudioFormat::ALL[index].encodable() {
                    self.audio_format = index;
                }
            }
            ExportMsg::AudioBitrateChanged(index) => {
                let index = (index.max(0) as usize).min(AudioFormat::BIT_RATES.len() - 1);
                self.audio_bitrate = Some(AudioFormat::BIT_RATES[index]);
            }
            ExportMsg::RateChanged(index) => {
                self.rate = (index.max(0) as usize).min(Self::rates(studio).len() - 1);
            }
            // A codec this device cannot encode is not taken: the picker
            // says so under it, and the choice before stands.
            ExportMsg::CodecChanged(index) => {
                let index = (index.max(0) as usize).min(concat_media::VideoCodec::ALL.len() - 1);
                if concat_media::VideoCodec::ALL[index].encodable() {
                    self.codec = index;
                }
            }
            ExportMsg::TenBitChanged(on) => self.ten_bit = on,
            ExportMsg::HdrChanged(on) => self.sdr = !on,
            ExportMsg::BitrateChanged(mbps) => {
                if mbps.is_finite() {
                    self.bitrate = ((mbps * 1000.0).round() as u32).clamp(500, 400_000);
                    self.custom = true;
                }
            }
            ExportMsg::Again => {
                self.phase = ExportPhase::Idle;
                self.progress = 0.0;
            }
            // The picker runs with the studio unborrowed and answers as a
            // message of its own; see `host::after_dialog`.
            ExportMsg::Browse => {
                let title = i18n::t("export.exportTo");
                let current = self.folder.clone();
                crate::host::after_dialog(
                    move || platform::pick_folder(&title, &current),
                    |studio, _, _, folder| {
                        studio.handle(Msg::Export(ExportMsg::FolderChosen(
                            folder.to_string_lossy().into_owned(),
                        )));
                    },
                );
            }
            ExportMsg::FolderChosen(folder) => self.folder = folder,
            ExportMsg::Reveal => {
                if !self.written.is_empty()
                    && let Err(error) = platform::reveal(&self.written)
                {
                    studio.notify(&i18n::tf("export.couldNotShowFile", &[&error]), true);
                }
            }
            ExportMsg::Start => self.start(studio),
            ExportMsg::Overwrite => self.begin(studio, true),
            ExportMsg::Cancel => {
                studio.host.exporter.cancel();
                self.phase = ExportPhase::Idle;
                self.progress = 0.0;
            }
            ExportMsg::Progress { fraction, stage } => {
                if self.phase == ExportPhase::Running {
                    self.progress = fraction.clamp(0.0, 1.0);
                    self.stage = stage;
                }
            }
            ExportMsg::Finished(Ok(written)) => {
                self.phase = ExportPhase::Done;
                self.progress = 1.0;
                self.written = written.clone();
                // A phone moves the file to where its gallery shows it, and
                // the sheet says "finished" once it is there. The sound
                // alone is no video for the gallery: it goes where the
                // phone keeps documents, as an exported SubRip does.
                let published = |result| {
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Export(ExportMsg::Published(result)));
                    });
                };
                let handed = if self.audio_only {
                    platform::publish_document(
                        written.into(),
                        self.audio_format().mime(),
                        published,
                    )
                } else {
                    platform::publish_export(written.into(), published)
                };
                if !handed {
                    studio.notify(&t("export.exportFinished"), false);
                }
            }
            ExportMsg::Published(Ok(path)) => {
                self.written = path;
                studio.notify(&t("export.exportFinished"), false);
            }
            ExportMsg::Published(Err(error)) => {
                let folder = platform::published_folder().unwrap_or_default();
                studio.notify(&tf("export.couldNotPublish", &[&folder, &error]), true);
            }
            ExportMsg::Finished(Err(error)) => {
                if self.phase == ExportPhase::Idle {
                    // Cancelled: the sheet already went back to idle.
                    return;
                }
                self.phase = ExportPhase::Failed;
                self.message = error.clone();
                studio.notify(&tf("export.exportFailed", &[&error]), true);
            }
        }
    }

    /// The resolution ladder for this project: the standard short sides,
    /// with the project's own among them where it is not already - a
    /// custom frame has its own rung - from largest to smallest.
    fn ladder(studio: &Studio) -> Vec<u32> {
        let mut rungs: Vec<u32> = EXPORT_SHORT_SIDES.to_vec();
        let own = Self::own_short(studio);
        if !rungs.contains(&own) {
            rungs.push(own);
        }
        rungs.sort_unstable_by(|a, b| b.cmp(a));
        rungs
    }

    /// The rates the sheet offers: the timeline's own first - what it is
    /// exported at unless the person picks another - then every other
    /// rate a project can be made at. A 25 fps cut used to export at 30
    /// unless someone noticed (audit 2026-10-04, #8).
    fn rates(studio: &Studio) -> Vec<(i64, i64)> {
        let video = studio.project().active().video;
        Self::rates_around((video.rate_num, video.rate_den.max(1)))
    }

    /// [`ExportPane::rates`] for a timeline at `own`.
    fn rates_around(own: (i64, i64)) -> Vec<(i64, i64)> {
        let same = |(num, den): (i64, i64)| num * own.1 == own.0 * den;
        std::iter::once(own)
            .chain(
                RATES
                    .iter()
                    .map(|&(_, num, den)| (num, den))
                    .filter(|&rate| !same(rate)),
            )
            .collect()
    }

    /// The rate the export renders at.
    fn rate(&self, studio: &Studio) -> (i64, i64) {
        let rates = Self::rates(studio);
        rates[self.rate.min(rates.len() - 1)]
    }

    /// A rate as the sheet names it: "23.976 fps", "25 fps".
    fn rate_name((num, den): (i64, i64)) -> String {
        let name = match RATES.iter().find(|&&(_, n, d)| n * den == num * d) {
            Some((name, _, _)) => (*name).to_owned(),
            None => {
                let fps = format!("{:.3}", crate::studio::fps_of(num, den));
                fps.trim_end_matches('0').trim_end_matches('.').to_owned()
            }
        };
        format!("{name} fps")
    }

    /// The project's short side, in pixels.
    fn own_short(studio: &Studio) -> u32 {
        let (width, height) = studio.output_size();
        width.min(height).max(2)
    }

    /// The rung of the ladder that is the project's own size: the one
    /// the sheet lets be picked, since an edit exports at the size it was
    /// cut at. https://github.com/jub0t/Concat/issues/109
    fn own_rung(studio: &Studio) -> usize {
        let own = Self::own_short(studio);
        Self::ladder(studio)
            .iter()
            .position(|rung| *rung == own)
            .unwrap_or(0)
    }

    /// `short` scaled along the project's aspect and rounded to even
    /// dimensions, which is what the encoder's chroma subsampling needs.
    fn frame_for(studio: &Studio, short: u32) -> (u32, u32) {
        let (project_w, project_h) = studio.output_size();
        let (project_w, project_h) = (project_w.max(1) as f64, project_h.max(1) as f64);
        let even = |side: f64| ((side / 2.0).round() as u32 * 2).max(2);
        if project_w >= project_h {
            (even(short as f64 * project_w / project_h), short)
        } else {
            (short, even(short as f64 * project_h / project_w))
        }
    }

    /// The frame the export renders at: the sheet's rung of the ladder,
    /// along the project's aspect.
    pub fn size(&self, studio: &Studio) -> (u32, u32) {
        let ladder = Self::ladder(studio);
        let short = ladder[self.resolution.min(ladder.len() - 1)];
        Self::frame_for(studio, short)
    }

    /// The bitrate the file is written at, in kbps: the typed one, or the
    /// one YouTube asks uploads at for the frame - 5 Mbps at 720p, 8 at
    /// 1080p, 16 at 1440p and 40 at 4K, half again above 30 fps - which
    /// every other platform takes too, less for HEVC and AV1, which spend
    /// fewer bits on the same picture, and a quarter more at ten bits.
    /// Rounded to half a megabit, as a person would say it. The
    /// recommended one follows the frame, the rate and the codec until a
    /// bitrate is typed.
    pub fn target_kbps(&self, studio: &Studio) -> u32 {
        if self.custom {
            return self.bitrate;
        }
        let (width, height) = self.size(studio);
        let (num, den) = self.rate(studio);
        let fps = num as f32 / den.max(1) as f32;
        let base = match width.min(height) {
            0..=480 => 2_500.0,
            481..=720 => 5_000.0,
            721..=1080 => 8_000.0,
            1081..=1440 => 16_000.0,
            _ => 40_000.0,
        };
        let motion = if fps > 30.5 { 1.5 } else { 1.0 };
        let deep = if self.deep(studio) { 1.25 } else { 1.0 };
        let kbps = base * motion * self.codec(studio).size_factor() * deep;
        ((kbps / 500.0).round() * 500.0).max(500.0) as u32
    }

    /// The file's size, in bytes: the bitrate is an average the encoder
    /// keeps to, so this is what is written, give or take.
    pub fn size_bytes(&self, studio: &Studio) -> f32 {
        if self.audio_only {
            let bps = match self.audio_bit_rate() {
                Some(bps) => bps as f32,
                None => self.audio_format().bits_per_second(),
            };
            return bps * studio.duration().max(1.0) / 8.0;
        }
        let video = self.target_kbps(studio) as f32 * 1000.0;
        (video + AUDIO_BPS) * studio.duration().max(1.0) / 8.0
    }

    /// The format the sound alone is written as.
    pub fn audio_format(&self) -> AudioFormat {
        AudioFormat::ALL[self.audio_format.min(AudioFormat::ALL.len() - 1)]
    }

    /// The bitrate the sound alone is written at, in bits a second: the
    /// one picked, or the format's own until one is; `None` for the two
    /// lossless formats, which have none.
    pub fn audio_bit_rate(&self) -> Option<usize> {
        self.audio_format()
            .bit_rate()
            .map(|own| self.audio_bitrate.unwrap_or(own))
    }

    /// The file's extension: the audio format's for the sound alone, an
    /// MP4's for a video.
    fn extension(&self) -> &'static str {
        if self.audio_only {
            self.audio_format().extension()
        } else {
            "mp4"
        }
    }

    /// The codec the sheet has chosen - HEVC for an HDR file where H.264
    /// was, since H.264 does not carry HDR.
    pub fn codec(&self, studio: &Studio) -> concat_media::VideoCodec {
        let codec =
            concat_media::VideoCodec::ALL[self.codec.min(concat_media::VideoCodec::ALL.len() - 1)];
        if self.hdr(studio) && codec == concat_media::VideoCodec::H264 {
            concat_media::VideoCodec::Hevc
        } else {
            codec
        }
    }

    /// Whether the file is written HDR: the timeline is, the sheet was
    /// not told to tone-map it to SDR, and the codec an HDR file would be
    /// written in can be written at ten bits here - on a phone it cannot,
    /// and the file is tone-mapped rather than failing.
    pub fn hdr(&self, studio: &Studio) -> bool {
        let chosen =
            concat_media::VideoCodec::ALL[self.codec.min(concat_media::VideoCodec::ALL.len() - 1)];
        let deep = match chosen {
            concat_media::VideoCodec::H264 => concat_media::VideoCodec::Hevc,
            codec => codec,
        };
        studio.timeline().video.color_space.is_hdr()
            && !self.sdr
            && deep.ten_bit_available()
            && deep.encodable()
    }

    /// Whether the file is written at ten bits: HDR always, SDR when the
    /// switch asks and the codec can be written that deep here.
    pub fn deep(&self, studio: &Studio) -> bool {
        self.hdr(studio) || (self.ten_bit && self.codec(studio).ten_bit_available())
    }

    /// Starts the render on a worker, once the name is a file's and no
    /// file is already there; its reports come back as messages.
    fn start(&mut self, studio: &mut Studio) {
        self.begin(studio, false);
    }

    /// [`ExportPane::start`], with `overwrite` saying the person has
    /// answered the sheet that asked about the file already there.
    fn begin(&mut self, studio: &mut Studio, overwrite: bool) {
        if studio.session.is_none() {
            return;
        }
        if studio.timeline().clips.is_empty() {
            self.phase = ExportPhase::Failed;
            self.message = t("export.nothingTimelineExport");
            return;
        }
        // The name is a file's, or the sheet says what is wrong with it: an
        // empty one wrote a hidden `.mp4`, and a slash made folders (audit
        // 2026-10-09, finding 7).
        let Some(name) = file_name(&self.name) else {
            self.phase = ExportPhase::Failed;
            self.message = t("export.nameNeedsToBeAFileName");
            return;
        };
        // The sound alone of a timeline with nothing to hear would be a
        // file of silence: the sheet says so instead. Audible as playback
        // has it, so the file holds what the window plays.
        if self.audio_only
            && studio.session.as_ref().is_some_and(|session| {
                crate::studio::audio_specs(&session.flattened_clips()).is_empty()
            })
        {
            self.phase = ExportPhase::Failed;
            self.message = t("export.nothingToHear");
            return;
        }
        let output = format!(
            "{}/{name}.{}",
            self.folder.trim_end_matches('/'),
            self.extension()
        );
        // A file already there is replaced once the person has said so:
        // the confirmation sheet asks, and its answer comes back as
        // `Overwrite`. Nothing is written until then; the engine renders
        // beside the file and renames over it at the end.
        if !overwrite && std::path::Path::new(&output).exists() {
            studio.confirm = Some(crate::studio::Confirm::ReplaceExport { path: output });
            return;
        }
        let job = match studio.host.exporter.begin() {
            Ok(job) => job,
            Err(error) => {
                self.phase = ExportPhase::Failed;
                self.message = error;
                return;
            }
        };
        let Some(session) = studio.session.as_ref() else {
            return;
        };
        let spec = ExportSpec {
            output: output.clone(),
            // Unused at an average bitrate; the quality the encoder falls
            // back on were it ever not given one.
            crf: EXPORT_CRF,
            preset: "veryfast".into(),
            codec: self.codec(studio),
            ten_bit: self.deep(studio),
            rate_mode: concat_media::RateMode::Average,
            bitrate_kbps: self.target_kbps(studio),
            // Video range, what every player and every platform expects.
            color_range: ColorRange::Limited,
            hdr: self.hdr(studio),
        };
        // Titles are pictures, and the sound alone draws none: they are
        // not rasterised for it.
        let titles = if self.audio_only {
            Vec::new()
        } else {
            let (frame_w, frame_h) = studio.output_size();
            studio
                .host
                .titles
                .clips(session.project(), frame_w, frame_h)
                .into_iter()
                .map(|title| title.clip)
                .collect()
        };
        let mut request = export::request(session, &spec, titles);
        // The sound alone keeps the timeline's own rate, which places its
        // transitions as the monitor plays them; the sheet's rate is for
        // a picture, and its row is not shown.
        if !self.audio_only {
            let (width, height) = self.size(studio);
            let (num, den) = self.rate(studio);
            request.width = width;
            request.height = height;
            request.rate_num = num;
            request.rate_den = den;
        }

        studio.pause();
        self.phase = ExportPhase::Running;
        self.progress = 0.0;
        self.stage = if self.audio_only {
            t("export.mixingAudio")
        } else {
            t("export.renderingVideo")
        };
        self.message.clear();
        self.written.clear();
        self.started_at = Some(std::time::Instant::now());

        // The export draws on the window's device - a sibling of the
        // monitor's compositor - not on a second device opened through the
        // drivers for the export alone, which on Windows laptops with NVIDIA
        // chips took the whole app down at the first frame with nothing in
        // the log (issue #202). Not on Vulkan, whose queue the window
        // submits to from its own thread (#324); see `Monitor::off_thread`.
        // None there or without a GPU, and the render then opens what it
        // can.
        // The sound alone draws nothing, and borrows no device for it.
        let audio_only = self.audio_only;
        let bit_rate = self.audio_bit_rate();
        let compositor = if audio_only {
            None
        } else {
            studio.host.monitor.off_thread()
        };
        spawn(
            move || {
                let job = job;
                let report = |progress: export::Progress| {
                    let fraction = if progress.total > 0 {
                        progress.frame as f32 / progress.total as f32
                    } else {
                        0.0
                    };
                    let stage = match progress.stage {
                        "rendering" => t("export.renderingVideo"),
                        "mixing audio" => t("export.mixingAudio"),
                        "muxing" => t("export.finalisingFile"),
                        other => other.to_owned(),
                    };
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Export(ExportMsg::Progress { fraction, stage }));
                    });
                };
                if audio_only {
                    export::run_audio(&request, bit_rate, job.cancel_flag(), report)
                } else {
                    export::run_on(&request, compositor, job.cancel_flag(), report)
                }
            },
            |studio, _, _, result| studio.handle(Msg::Export(ExportMsg::Finished(result))),
        );
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> ExportData {
        let (width, height) = self.size(studio);
        let (num, den) = self.rate(studio);
        let rate = num as f32 / den as f32;
        let clips = studio.timeline().clips.len();
        let titles = studio
            .timeline()
            .clips
            .iter()
            .filter(|clip| clip.kind == concat_project::model::ClipKind::Text)
            .count();
        ExportData {
            open: self.open,
            name: self.name.as_str().into(),
            // Where the file ends up: on a phone, the folder its gallery
            // shows, not the app's own that the export writes into.
            // The name as it will be written: trimmed, as `begin` trims it.
            path: match platform::published_folder() {
                Some(folder) => {
                    format!("{folder}/{}.{}", self.name.trim(), self.extension())
                }
                None => format!(
                    "{}/{}.{}",
                    self.folder.trim_end_matches('/'),
                    self.name.trim(),
                    self.extension()
                ),
            }
            .into(),
            format: if self.audio_only {
                let khz = concat_media::audio::MIX_RATE / 1000;
                tf("export.kilohertzStereo", &[&khz.to_string()])
            } else {
                format!("{width} × {height} · {rate:.2} fps")
            }
            .into(),
            duration: {
                let whole = studio.duration().max(0.0) as i32;
                format!("{}:{:02}", whole / 60, whole % 60).into()
            },
            contents: if titles > 0 {
                tf("export.contentsClipsTitles", &[&clips, &titles])
            } else {
                tf("export.contentsClips", &[&clips])
            }
            .into(),
            resolution: self.resolution as i32,
            resolutions: {
                let sizes: Vec<SharedString> = Self::ladder(studio)
                    .iter()
                    .map(|rung| {
                        let (width, height) = Self::frame_for(studio, *rung);
                        format!("{width} × {height}").into()
                    })
                    .collect();
                ModelRc::new(VecModel::from(sizes))
            },
            rate: self.rate as i32,
            rates: {
                let names: Vec<SharedString> = Self::rates(studio)
                    .into_iter()
                    .map(|rate| Self::rate_name(rate).into())
                    .collect();
                ModelRc::new(VecModel::from(names))
            },
            rate_details: {
                let mut details: Vec<SharedString> =
                    vec![SharedString::new(); Self::rates(studio).len()];
                details[0] = t("export.timelineSize").into();
                ModelRc::new(VecModel::from(details))
            },
            codec: concat_media::VideoCodec::ALL
                .iter()
                .position(|codec| *codec == self.codec(studio))
                .unwrap_or(0) as i32,
            h264_encodable: concat_media::VideoCodec::H264.encodable(),
            hevc_encodable: concat_media::VideoCodec::Hevc.encodable(),
            av1_encodable: concat_media::VideoCodec::Av1.encodable(),
            ten_bit: self.ten_bit,
            hdr_timeline: studio.timeline().video.color_space.is_hdr(),
            hdr: self.hdr(studio),
            deep_available: concat_media::VideoCodec::ALL
                .iter()
                .any(|codec| codec.ten_bit_available()),
            hdr_label: match studio.timeline().video.color_space {
                concat_project::model::ColorSpace::Pq => "HDR (PQ)",
                _ => "HDR (HLG)",
            }
            .into(),
            target: self.target_kbps(studio) as f32 / 1000.0,
            audio_only: self.audio_only,
            audio_format: self.audio_format as i32,
            audio_formats: {
                let labels: Vec<SharedString> = AudioFormat::ALL
                    .iter()
                    .map(|format| format.label().into())
                    .collect();
                ModelRc::new(VecModel::from(labels))
            },
            audio_format_details: {
                let details: Vec<SharedString> = AudioFormat::ALL
                    .iter()
                    .map(|format| {
                        if !format.encodable() {
                            return t("export.notOnThisDevice").into();
                        }
                        match format {
                            AudioFormat::Wav => t("export.uncompressed"),
                            AudioFormat::Flac => t("export.lossless"),
                            AudioFormat::Aac => t("export.smallest"),
                            AudioFormat::Mp3 => t("export.playsEverywhere"),
                        }
                        .into()
                    })
                    .collect();
                ModelRc::new(VecModel::from(details))
            },
            // The bitrates, for the two formats that take one; none, and
            // so no row, for the lossless two.
            audio_bitrate: self
                .audio_bit_rate()
                .and_then(|bps| AudioFormat::BIT_RATES.iter().position(|rate| *rate == bps))
                .map_or(-1, |index| index as i32),
            audio_bitrates: {
                let labels: Vec<SharedString> = self
                    .audio_format()
                    .bit_rates()
                    .iter()
                    .map(|bps| format!("{} kbps", bps / 1000).into())
                    .collect();
                ModelRc::new(VecModel::from(labels))
            },
            audio_bitrate_details: {
                let own = self.audio_format().bit_rate();
                let details: Vec<SharedString> = self
                    .audio_format()
                    .bit_rates()
                    .iter()
                    .map(|bps| {
                        if Some(*bps) == own {
                            t("export.default").into()
                        } else {
                            SharedString::new()
                        }
                    })
                    .collect();
                ModelRc::new(VecModel::from(details))
            },
            audio_format_disabled: {
                let disabled: Vec<bool> = AudioFormat::ALL
                    .iter()
                    .map(|format| !format.encodable())
                    .collect();
                ModelRc::new(VecModel::from(disabled))
            },
            encoding: if self.audio_only {
                // "WAV 16-bit", "MP3 320 kbps": the format, and the depth or
                // the rate it is written at.
                let format = self.audio_format();
                match self.audio_bit_rate() {
                    Some(bps) => format!("{} {} kbps", format.label(), bps / 1000),
                    None => format!("{} 16-bit", format.label()),
                }
                .into()
            } else {
                // "HEVC 10-bit · hardware": the standard, the depth when it
                // is the deeper one, and whether the platform's own encoder
                // will be doing it.
                let codec = self.codec(studio);
                let mut words = vec![codec.label().to_owned()];
                if self.deep(studio) {
                    words.push("10-bit".to_owned());
                }
                if self.hdr(studio) {
                    words.push(
                        match studio.timeline().video.color_space {
                            concat_project::model::ColorSpace::Pq => "PQ",
                            _ => "HLG",
                        }
                        .to_owned(),
                    );
                }
                if codec.hardware_encoded(true, self.deep(studio)) {
                    words.push(format!("· {}", t("export.hardware")));
                }
                words.join(" ").into()
            },
            size_chosen: bytes(self.size_bytes(studio)).into(),
            phase: self.phase,
            progress: self.progress,
            stage: self.stage.as_str().into(),
            eta: if self.phase == ExportPhase::Running && self.progress > 0.02 {
                self.started_at
                    .map(|started| {
                        let elapsed = started.elapsed().as_secs_f32();
                        eta(elapsed / self.progress * (1.0 - self.progress)).into()
                    })
                    .unwrap_or_default()
            } else {
                slint::SharedString::new()
            },
            message: self.message.as_str().into(),
            done_size: bytes(self.size_bytes(studio)).into(),
            empty: clips == 0,
        }
    }
}

/// The typed name as the file's, or `None` for one that cannot be: empty
/// once trimmed, a dot or two, or holding a character no file system
/// here takes. The set is Windows' - a slash and a backslash, the colon,
/// the quote and the wildcards, the angle brackets and the bar - on every
/// platform, so a project's export is named the same wherever it is
/// opened; a control character is refused with them.
fn file_name(typed: &str) -> Option<String> {
    let name = typed.trim();
    if name.is_empty() || name.chars().all(|c| c == '.') {
        return None;
    }
    let refused = |c: char| c.is_control() || r#"/\:*?"<>|"#.contains(c);
    (!name.chars().any(refused)).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name is a file's or the export does not start: trimmed, never
    /// empty, never a path, and none of the characters Windows refuses.
    #[test]
    fn the_file_name_is_a_file_name() {
        assert_eq!(file_name("  My cut "), Some("My cut".to_owned()));
        assert_eq!(file_name("cut.final"), Some("cut.final".to_owned()));
        assert_eq!(file_name("Ünïcödé 日本"), Some("Ünïcödé 日本".to_owned()));
        for bad in [
            "", "   ", ".", "..", "a/b", r"a\b", "c:d", "a?b", "<a>", "a|b", "a\"b", "a*", "a\tb",
        ] {
            assert_eq!(file_name(bad), None, "{bad:?}");
        }
    }

    /// The file is named for what it holds: an MP4 for a video, and the
    /// format's own extension for the sound alone.
    #[test]
    fn the_extension_follows_what_is_exported() {
        let mut pane = ExportPane::default();
        assert_eq!(pane.extension(), "mp4");
        pane.audio_only = true;
        for (index, format) in AudioFormat::ALL.iter().enumerate() {
            pane.audio_format = index;
            assert_eq!(pane.extension(), format.extension(), "{format:?}");
        }
    }

    /// The two formats that compress are written at their own bitrate
    /// until one is picked, and then at that one, whichever of the two is
    /// chosen; the lossless two have none, and so no row to pick it in.
    #[test]
    fn the_bitrate_is_the_formats_own_until_one_is_picked() {
        let mut pane = ExportPane {
            audio_only: true,
            ..ExportPane::default()
        };
        let at = |format: AudioFormat| {
            AudioFormat::ALL
                .iter()
                .position(|each| *each == format)
                .expect("every format is in ALL")
        };
        pane.audio_format = at(AudioFormat::Aac);
        assert_eq!(pane.audio_bit_rate(), Some(192_000));
        pane.audio_format = at(AudioFormat::Mp3);
        assert_eq!(pane.audio_bit_rate(), Some(320_000));
        pane.audio_bitrate = Some(AudioFormat::BIT_RATES[0]);
        assert_eq!(pane.audio_bit_rate(), Some(128_000));
        pane.audio_format = at(AudioFormat::Aac);
        assert_eq!(pane.audio_bit_rate(), Some(128_000));
        for format in [AudioFormat::Wav, AudioFormat::Flac] {
            pane.audio_format = at(format);
            assert_eq!(pane.audio_bit_rate(), None, "{format:?}");
            assert!(format.bit_rates().is_empty(), "{format:?}");
        }
    }

    /// The timeline's rate leads the list and is not offered twice; the
    /// NTSC fractions keep their camera names.
    #[test]
    fn the_timeline_rate_comes_first() {
        let rates = ExportPane::rates_around((25, 1));
        assert_eq!(rates[0], (25, 1));
        assert_eq!(rates.iter().filter(|rate| **rate == (25, 1)).count(), 1);
        assert_eq!(rates.len(), RATES.len());
        let rates = ExportPane::rates_around((50, 2));
        assert_eq!(rates.len(), RATES.len(), "25 written as 50/2 is still 25");
        let odd = ExportPane::rates_around((15, 1));
        assert_eq!((odd[0], odd.len()), ((15, 1), RATES.len() + 1));
        assert_eq!(ExportPane::rate_name((24000, 1001)), "23.976 fps");
        assert_eq!(ExportPane::rate_name((25, 1)), "25 fps");
        assert_eq!(ExportPane::rate_name((15, 1)), "15 fps");
        assert_eq!(ExportPane::rate_name((12500, 1000)), "12.5 fps");
    }
}
