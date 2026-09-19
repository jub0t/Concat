// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The project sheet: the Details panel's Modify button, as a form.

use concat_project::Command;
use slint::SharedString;

use crate::studio::{OUTPUTS, START_RATES, Studio};
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
    Apply,
}

/// The project sheet's state.
#[derive(Default)]
pub struct ProjectPane {
    pub open: bool,
    pub name: String,
    /// Row in `OUTPUTS`, or -1 for a frame the list does not carry.
    pub size: i32,
    /// Row in `START_RATES`.
    pub rate: usize,
}

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
                *self = ProjectPane {
                    open: true,
                    name: studio.project_name.clone(),
                    size: OUTPUTS
                        .iter()
                        .position(|size| *size == (width as i32, height as i32))
                        .map_or(-1, |index| index as i32),
                    rate: START_RATES
                        .iter()
                        .position(|(_, n, d)| (*n, *d) == (num, den))
                        .unwrap_or(3),
                };
            }
            ProjectMsg::Close => self.open = false,
            ProjectMsg::NameEdited(name) => self.name = name,
            ProjectMsg::SizeChanged(index) => self.size = index,
            ProjectMsg::RateChanged(index) => {
                self.rate = (index.max(0) as usize).min(START_RATES.len() - 1);
            }
            ProjectMsg::Apply => self.apply(studio),
        }
    }

    /// Applies the sheet and closes it. The name is the project's; the frame
    /// and the rate are the active timeline's, and go as one edit so an undo
    /// takes both back together. The frame goes the way the monitor's picker
    /// sends it, so the two cannot disagree about what a size means.
    fn apply(&mut self, studio: &mut Studio) {
        let sheet = std::mem::take(self);
        let name = sheet.name.trim().to_owned();
        let size = usize::try_from(sheet.size)
            .ok()
            .and_then(|index| OUTPUTS.get(index).copied());
        let (_, num, den) = START_RATES[sheet.rate.min(START_RATES.len() - 1)];
        let Some(session) = studio.session.as_mut() else {
            return;
        };
        let mut video = session.video();
        if let Some((width, height)) = size {
            video.width = width as u32;
            video.height = height as u32;
        }
        video.rate_num = num;
        video.rate_den = den;
        session.prepare_save((!name.is_empty()).then_some(name.as_str()));
        if !name.is_empty() {
            studio.project_name = name;
        }
        let timeline_id = studio.project().active_timeline_id.clone();
        studio.apply(Command::SetTimelineVideo { timeline_id, video });
        studio.request_preview();
    }

    /// The sheet as Slint shows it.
    pub fn data(&self, studio: &Studio) -> ProjectSheetData {
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
            size: self.size,
            rate: self.rate as i32,
        }
    }
}
