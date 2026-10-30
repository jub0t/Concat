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

use concat_host::export::{self, ExportSpec};
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
                self.follow_canvas(studio);
            }
            ExportMsg::Close => self.open = false,
            ExportMsg::NameEdited(name) => self.name = name,
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
                    // Gone back: the export started again from its first
                    // frame on the slower encoder, and the time left is
                    // reckoned from there.
                    if fraction < self.progress {
                        self.started_at = Some(std::time::Instant::now());
                    }
                    self.progress = fraction.clamp(0.0, 1.0);
                    self.stage = stage;
                }
            }
            ExportMsg::Finished(Ok(written)) => {
                self.phase = ExportPhase::Done;
                self.progress = 1.0;
                self.written = written.clone();
                // A phone moves the file to where its gallery shows it, and
                // the sheet says "finished" once it is there.
                let handed = platform::publish_export(written.into(), |result| {
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Export(ExportMsg::Published(result)));
                    });
                });
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
        let video = self.target_kbps(studio) as f32 * 1000.0;
        (video + AUDIO_BPS) * studio.duration().max(1.0) / 8.0
    }

    /// The codec a sheet opens on follows the canvas (#456): a timeline
    /// that lets something show through opens on ProRes 4444, which every
    /// editor takes with its alpha (VP9 where ProRes cannot be written),
    /// and an opaque one leaves an alpha codec picked for an overlay
    /// before for H.264, rather than writing it hundreds of megabits it
    /// has no use for. Within the sheet the pick is the person's.
    fn follow_canvas(&mut self, studio: &Studio) {
        use concat_media::VideoCodec;
        let chosen = VideoCodec::ALL[self.codec.min(VideoCodec::ALL.len() - 1)];
        let see_through = studio.timeline().video.canvas[3] < 255;
        let wanted = if see_through && !chosen.keeps_alpha() {
            [VideoCodec::ProRes4444, VideoCodec::Vp9Alpha]
                .into_iter()
                .find(|codec| codec.encodable())
        } else if !see_through && chosen.keeps_alpha() {
            Some(VideoCodec::H264)
        } else {
            None
        };
        if let Some(index) = wanted.and_then(|codec| VideoCodec::ALL.iter().position(|c| *c == codec)) {
            self.codec = index;
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
        // An overlay that keeps its alpha is written SDR: the export
        // tone-maps an HDR timeline for it.
        !chosen.keeps_alpha()
            && studio.timeline().video.color_space.is_hdr()
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
        let output = format!(
            "{}/{name}.{}",
            self.folder.trim_end_matches('/'),
            self.codec(studio).container()
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
        let (frame_w, frame_h) = studio.output_size();
        let titles = studio
            .host
            .titles
            .clips(session.project(), frame_w, frame_h)
            .into_iter()
            .map(|title| title.clip)
            .collect();
        let mut request = export::request(session, &spec, titles);
        let (width, height) = self.size(studio);
        let (num, den) = self.rate(studio);
        request.width = width;
        request.height = height;
        request.rate_num = num;
        request.rate_den = den;

        studio.pause();
        // The monitor's readers that sit idle are closed now rather than
        // ten seconds on by the sweep: on a phone up to three of them hold
        // MediaCodec decoder sessions, which the chip has few of and its
        // encoder and the export's own decoders need. A reader comes back
        // on the next frame the monitor asks of it; the cached frames stay.
        let closed = concat_host::scheduler()
            .pool()
            .close_idle(std::time::Duration::ZERO);
        if closed > 0 {
            log::info!("export: closed {closed} idle preview decoder(s) before starting");
        }
        self.phase = ExportPhase::Running;
        self.progress = 0.0;
        self.stage = t("export.renderingVideo");
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
        let compositor = studio.host.monitor.off_thread();
        spawn(
            move || {
                let job = job;
                export::run_on(&request, compositor, job.cancel_flag(), |progress| {
                    let fraction = if progress.total > 0 {
                        progress.frame as f32 / progress.total as f32
                    } else {
                        0.0
                    };
                    let stage = match progress.stage {
                        "rendering" => t("export.renderingVideo"),
                        concat_export::RENDERING_IN_SOFTWARE => t("export.renderingInSoftware"),
                        "mixing audio" => t("export.mixingAudio"),
                        "muxing" => t("export.finalisingFile"),
                        other => other.to_owned(),
                    };
                    on_ui(move |studio, _, _| {
                        studio.handle(Msg::Export(ExportMsg::Progress { fraction, stage }));
                    });
                })
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
        let container = self.codec(studio).container();
        ExportData {
            open: self.open,
            name: self.name.as_str().into(),
            // Where the file ends up: on a phone, the folder its gallery
            // shows, not the app's own that the export writes into.
            // The name as it will be written: trimmed, as `begin` trims it.
            path: match platform::published_folder() {
                Some(folder) => format!("{folder}/{}.{container}", self.name.trim()),
                None => format!(
                    "{}/{}.{container}",
                    self.folder.trim_end_matches('/'),
                    self.name.trim()
                ),
            }
            .into(),
            format: format!("{width} × {height} · {rate:.2} fps").into(),
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
            prores_encodable: concat_media::VideoCodec::ProRes4444.encodable(),
            vp9_alpha_encodable: concat_media::VideoCodec::Vp9Alpha.encodable(),
            see_through: studio.timeline().video.canvas[3] < 255,
            keeps_alpha: self.codec(studio).keeps_alpha(),
            rate_controlled: self.codec(studio).rate_controlled(),
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
            encoding: {
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
                words.join(" ")
            }
            .into(),
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
