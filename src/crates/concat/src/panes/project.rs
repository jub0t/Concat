// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The project sheet: the Details panel's Modify button, as a form.

use concat_project::Command;
use concat_project::model::ColorSpace;
use slint::SharedString;

use crate::studio::{OUTPUTS, RATES, Studio, custom_frame, custom_rate, fps_of};
use crate::ui::ProjectSheetData;

/// Everything that can happen to the project sheet.
#[derive(Clone, Debug)]
pub enum ProjectMsg {
    /// The Details panel's Modify button.
    Open,
    Close,
    NameEdited(String),
    SizeChanged(i32),
    RateChanged(i32),
    /// The custom frame's sides and rate, as typed.
    CustomWidth(f32),
    CustomHeight(f32),
    CustomFps(f32),
    /// A row of the colour list: SDR, HDR HLG, HDR PQ.
    ColorSpaceChanged(i32),
    Apply,
}

/// The project sheet's state.
#[derive(Default)]
pub struct ProjectPane {
    pub open: bool,
    pub name: String,
    /// Row in `OUTPUTS`, or one past its end for a custom frame.
    pub size: usize,
    /// Row in `RATES`, or one past its end for a custom rate.
    pub rate: usize,
    /// The custom frame, even on both sides.
    pub custom_size: (u32, u32),
    /// The custom rate, as the exact fraction.
    pub custom_rate: (i64, i64),
    /// What the timeline is output in.
    pub color_space: ColorSpace,
}

/// The colour list's rows, in the sheet's order.
const COLOR_SPACES: [ColorSpace; 3] = [ColorSpace::Sdr, ColorSpace::Hlg, ColorSpace::Pq];

impl ProjectPane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: ProjectMsg, studio: &mut Studio) {
        match msg {
            ProjectMsg::Open => {
                // The project and its active timeline as they stand.
                let (width, height) = studio.output_size();
                let video = studio.project().active().video;
                let (num, den) = (video.rate_num, video.rate_den);
                // A frame or a rate the lists do not carry opens as Custom
                // with the project's own numbers, exact: an Apply that only
                // renamed the project must not move its rate to the nearest
                // row, as it once moved every 23.976 project to 30.
                *self = ProjectPane {
                    open: true,
                    name: studio.project_name.clone(),
                    size: OUTPUTS
                        .iter()
                        .position(|size| *size == (width as i32, height as i32))
                        .unwrap_or(OUTPUTS.len()),
                    rate: RATES
                        .iter()
                        .position(|(_, n, d)| (*n, *d) == (num, den))
                        .unwrap_or(RATES.len()),
                    custom_size: (width, height),
                    custom_rate: (num, den),
                    color_space: video.color_space,
                };
            }
            ProjectMsg::Close => self.open = false,
            ProjectMsg::NameEdited(name) => self.name = name,
            ProjectMsg::SizeChanged(index) => {
                let index = (index.max(0) as usize).min(OUTPUTS.len());
                if index == OUTPUTS.len() && self.size < OUTPUTS.len() {
                    self.custom_size = self.frame();
                }
                self.size = index;
            }
            ProjectMsg::RateChanged(index) => {
                let index = (index.max(0) as usize).min(RATES.len());
                if index == RATES.len() && self.rate < RATES.len() {
                    self.custom_rate = self.rate();
                }
                self.rate = index;
            }
            ProjectMsg::CustomWidth(width) => {
                self.custom_size = custom_frame(width, self.custom_size.1 as f32);
            }
            ProjectMsg::CustomHeight(height) => {
                self.custom_size = custom_frame(self.custom_size.0 as f32, height);
            }
            ProjectMsg::CustomFps(fps) => self.custom_rate = custom_rate(f64::from(fps)),
            ProjectMsg::ColorSpaceChanged(index) => {
                self.color_space =
                    COLOR_SPACES[(index.max(0) as usize).min(COLOR_SPACES.len() - 1)];
            }
            ProjectMsg::Apply => self.apply(studio),
        }
    }

    /// Applies the sheet and closes it. The name is the project's; the
    /// frame, the rate and the colour are the active timeline's, and go as
    /// one edit so an undo takes them back together. The frame goes the way the monitor's picker
    /// sends it, so the two cannot disagree about what a size means.
    fn apply(&mut self, studio: &mut Studio) {
        let sheet = std::mem::take(self);
        let name = sheet.name.trim().to_owned();
        let (width, height) = sheet.frame();
        let (num, den) = sheet.rate();
        let Some(session) = studio.session.as_mut() else {
            return;
        };
        let mut video = session.video();
        video.width = width;
        video.height = height;
        video.rate_num = num;
        video.rate_den = den;
        video.color_space = sheet.color_space;
        session.prepare_save((!name.is_empty()).then_some(name.as_str()));
        if !name.is_empty() {
            studio.project_name = name;
        }
        let timeline_id = studio.project().active_timeline_id.clone();
        studio.apply(Command::SetTimelineVideo { timeline_id, video });
        studio.request_preview();
    }

    /// The frame the sheet describes: the row picked, or the typed one.
    fn frame(&self) -> (u32, u32) {
        OUTPUTS
            .get(self.size)
            .map_or(self.custom_size, |&(width, height)| {
                (width as u32, height as u32)
            })
    }

    /// The rate the sheet describes, as the exact fraction.
    fn rate(&self) -> (i64, i64) {
        RATES
            .get(self.rate)
            .map_or(self.custom_rate, |&(_, num, den)| (num, den))
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> ProjectSheetData {
        let (width, height) = self.frame();
        let (num, den) = self.rate();
        let folder: SharedString = studio
            .session
            .as_ref()
            .map(|session| session.path().to_owned())
            .unwrap_or_else(|| "—".to_owned())
            .into();
        ProjectSheetData {
            open: self.open,
            name: self.name.as_str().into(),
            folder,
            timeline: studio.timeline().name.as_str().into(),
            size: self.size as i32,
            rate: self.rate as i32,
            custom_width: width as f32,
            custom_height: height as f32,
            custom_fps: fps_of(num, den) as f32,
            rate_readout: format!("{num}/{den}").into(),
            color_space: COLOR_SPACES
                .iter()
                .position(|space| *space == self.color_space)
                .unwrap_or(0) as i32,
        }
    }
}
