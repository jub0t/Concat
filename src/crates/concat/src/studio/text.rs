// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! Fonts, and the styled words of a title.

use super::*;

impl Studio {
    // ── fonts ──

    /// The families a title can be set in, in the order the picker shows
    /// them: the bundled face; then the user's own - the project's fonts
    /// and the app's installed ones, once each, in alphabetical order; then
    /// the system's, likewise, less any already listed.
    pub fn font_families(&self) -> Vec<String> {
        let mut out = vec![concat_text::BUNDLED_FAMILY.to_owned()];
        let mut own: Vec<String> = self
            .installed_fonts
            .iter()
            .map(|(family, _)| family.clone())
            .chain(
                self.session
                    .as_ref()
                    .map(|session| session.project().fonts.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|font| font.family),
            )
            .filter(|family| !family.is_empty())
            .collect();
        own.sort_by_cached_key(|family| family.to_lowercase());
        own.dedup();
        let mut held: std::collections::HashSet<String> = out
            .iter()
            .map(|family| family.to_ascii_lowercase())
            .collect();
        for family in own.into_iter().chain(self.system_fonts.iter().cloned()) {
            if held.insert(family.to_ascii_lowercase()) {
                out.push(family);
            }
        }
        out
    }

    /// What [`Studio::font_families`] is made of, hashed: the same number
    /// for the same list, without building it.
    pub(super) fn fonts_fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.installed_fonts.hash(&mut hasher);
        if let Some(session) = &self.session {
            for font in &session.project().fonts {
                font.family.hash(&mut hasher);
            }
        }
        self.system_fonts.hash(&mut hasher);
        hasher.finish()
    }

    /// The machine's families have been read; see `system_fonts`.
    pub fn system_fonts_known(&mut self, families: Vec<String>) {
        self.system_fonts = families;
    }

    /// Font files the user picked: each is copied to the app's own folder,
    /// so it is there next session and for every project, and registered
    /// on this project; the selected title is set in the first of them.
    pub fn import_fonts(&mut self, paths: Vec<std::path::PathBuf>) {
        log::info!("fonts: importing {} file(s)", paths.len());
        let mut first: Option<String> = None;
        for path in paths {
            let families = concat_text::families_in(&path);
            if families.is_empty() {
                self.notify(&tf("studio.notAFont", &[&path.display()]), true);
                continue;
            }
            let Some(installed) = presets::install_font_file(&self.host.dirs, &path) else {
                self.notify(&tf("studio.notAFont", &[&path.display()]), true);
                continue;
            };
            let file = installed.to_string_lossy().into_owned();
            for family in families {
                if !self.installed_fonts.iter().any(|(held, _)| *held == family) {
                    self.installed_fonts.push((family.clone(), file.clone()));
                }
                self.ensure_font(&family);
                first.get_or_insert(family);
            }
        }
        if let Some(family) = first {
            self.notify(&tf("studio.fontImported", &[&family]), false);
            if self.sole_selection().is_some() {
                self.clip_set_text(ClipTextField::FontFamily, &family);
                self.clip_commit();
            }
        }
    }

    /// Registers the file behind one of the app's installed families on the
    /// project, once, so the painter - which reads a project's fonts and
    /// the system's, and nothing else - finds it. Nothing for a family the
    /// project already carries, or one that is not the app's to register.
    pub(super) fn ensure_font(&mut self, family: &str) {
        let family = family.trim().trim_matches('"');
        let carried = self.session.as_ref().is_some_and(|session| {
            session
                .project()
                .fonts
                .iter()
                .any(|font| font.family == family)
        });
        if carried {
            return;
        }
        let Some((_, path)) = self.installed_fonts.iter().find(|(held, _)| held == family) else {
            return;
        };
        let (family, path) = (family.to_owned(), path.clone());
        self.apply(Command::AddFont { family, path });
    }

    pub fn clip_set_colour(&mut self, field: ClipTextField, value: slint::Color) {
        if field == ClipTextField::Color
            && let Some((id, start, end)) = self.styled_words()
        {
            let patch = model::RunStyle {
                color: Some(hex_of(value)),
                ..model::RunStyle::default()
            };
            self.style_words(&id, start, end, &patch);
            return;
        }
        let ids = self.edit_targets();
        if ids.is_empty() {
            return;
        }
        self.commit_targets = ids.clone();
        self.begin_echo();
        for id in ids {
            let Some(clip) = self.echo_clip_mut(&id) else {
                continue;
            };
            if clip.kind != model::ClipKind::Text {
                continue;
            }
            let text = clip.text.get_or_insert_with(TextStyle::default);
            match field {
                ClipTextField::Color => text.color = hex_of(value),
                ClipTextField::StrokeColor => text.stroke_color = hex_rgba(value),
                ClipTextField::Background => text.background = hex_with_alpha(value),
                _ => {}
            }
        }
    }

    /// One of a shape's colours from the Shape section: 0 the fill, 1 the
    /// blend's second colour, 2 the outline - a line's own colour - and 3
    /// the shadow. Each keeps its alpha.
    pub fn clip_shape_colour(&mut self, which: i32, value: slint::Color) {
        let ids = self.edit_targets();
        if ids.is_empty() {
            return;
        }
        self.commit_targets = ids.clone();
        self.begin_echo();
        let hex = crate::format::hex_rgba(value);
        for id in ids {
            let Some(clip) = self.echo_clip_mut(&id) else {
                continue;
            };
            if clip.kind != model::ClipKind::Shape {
                continue;
            }
            let shape = clip.shape.get_or_insert_with(model::ShapeStyle::default);
            match which {
                0 => shape.fill = hex.clone(),
                1 => {
                    shape
                        .gradient
                        .get_or_insert_with(model::ShapeGradient::default)
                        .color = hex.clone();
                }
                2 => shape.stroke = hex.clone(),
                3 => {
                    shape
                        .shadow
                        .get_or_insert_with(model::ShapeShadow::default)
                        .color = hex.clone()
                }
                _ => {}
            }
        }
    }

    // ── styled words ──
    //
    // A title is one look, and over it some of its words can wear their
    // own: drag over them in the Text tab's box and the colour, B/I/U/S
    // and size go on those words alone, the way After Effects recolours
    // a dragged-over word. They are runs on the title's style, kept
    // with the words as they are typed; see `concat_project::runs`.

    /// The text box's selection, from the box: where its anchor and its
    /// cursor are, as bytes into its words. A selection with nothing in it
    /// - a caret - styles the whole title again.
    pub fn select_words(&mut self, anchor: i32, cursor: i32) {
        let Some(id) = self.sole_selection() else {
            self.words = None;
            return;
        };
        let Some(content) = self
            .clip(&id)
            .filter(|clip| clip.kind == model::ClipKind::Text)
            .and_then(|clip| clip.text.as_ref())
            .map(|text| text.content.clone())
        else {
            self.words = None;
            return;
        };
        let (anchor, cursor) = (char_at(&content, anchor), char_at(&content, cursor));
        let (start, end) = (anchor.min(cursor), anchor.max(cursor));
        self.words = (start < end).then_some((id, start, end));
    }

    /// The selected words the styling controls are aimed at, while there
    /// are some on the title still selected alone: (clip, start, end) in
    /// characters, clamped to its words.
    pub(super) fn styled_words(&self) -> Option<(String, usize, usize)> {
        let (id, start, end) = self.words.as_ref()?;
        if self.sole_selection().as_deref() != Some(id.as_str()) {
            return None;
        }
        let text = self
            .clip(id)
            .filter(|clip| clip.kind == model::ClipKind::Text)?
            .text
            .as_ref()?;
        let count = text.char_count();
        let (start, end) = ((*start).min(count), (*end).min(count));
        (start < end).then(|| (id.clone(), start, end))
    }

    /// What a typography control means for some words: the override it
    /// lays on them, or none for a control that is the whole title's -
    /// its outline, its box, its alignment. A size is kept relative to the
    /// title's, so the words stay bigger by as much when the title grows.
    pub(super) fn word_patch(
        &self,
        id: &str,
        field: ClipField,
        value: f64,
    ) -> Option<model::RunStyle> {
        let mut patch = model::RunStyle::default();
        match field {
            ClipField::FontSize => {
                let size = self.clip(id)?.text.as_ref()?.font_size.max(0.005);
                patch.font_scale = Some(value.clamp(0.01, 0.5) / size);
            }
            ClipField::FontWeight => patch.font_weight = Some(value.clamp(100.0, 900.0)),
            ClipField::Italic => patch.italic = Some(value != 0.0),
            ClipField::Underline => patch.underline = Some(value != 0.0),
            ClipField::Strikethrough => patch.strikethrough = Some(value != 0.0),
            _ => return None,
        }
        Some(patch)
    }

    /// Lays `patch` over the characters `start..end` of a title, on the
    /// echo, for the inspector's commit to land as one step.
    pub(super) fn style_words(
        &mut self,
        id: &str,
        start: usize,
        end: usize,
        patch: &model::RunStyle,
    ) {
        self.commit_targets = vec![id.to_owned()];
        self.begin_echo();
        if let Some(text) = self.echo_clip_mut(id).and_then(|clip| clip.text.as_mut()) {
            text.style_range(start, end, patch);
        }
    }

    /// The selected words back in the title's own look.
    pub fn reset_words(&mut self) {
        let Some((id, start, end)) = self.styled_words() else {
            return;
        };
        self.commit_targets = vec![id.clone()];
        self.begin_echo();
        if let Some(text) = self.echo_clip_mut(&id).and_then(|clip| clip.text.as_mut()) {
            text.clear_range(start, end);
        }
        self.clip_commit();
    }

    /// The selected title's look becomes the one new titles start from,
    /// and the one generated captions are set in: its family, weight,
    /// colours, stroke, shadow and background, never its words. Kept in
    /// the preferences, so it outlives the project.
    /// https://github.com/jub0t/Concat/issues/224
    pub fn use_title_style_for_new(&mut self) {
        let Some(mut style) = self
            .edit_targets()
            .first()
            .and_then(|id| self.clip(id))
            .filter(|clip| clip.kind == model::ClipKind::Text)
            .and_then(|clip| clip.text.clone())
        else {
            return;
        };
        // The words are this title's own, and so are the looks of some of
        // them: a run says nothing about another title's words.
        style.content.clear();
        style.runs.clear();
        self.prefs.title_style = Some(style);
        self.prefs.save(&self.host.dirs);
        self.notify(&t("textPanel.newTitlesUseThisStyle"), false);
    }

    /// The inspector's gesture is over: what differs between the echo and
    /// the session becomes commands, as one batch.
    /// An inspector control changed something on the echo and wants it
    /// committed. Held, not applied: a knob commits on every move, and the
    /// command, the mix and the publish happen once the moves pause. The
    /// monitor follows the echo in the meantime.
    pub fn clip_commit(&mut self) {
        self.commit_pending = true;
        self.request_preview();
        self.commit_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_millis(120),
            || {
                crate::host::Shell::with(|shell, app| {
                    shell.studio.borrow_mut().flush_commit();
                    shell.studio.borrow().publish(&app, &shell.models);
                });
            },
        );
    }

    /// Lands the held commit now, if there is one. Called before anything
    /// that would drop the echo it lives on - a command from elsewhere, an
    /// undo, a gesture on the lanes or the stage.
    pub fn flush_commit(&mut self) {
        if !self.commit_pending {
            return;
        }
        self.commit_pending = false;
        self.commit_timer.stop();
        self.commit_now();
    }

    pub(super) fn commit_now(&mut self) {
        // The clips the echo was written for, whatever is selected now; see
        // `commit_targets`. The selection is the fallback for a commit asked
        // for with nothing written, which has nothing to land anyway.
        let mut ids = std::mem::take(&mut self.commit_targets);
        if ids.is_empty() {
            ids = self.sole_selection().into_iter().collect();
        }
        let Some(echo) = self.echo.take() else {
            return;
        };
        let mut commands = Vec::new();
        for id in &ids {
            let (Some(after), Some(before)) = (
                echo.active().clip(id),
                self.session
                    .as_ref()
                    .and_then(|session| session.project().active().clip(id)),
            ) else {
                continue;
            };
            commands.extend(clip_commands(id, before, after, self.prefs.magnetic));
        }
        if commands.is_empty() {
            return;
        }
        // One undo step per gesture, not per pointer move: a commit that
        // changes the same things on the same clips as the last, within a
        // moment of it, folds into the last one's step. The editor does the
        // folding; this decides when a pause is long enough to be a new
        // gesture on the same knob.
        let key = format!("{}:{}", ids.join("+"), commit_key(&commands));
        let now = std::time::Instant::now();
        let continues = self
            .last_commit
            .as_ref()
            .is_some_and(|(last, at)| *last == key && now.duration_since(*at).as_millis() < 900);
        if !continues && let Some(session) = self.session.as_mut() {
            session.end_gesture();
        }
        let command = match commands.len() {
            1 => commands.remove(0),
            _ => Command::Batch { commands },
        };
        self.apply_within(&key, command);
        self.last_commit = Some((key, now));
    }
}
