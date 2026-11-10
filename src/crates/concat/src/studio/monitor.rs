// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors
//! The monitor, playback, the phone's throw and the preview axis.

use super::*;

impl Studio {
    // ── the monitor ──

    /// Asks the monitor for the frame at the playhead; see
    /// `MonitorPane`.
    pub fn request_preview(&mut self) {
        self.handle(crate::panes::Msg::Monitor(
            crate::panes::monitor::MonitorMsg::Request,
        ));
    }

    /// What the monitor draws at the playhead: the document flattened with
    /// its titles, plus the frame's own additions - a look being shown, a
    /// cutout being painted - and the session's settings. None without a
    /// project.
    pub fn preview_clips(
        &mut self,
    ) -> Option<(
        std::sync::Arc<Vec<concat_export::ExportClip>>,
        concat_project::DocumentSettings,
    )> {
        let session = self.session.as_ref()?;
        // A library file on the monitor: its clip alone, flattened once.
        if let Some(source) = self.source.as_ref() {
            return Some((std::sync::Arc::clone(&source.clips), session.settings()));
        }
        // The echo when there is one: a picture being dragged on the stage
        // is drawn where the pointer has it, not where the document last
        // had it. Same flattening the session does for itself, project
        // folder included: that is what names a cutout's masks, and
        // without it the monitor would show every cutout as shot.
        let project_dir = std::path::PathBuf::from(session.path());
        let (width, height) = self.output_size();
        // The flattening is the document's, not the frame's: kept between
        // frames of an unchanged document and handed over by pointer, so
        // playback and scrubbing neither flatten nor plan again. A gesture
        // in flight - the echo - is not the document, and flattens fresh.
        let cached = self.echo.is_none()
            && self
                .flat
                .as_ref()
                .is_some_and(|(at, w, h, _)| *at == self.revision && *w == width && *h == height);
        let mut painted_all = true;
        let clips = if cached {
            std::sync::Arc::clone(&self.flat.as_ref().expect("checked").3)
        } else {
            let mut clips = concat_export::flatten::flatten_timeline_in(
                self.project(),
                None,
                Some(&project_dir),
            );
            // Titles, painted to pictures and rejoined; see concat-host's titles.
            if self.echo.is_some() {
                // A change in flight: the titles are painted in memory at
                // the monitor's size and handed to it as pixels - no file
                // per pointer step - and the blocks they report are scaled
                // back up to the output's terms, which the stage measures
                // in. Nothing is written until the change is committed.
                let (shown_w, shown_h) = crate::panes::monitor::MonitorPane::frame_size(
                    self.quality_of(),
                    (width, height),
                );
                let scale = f64::from(shown_w) / f64::from(width);
                let up = |px: u32| (f64::from(px) / scale).round() as u32;
                let up_off = |px: i32| (f64::from(px) / scale).round() as i32;
                for title in self
                    .host
                    .titles
                    .clips_live(self.project(), shown_w, shown_h)
                {
                    self.title_blocks.insert(
                        title.clip_id,
                        (
                            (up(title.block.0), up(title.block.1)),
                            (up_off(title.offset.0), up_off(title.offset.1)),
                        ),
                    );
                    if let Some(frame) = title.frame {
                        self.host
                            .monitor
                            .hold_still(std::path::Path::new(&title.clip.path), frame);
                    }
                    clips.push(title.clip);
                }
            } else {
                // The titles painted already; the rest are painted on a
                // thread of their own and the frame asked for again when
                // they are in. A transcribed interview is hundreds of
                // captions, and painting them here held the window.
                let (titles, complete) = self.host.titles.ready(self.project(), width, height);
                for title in titles {
                    self.title_blocks
                        .insert(title.clip_id, (title.block, title.offset));
                    clips.push(title.clip);
                }
                painted_all = complete;
            }
            let clips = std::sync::Arc::new(clips);
            // Kept only once every title is in it, so the frame after the
            // painter is done flattens again and finds them.
            if self.echo.is_none() && painted_all {
                self.flat = Some((self.revision, width, height, std::sync::Arc::clone(&clips)));
            }
            clips
        };
        // The frame's own additions - a look being shown, a cutout being
        // painted - go on a copy, so the kept list stays the document's.
        let mut own: Option<Vec<concat_export::ExportClip>> = None;
        // The look being shown before it is laid down goes into this
        // frame only, as the layer it would be: over every track, the
        // whole way along, at full strength. The timeline is as it was.
        if let Some(filter_id) = self.audition.clone() {
            let effects = vec![AppliedFilter::new(filter_id)];
            let track = clips
                .iter()
                .map(|flat| flat.track)
                .max()
                .map_or(0, |top| top + 1);
            own.get_or_insert_with(|| (*clips).clone())
                .push(concat_export::ExportClip {
                    muted: true,
                    volume: 0.0,
                    effects,
                    has_audio: Some(false),
                    ..concat_export::ExportClip::blank(
                        concat_export::ClipKind::Layer,
                        0.0,
                        f64::from(self.duration()).max(1.0),
                        track,
                    )
                });
        }
        // While the brushes are out, the clip being painted is drawn with
        // its cutout tinted over the whole picture rather than cut, so a
        // stroke shows what it grabbed against what it left.
        if self.painting
            && let Some(target) = self.paint_target()
            && let Some(media) = self.project().media_by_id(&target.media_id)
        {
            let (path, start) = (media.path.clone(), target.start);
            if let Some(flat) = own
                .get_or_insert_with(|| (*clips).clone())
                .iter_mut()
                .find(|flat| flat.path == path && (flat.start - start).abs() < 1e-6)
            {
                flat.highlighted = true;
            }
        }
        let clips = own.map(std::sync::Arc::new).unwrap_or(clips);
        let settings = session.settings();
        if !painted_all {
            self.paint_titles(width, height);
        }
        Some((clips, settings))
    }

    /// Paints the titles the monitor does not have yet, at `width` by
    /// `height`, on every core and off the window's thread, and asks for
    /// the frame again when they are done. One run at a time: a frame asked
    /// for meanwhile shows what is painted and leaves the run to finish.
    fn paint_titles(&mut self, width: u32, height: u32) {
        // Once a document revision and size: a title that will not paint
        // stays missing, and must not send the painter round again for
        // every frame after.
        let attempt = (self.revision, width, height);
        if self.titles_painting || self.titles_tried == Some(attempt) {
            return;
        }
        self.titles_painting = true;
        self.titles_tried = Some(attempt);
        let titles = std::sync::Arc::clone(&self.host.titles);
        let project = self.project().clone();
        crate::host::spawn_in_project_publishing(
            move || {
                titles.clips(&project, width, height);
            },
            |studio, _, _, ()| {
                studio.titles_painting = false;
                studio.flat = None;
                studio.request_preview();
            },
            |_, _, _| {},
        );
    }

    // ── playback ──

    pub fn play_toggle(&mut self) {
        self.stop_glide();
        // A voiceover take owns the transport while it runs: Space, the
        // monitor's button and the keymap's play pause or resume the take
        // and the picture together, never the picture alone.
        if let Some(msg) = self.voiceover.on_play() {
            self.handle(crate::panes::Msg::Voiceover(msg));
            return;
        }
        // The monitor's play button and Space play what it shows.
        if self.source.is_some() {
            self.source_toggle();
            return;
        }
        if self.playing {
            self.pause();
            return;
        }
        if self.session.is_none() {
            return;
        }
        // Playing from the tail would sit there doing nothing, so the
        // button rewinds first - what every editor does.
        if self.playhead >= self.duration() {
            self.playhead = 0.0;
        }
        // The transport owns the monitor from here; a frame from under a
        // pointer that happens to be over the lanes must not stay on it.
        self.end_hover();
        self.playing = true;
        log::debug!("playback: playing from {:.3}s", self.playhead);
        self.host.playback.play(f64::from(self.playhead));
        self.wake_meters();
        // The clock is the audio device's. This looks at it far more often
        // than any timeline's frame rate and acts only when the frame under
        // it changes, so each frame is asked for within a few milliseconds of
        // its time instead of whenever a fixed 30 Hz beat next comes round -
        // which at 24, 25 or 60 fps is unevenly late, and reads as judder.
        let mut shown: Option<i64> = None;
        self.transport.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(8),
            move || {
                crate::host::Shell::with(|shell, app| {
                    {
                        let mut studio = shell.studio.borrow_mut();
                        let end = studio.duration();
                        let position = studio.host.playback.position_now() as f32;
                        let fps = studio.project().active().video.rate();
                        let frame = (f64::from(position) * fps + 1e-6).floor() as i64;
                        if position < end && shown == Some(frame) {
                            return;
                        }
                        shown = Some(frame);
                        studio.playhead = position.min(end);
                        // The view follows: a playhead that runs off the
                        // right edge, or sits off the left, pages the lanes
                        // to it, the way every editor keeps the cut in view.
                        // A phone's lanes glide instead, the playhead held
                        // at their middle.
                        if studio.lanes.centred {
                            let playhead = studio.playhead;
                            studio.lanes.recentre(playhead);
                        } else if let Some((low, high)) = studio.lanes.published_span() {
                            let screen = (high - low) / 3.0;
                            let left = studio.lanes.scroll_left;
                            if screen > 0.0
                                && (studio.playhead > left + screen * 0.95
                                    || studio.playhead < left)
                            {
                                studio.lanes.scroll_left =
                                    (studio.playhead - screen * 0.05).max(0.0);
                            }
                        }
                        if position >= end {
                            studio.pause();
                        } else {
                            studio.request_preview();
                        }
                    }
                    shell.studio.borrow().publish_lanes(&app, &shell.models);
                });
            },
        );
    }

    pub fn pause(&mut self) {
        // The picture stopped from anywhere - a seek, a step, an edit -
        // stops the take with it. The panel's own pause comes through here
        // with the pane taken out of the studio, so it is not sent back.
        let at_end = self.playhead >= self.duration();
        if let Some(msg) = self.voiceover.on_pause(at_end) {
            self.handle(crate::panes::Msg::Voiceover(msg));
            return;
        }
        if self.playing {
            log::debug!("playback: paused at {:.3}s", self.playhead);
        }
        let was_playing = self.playing;
        self.playing = false;
        self.transport.stop();
        self.host.playback.pause();
        // The textures playback streamed through are let go once it stops.
        if was_playing {
            self.host.monitor.shrink();
        }
    }

    /// Moves the playhead, the transport with it, and asks for the frame.
    /// Never before zero; past the end of the content only when Settings
    /// lets it, which is the default, so the ruler can be clicked beyond the
    /// last clip and something placed at the playhead there.
    pub fn seek(&mut self, seconds: f32) {
        // The playhead belongs to a voiceover take until it stops: moved
        // under it, the words would land away from the frames they were
        // spoken to.
        if let Some(first) = self.voiceover.on_seek() {
            if first {
                self.notify(&t("voiceover.playheadHeld"), false);
            }
            return;
        }
        // The timeline was clicked: it has the monitor back.
        self.close_source();
        self.playhead = seconds.max(0.0);
        if self.prefs.playhead_stops_at_end {
            self.playhead = self.playhead.min(self.duration().max(0.0));
        }
        // A phone's lanes move under a playhead held at their middle.
        self.lanes.recentre(self.playhead);
        self.host.playback.seek(f64::from(self.playhead));
        self.request_preview();
    }

    // ── the phone's throw ──

    /// A finger landed on the phone's lanes, or two began a pinch: a glide
    /// still running stops where it is, the way a list does under a
    /// thumb, and the scrubs from here are the new gesture's samples.
    pub fn finger_held(&mut self) {
        self.stop_glide();
        self.coast.tracker.clear();
        self.coast.tracking = true;
    }

    /// A scrub: the playhead goes to `seconds`, and while a finger is on
    /// the phone's lanes, where it went is a sample of how fast they move.
    pub fn scrub(&mut self, seconds: f32) {
        self.seek(seconds.max(0.0));
        if self.coast.tracking {
            let at = self.coast.clock();
            self.coast.tracker.push(at, f64::from(self.playhead));
        }
    }

    /// The finger that dragged the phone's lanes along the cut has let go:
    /// they carry on at its speed and run down, scrubbing as they go.
    /// Nothing while playing - the transport owns the playhead then - and
    /// nothing for a finger that stopped before it lifted.
    pub fn finger_lifted(&mut self) {
        let tracking = std::mem::replace(&mut self.coast.tracking, false);
        if !tracking || self.playing || !self.lanes.centred {
            return;
        }
        // Thrown in pixels, so the friction and the thresholds mean the
        // same at every zoom; each step goes back to seconds at the zoom
        // of the moment.
        let per_pixel = f64::from(self.lanes.seconds_per_pixel.max(1e-6));
        let speed = self.coast.tracker.velocity(self.coast.clock()) / per_pixel;
        let Some(glide) = crate::panes::glide::Glide::thrown(speed) else {
            return;
        };
        let limit = self.duration().max(self.playhead);
        self.coast.glide = Some((glide, std::time::Instant::now(), limit));
        // A beat a frame, only while it moves: nothing is drawn for a glide
        // that is not running.
        self.glide_beat.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            || {
                crate::host::Shell::with(|shell, app| {
                    shell.studio.borrow_mut().glide_step();
                    shell.studio.borrow_mut().refresh_art();
                    shell.studio.borrow().publish_lanes(&app, &shell.models);
                });
            },
        );
    }

    /// One beat of a glide: as far as it has run since the last, never
    /// before the start of the cut or past `limit`, and the end of it
    /// when it has slowed to nothing or reached either.
    fn glide_step(&mut self) {
        let Some((mut glide, since, limit)) = self.coast.glide else {
            self.glide_beat.stop();
            return;
        };
        let (moved, going) = glide.step(since.elapsed().as_secs_f64());
        let per_pixel = f64::from(self.lanes.seconds_per_pixel);
        let wanted = f64::from(self.playhead) + moved * per_pixel;
        let limit = f64::from(limit);
        let reached = wanted <= 0.0 || wanted >= limit;
        self.seek(wanted.clamp(0.0, limit) as f32);
        if going && !reached {
            self.coast.glide = Some((glide, since, limit as f32));
        } else {
            self.stop_glide();
        }
    }

    /// Ends a glide, if one is running.
    pub fn stop_glide(&mut self) {
        self.coast.glide = None;
        self.glide_beat.stop();
    }

    // ── the preview axis ──

    /// The instant the monitor shows: the pointer's while it crosses the
    /// lanes with the preview axis on, else the playhead's.
    pub fn preview_time(&self) -> f32 {
        if let Some(source) = self.source.as_ref() {
            return source.time as f32;
        }
        self.hover.unwrap_or(self.playhead)
    }

    /// The pointer is over the lanes at `seconds`. Nothing while playing -
    /// the transport owns the monitor then - and nothing with the axis off.
    pub fn hover(&mut self, seconds: f32) {
        if !self.prefs.preview_axis || self.playing {
            return;
        }
        let seconds = seconds.max(0.0);
        if self.hover == Some(seconds) {
            return;
        }
        self.hover = Some(seconds);
        if self.prefs.preview_axis_audio {
            // A burst of the sound under the pointer: the engine plays from
            // there and is stopped a beat later, then put back where the
            // playhead is, so the next Play starts where the cut says.
            self.host.playback.play(f64::from(seconds));
            let playhead = f64::from(self.playhead);
            self.hover_hush.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::from_millis(120),
                move || {
                    crate::host::Shell::with(|shell, _| {
                        let studio = shell.studio.borrow();
                        if !studio.playing {
                            studio.host.playback.pause();
                            studio.host.playback.seek(playhead);
                        }
                    });
                },
            );
        }
        self.request_preview();
    }

    /// The pointer left the lanes, or the axis went off: the monitor is the
    /// playhead's again.
    pub fn end_hover(&mut self) {
        if self.hover.take().is_some() {
            self.request_preview();
        }
    }
}
