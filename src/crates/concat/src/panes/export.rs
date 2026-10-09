// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
use crate::studio::{
    AUDIO_BPS, EXPORT_CRF, EXPORT_SHORT_SIDES, EXPORT_TIERS, RATES, Studio, home_folder,
};
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
    ResolutionChanged(i32),
    RateChanged(i32),
    QualityChanged(i32),
    CodecChanged(i32),
    TenBitChanged(bool),
    /// For an HDR timeline: written HDR (true), or tone-mapped to SDR.
    HdrChanged(bool),
    /// Limited or full range, as a row of the Advanced section's list.
    ColorRangeChanged(i32),
    /// The Advanced section is opened or closed.
    AdvancedToggled(bool),
    /// VBR or CBR.
    RateModeChanged(i32),
    /// The target bitrate field: digits only, in kbps.
    BitrateChanged(String),
    /// Back to the form after a finished or failed render.
    Again,
    /// Pick the destination folder.
    Browse,
    /// Show the finished file in the file manager.
    Reveal,
    Start,
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
    pub quality: usize,
    /// Index into `VideoCodec::ALL`.
    pub codec: usize,
    pub ten_bit: bool,
    /// An HDR timeline written tone-mapped to SDR rather than as it is.
    pub sdr: bool,
    /// Index into `ColorRange::ALL`: 0 limited, 1 full. Read only while
    /// the Advanced section is open, like the bitrate.
    /// https://github.com/jub0t/Concat/issues/103
    pub color_range: usize,
    /// The Advanced section is open: bitrate controls show, and the size
    /// estimate reads the chosen bitrate.
    pub advanced: bool,
    /// Index into `RateMode::ALL`.
    pub rate_mode: usize,
    /// Target bitrate in kbps, used when `rate_mode` is CBR.
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
            folder: home_folder("Movies"),
            resolution: 2,
            rate: 0,
            quality: 1,
            codec: 0,
            ten_bit: false,
            sdr: false,
            color_range: 0,
            advanced: false,
            rate_mode: 0,
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
            ExportMsg::ResolutionChanged(index) => {
                // Only the timeline's own rung is taken; the sheet greys the
                // rest, and this is the same rule for a caller that did not.
                let index = index.max(0) as usize;
                if index == Self::own_rung(studio) {
                    self.resolution = index;
                }
            }
            ExportMsg::RateChanged(index) => {
                self.rate = (index.max(0) as usize).min(Self::rates(studio).len() - 1);
            }
            ExportMsg::QualityChanged(index) => self.quality = (index.max(0) as usize).min(2),
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
            ExportMsg::ColorRangeChanged(index) => {
                self.color_range = (index.max(0) as usize).min(ColorRange::ALL.len() - 1);
            }
            ExportMsg::AdvancedToggled(on) => self.advanced = on,
            ExportMsg::RateModeChanged(index) => self.rate_mode = index.max(0) as usize,
            ExportMsg::BitrateChanged(text) => {
                let digits: String = text.chars().filter(|c| c.is_ascii_digit()).collect();
                if let Ok(value) = digits.parse::<u32>() {
                    self.bitrate = value.clamp(100, 200_000);
                }
            }
            ExportMsg::Again => {
                self.phase = ExportPhase::Idle;
                self.progress = 0.0;
            }
            ExportMsg::Browse => {
                if let Some(folder) =
                    platform::pick_folder(&i18n::t("export.exportTo"), &self.folder)
                {
                    self.folder = folder.to_string_lossy().into_owned();
                }
            }
            ExportMsg::Reveal => {
                if !self.written.is_empty()
                    && let Err(error) = platform::reveal(&self.written)
                {
                    studio.notify(&i18n::tf("export.couldNotShowFile", &[&error]), true);
                }
            }
            ExportMsg::Start => self.start(studio),
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

    /// A rough size of the file at one quality tier, in bytes.
    pub fn size_bytes(&self, studio: &Studio, tier: usize) -> f32 {
        let (width, height) = self.size(studio);
        let (num, den) = self.rate(studio);
        let rate = num as f32 / den as f32;
        let pixels = (width as f32 * height as f32) / (1920.0 * 1080.0);
        // In CBR the bitrate is the number, not the tier; the pixels, rate
        // and codec factors no longer apply because the encoder is pinned.
        let video = if self.advanced && self.rate_mode == 1 {
            self.bitrate as f32 * 1000.0
        } else {
            EXPORT_TIERS[tier.min(2)]
                * 1_000_000.0
                * pixels
                * (rate / 30.0)
                * self.codec(studio).size_factor()
                * if self.deep(studio) { 1.05 } else { 1.0 }
        };
        (video + AUDIO_BPS) * studio.duration().max(1.0) / 8.0
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

    /// The range the file is written in: the Advanced section's choice
    /// while the section is open, video range otherwise - so a sheet with
    /// Advanced off exports what it always did.
    pub fn color_range(&self) -> ColorRange {
        if self.advanced {
            ColorRange::ALL[self.color_range.min(ColorRange::ALL.len() - 1)]
        } else {
            ColorRange::Limited
        }
    }

    /// Starts the render on a worker. Its reports come back as messages.
    fn start(&mut self, studio: &mut Studio) {
        let Some(session) = studio.session.as_ref() else {
            return;
        };
        if studio.timeline().clips.is_empty() {
            self.phase = ExportPhase::Failed;
            self.message = t("export.nothingTimelineExport");
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
        let output = format!(
            "{}/{}.mp4",
            self.folder.trim_end_matches('/'),
            self.name.trim()
        );
        let spec = ExportSpec {
            output: output.clone(),
            crf: EXPORT_CRF[self.quality.min(2)],
            preset: "veryfast".into(),
            codec: self.codec(studio),
            ten_bit: self.deep(studio),
            rate_mode: if self.advanced && self.rate_mode == 1 {
                concat_media::RateMode::Cbr
            } else {
                concat_media::RateMode::Vbr
            },
            bitrate_kbps: if self.advanced && self.rate_mode == 1 {
                self.bitrate
            } else {
                0
            },
            color_range: self.color_range(),
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
        // the log (issue #202). None without a GPU here, and the render then
        // opens what it can.
        let compositor = studio.host.monitor.sibling();
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
        ExportData {
            open: self.open,
            name: self.name.as_str().into(),
            // Where the file ends up: on a phone, the folder its gallery
            // shows, not the app's own that the export writes into.
            path: match platform::published_folder() {
                Some(folder) => format!("{folder}/{}.mp4", self.name),
                None => format!("{}/{}.mp4", self.folder.trim_end_matches('/'), self.name),
            }
            .into(),
            format: format!("{width} × {height} · {rate:.2} fps").into(),
            duration: {
                let whole = studio.duration().max(0.0) as i32;
                format!("{}:{:02}", whole / 60, whole % 60).into()
            },
            contents: if titles > 0 {
                format!("{clips} clips · {titles} titles")
            } else {
                format!("{clips} clips")
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
            resolution_details: {
                let names: Vec<SharedString> = Self::ladder(studio)
                    .iter()
                    .map(|rung| match rung {
                        2160 => "4K".into(),
                        1440 => "QHD".into(),
                        1080 => "1080p".into(),
                        720 => "720p".into(),
                        _ => t("export.timelineSize").into(),
                    })
                    .collect();
                ModelRc::new(VecModel::from(names))
            },
            resolution_disabled: {
                let own = Self::own_rung(studio);
                let off: Vec<bool> = (0..Self::ladder(studio).len())
                    .map(|rung| rung != own)
                    .collect();
                ModelRc::new(VecModel::from(off))
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
            quality: self.quality as i32,
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
            color_range: self.color_range as i32,
            advanced: self.advanced,
            rate_mode: self.rate_mode as i32,
            bitrate: self.bitrate as i32,
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
                if self.color_range() == ColorRange::Full {
                    words.push(t("export.fullRange"));
                }
                if codec.hardware_encoded(true, self.deep(studio)) {
                    words.push(format!("· {}", t("export.hardware")));
                }
                words.join(" ")
            }
            .into(),
            size_high: bytes(self.size_bytes(studio, 0)).into(),
            size_balanced: bytes(self.size_bytes(studio, 1)).into(),
            size_small: bytes(self.size_bytes(studio, 2)).into(),
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
            done_size: bytes(self.size_bytes(studio, self.quality)).into(),
            empty: clips == 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
