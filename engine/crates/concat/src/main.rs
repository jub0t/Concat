// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Concat's editor window, in Slint.
//!
//! This file is the wiring: it starts the engine's services, builds the
//! window, and binds every callback the `.slint` tree exposes to the state
//! in [`studio`]. The state reads the engine's project and writes commands
//! to it; nothing here decides what an edit means.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use slint::{ModelRc, SharedString, VecModel};

// `DataTransfer` is what a drag carries. Slint keeps the platform's drag
// object opaque and leaves building and reading one to the host language.
use slint::private_unstable_api::re_exports::DataTransfer;

// Everything the .slint tree exports, in a module of its own: the workspace
// lints every public item for documentation, and the generated accessors are
// thousands of public items nobody documents. The allow covers them and
// nothing in this file.
#[allow(missing_docs)]
mod ui {
    slint::include_modules!();
}

mod chips;
mod dock;
mod format;
mod gpu;
mod host;
mod prefs;
mod studio;
mod sysinfo;

use dock::{Dock, SEAT_MIN_GRAB, SEAT_MIN_H, SEAT_MIN_W};
use host::{Host, Shell};
use studio::{LANGUAGES, Models, OUTPUTS, RESOLUTIONS, START_RATES, Studio};
use ui::*;

fn main() -> Result<(), slint::PlatformError> {
    // The device the renderer and the monitor share. Taken first, because
    // the backend is selected with it.
    let gpu = gpu::Gpu::acquire();
    if gpu.is_none() {
        eprintln!("concat: no GPU adapter; the monitor composites on the CPU");
    }

    let mut selector = slint::BackendSelector::new().backend_name("winit".into());
    selector = match &gpu {
        Some(gpu) => selector.require_wgpu_29(gpu.configuration()),
        None => {
            // Without a shared device, ask for the platform's own API by
            // name: Skia picks its surface from a cfg chain, and requiring
            // one turns a silent fall back to the CPU rasteriser into a
            // refusal to start, which is a fault you can see.
            #[cfg(target_os = "macos")]
            {
                selector.require_metal()
            }
            #[cfg(target_family = "windows")]
            {
                selector.require_d3d()
            }
            #[cfg(not(any(target_os = "macos", target_family = "windows")))]
            {
                selector
            }
        }
    };

    // The custom title bar. On macOS the native bar is hidden and the traffic
    // lights are overlaid on the strip the UI draws. Other platforms keep
    // their decorations for now.
    #[cfg(target_os = "macos")]
    {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        selector = selector.with_winit_window_attributes_hook(|attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        });
    }
    selector.select()?;

    let host = match Host::start(gpu) {
        Ok(host) => host,
        Err(error) => {
            eprintln!("concat: {error}");
            return Err(slint::PlatformError::Other(error));
        }
    };

    let app = App::new()?;
    app.set_macos(cfg!(target_os = "macos"));

    let studio = Studio::new(host);
    let dark = studio.prefs.dark.unwrap_or(false);
    app.global::<Theme>().set_dark(dark);

    let shell = Rc::new(Shell {
        app: app.as_weak(),
        studio: RefCell::new(studio),
        models: Models::new(),
    });
    Shell::install(shell.clone());

    // Handed over once, here, and never replaced: a fresh model is a reset,
    // and a reset rebuilds every row that hangs off it.
    {
        let editor = app.global::<Editor>();
        let models = &shell.models;
        editor.set_timeline_tabs(ModelRc::from(models.tabs.clone()));
        editor.set_tracks(ModelRc::from(models.tracks.clone()));
        editor.set_clips(ModelRc::from(models.clips.clone()));
        editor.set_stage_items(ModelRc::from(models.stage.clone()));
        editor.set_stage_guides(ModelRc::from(models.guides.clone()));
        editor.set_media(ModelRc::from(models.media.clone()));
        editor.set_video_effects(ModelRc::from(models.video_effects.clone()));
        editor.set_audio_effects(ModelRc::from(models.audio_effects.clone()));
        editor.set_menu_items(ModelRc::from(models.menu.clone()));
        editor.set_av_items(ModelRc::from(models.av.clone()));
        app.set_app_menu_items(ModelRc::from(models.bar.clone()));
        app.set_transcribers(ModelRc::from(models.transcribers.clone()));
        app.set_voices(ModelRc::from(models.voices.clone()));
        editor.set_seats(ModelRc::from(models.seats.clone()));
        editor.set_dividers(ModelRc::from(models.dividers.clone()));
        app.set_recents(ModelRc::from(models.recents.clone()));
    }

    // Settings > About's block, gathered once: nothing in it changes while
    // the process runs.
    let facts = sysinfo::system_facts();
    app.set_system_report(
        facts
            .iter()
            .map(|(label, value)| format!("{label}: {value}"))
            .collect::<Vec<_>>()
            .join("\n")
            .into(),
    );
    app.set_system_facts(ModelRc::from(Rc::new(VecModel::from(
        facts
            .into_iter()
            .map(|(label, value)| SystemFactData {
                label: label.into(),
                value: value.into(),
            })
            .collect::<Vec<_>>(),
    ))));

    // The ladders' labels, handed over once; the index the form reports back
    // is what carries the meaning.
    app.set_start_resolutions(ModelRc::from(Rc::new(VecModel::from(
        RESOLUTIONS
            .iter()
            .map(|(label, _, _)| SharedString::from(*label))
            .collect::<Vec<_>>(),
    ))));
    app.set_start_rates(ModelRc::from(Rc::new(VecModel::from(
        START_RATES
            .iter()
            .map(|(label, _, _)| SharedString::from(*label))
            .collect::<Vec<_>>(),
    ))));
    app.set_languages(ModelRc::from(Rc::new(VecModel::from(
        LANGUAGES
            .iter()
            .map(|label| SharedString::from(*label))
            .collect::<Vec<_>>(),
    ))));

    // The strip's drag region and double-click. Only the winit window can do
    // either; the scene graph forwards the gestures here.
    app.on_titlebar_begin_drag({
        let weak = app.as_weak();
        move || {
            use slint::winit_030::WinitWindowAccessor;
            if let Some(app) = weak.upgrade() {
                app.window().with_winit_window(|window| {
                    let _ = window.drag_window();
                });
            }
        }
    });
    app.on_titlebar_toggle_maximize({
        let weak = app.as_weak();
        move || {
            use slint::winit_030::WinitWindowAccessor;
            if let Some(app) = weak.upgrade() {
                app.window().with_winit_window(|window| {
                    window.set_maximized(!window.is_maximized());
                });
            }
        }
    });

    // Mutate, then republish. Every handler is one of these three: the whole
    // window, the lanes alone (for the handlers a pointer drives directly,
    // which arrive as a stream), or the dock alone (a gutter drag).
    macro_rules! handler {
        ($publish:ident, |$state:ident $(, $arg:ident : $ty:ty)*| $body:block) => {{
            move |$($arg : $ty),*| {
                Shell::with(|shell, app| {
                    {
                        let mut $state = shell.studio.borrow_mut();
                        $body
                    }
                    shell.studio.borrow_mut().refresh_art();
                    shell.studio.borrow().$publish(&app, &shell.models);
                });
            }
        }};
    }
    macro_rules! on_window {
        ($($handler:tt)*) => { handler!(publish, $($handler)*) };
    }
    macro_rules! on_lanes {
        ($($handler:tt)*) => { handler!(publish_lanes, $($handler)*) };
    }
    macro_rules! on_dock {
        ($($handler:tt)*) => { handler!(publish_dock, $($handler)*) };
    }

    let editor = app.global::<Editor>();

    // ── the launch screen ──
    app.on_start_name_edited(on_window!(|state, name: SharedString| {
        state.start.name = name.to_string();
    }));
    app.on_start_location_edited(on_window!(|state, path: SharedString| {
        state.start.location = path.to_string();
    }));
    app.on_start_resolution_changed(on_window!(|state, index: i32| {
        state.start.resolution = (index.max(0) as usize).min(RESOLUTIONS.len() - 1);
    }));
    app.on_start_rate_changed(on_window!(|state, index: i32| {
        state.start.rate = (index.max(0) as usize).min(START_RATES.len() - 1);
    }));
    app.on_start_dismiss_error(on_window!(|state| {
        state.start.error.clear();
    }));
    app.on_start_browse(on_window!(|state| {
        let mut dialog = rfd::FileDialog::new().set_title("Where should the project folder go?");
        if !state.start.location.is_empty() {
            dialog = dialog.set_directory(&state.start.location);
        }
        if let Some(folder) = dialog.pick_folder() {
            state.start.location = folder.to_string_lossy().into_owned();
        }
    }));
    app.on_start_create(on_window!(|state| {
        state.create_project();
    }));
    app.on_start_open_recent(on_window!(|state, path: SharedString| {
        state.open_recent(path.as_str());
    }));
    app.on_start_forget_recent(on_window!(|state, path: SharedString| {
        state.forget_recent(path.as_str());
    }));

    // ── the workspace's arrangement ──
    editor.on_workspace_resized(on_dock!(|state, width: f32, height: f32| {
        state.workspace = (width, height);
    }));
    editor.on_dock_set(on_dock!(|state, seat: i32, kind: PaneKind| {
        let Some(path) = state.dock.leaf_path(seat.max(0) as usize) else {
            return;
        };
        if let Dock::Leaf(held) = state.dock.at_mut(&path) {
            *held = kind;
        }
    }));
    editor.on_dock_dropped(on_dock!(|state, from: i32, onto: i32, side: DockSide| {
        let (from, onto) = (from.max(0) as usize, onto.max(0) as usize);
        if from == onto {
            return;
        }
        let (Some(taken), Some(displaced)) = (state.dock.kind_at(from), state.dock.kind_at(onto))
        else {
            return;
        };
        if side == DockSide::Centre {
            for (index, kind) in [(from, displaced), (onto, taken)] {
                let Some(path) = state.dock.leaf_path(index) else {
                    continue;
                };
                if let Dock::Leaf(held) = state.dock.at_mut(&path) {
                    *held = kind;
                }
            }
            return;
        }
        let (Some(onto_path), Some(from_path)) =
            (state.dock.leaf_path(onto), state.dock.leaf_path(from))
        else {
            return;
        };
        state.dock.split_leaf(&onto_path, taken, side);
        state.dock.remove_leaf(&from_path);
    }));
    editor.on_dock_add(on_dock!(|state, kind: PaneKind| {
        let seats = state.dock_layout().seats;
        let Some(biggest) = seats
            .iter()
            .max_by(|a, b| (a.width * a.height).total_cmp(&(b.width * b.height)))
        else {
            return;
        };
        let across = biggest.width >= SEAT_MIN_W * 2.0;
        let down = biggest.height >= SEAT_MIN_H * 2.0;
        let side = if biggest.width >= biggest.height && (across || !down) {
            DockSide::Right
        } else {
            DockSide::Bottom
        };
        let Some(path) = state.dock.leaf_path(biggest.index.max(0) as usize) else {
            return;
        };
        state.dock.split_leaf(&path, kind, side);
    }));
    editor.on_dock_remove(on_dock!(|state, seat: i32| {
        let Some(path) = state.dock.leaf_path(seat.max(0) as usize) else {
            return;
        };
        state.dock.remove_leaf(&path);
    }));
    editor.on_divider_pressed(on_dock!(|state, index: i32| {
        let index = index.max(0) as usize;
        state.divider_press = match (state.split_ratio(index), state.split_extent(index)) {
            (Some(ratio), Some(extent)) => Some((index, ratio, extent)),
            _ => None,
        };
    }));
    editor.on_divider_dragged(on_dock!(|state, index: i32, delta: f32| {
        let Some((held, from, extent)) = state.divider_press else {
            return;
        };
        if held != index.max(0) as usize || extent <= 0.0 {
            return;
        }
        let Some(path) = state.dock.split_path(held) else {
            return;
        };
        let Dock::Split { columns, ratio, .. } = state.dock.at_mut(&path) else {
            return;
        };
        let wanted = if *columns { SEAT_MIN_W } else { SEAT_MIN_H };
        let floor = if wanted * 2.0 <= extent {
            wanted
        } else {
            SEAT_MIN_GRAB.min(extent / 2.0)
        } / extent;
        *ratio = (from + delta / extent).clamp(floor, 1.0 - floor);
    }));

    // ── the bin ──
    editor.on_media_filter_changed(on_window!(|state, filter: MediaFilter| {
        state.set_media_filter(filter);
    }));
    editor.on_media_select(on_window!(|state, id: i32, additive: bool| {
        state.media_select(id, additive);
    }));
    editor.on_media_band_selected(on_window!(
        |state,
         columns: i32,
         from_col: i32,
         to_col: i32,
         from_row: i32,
         to_row: i32,
         additive: bool| {
            state.media_band(columns, from_col, to_col, from_row, to_row, additive);
        }
    ));
    editor.on_media_remove(on_window!(|state, id: i32| {
        state.media_remove(id);
    }));
    editor.on_media_remove_selected(on_window!(|state| {
        state.media_remove_selected();
    }));
    editor.on_import_media(on_window!(|state| {
        if state.session.is_none() {
            return;
        }
        let picked = rfd::FileDialog::new()
            .set_title("Import media")
            .add_filter(
                "Media",
                &[
                    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "aac", "m4a", "flac",
                    "ogg", "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff",
                ],
            )
            .pick_files();
        if let Some(paths) = picked {
            state.import(paths);
        }
    }));
    editor.on_media_activate(on_window!(|state, id: i32| {
        state.place_at_playhead(&format!("media:{id}"));
    }));
    editor.on_library_add_text(on_window!(|state| {
        state.place_at_playhead("text:default:Title");
    }));
    editor.on_library_apply_filter(on_window!(
        |state, id: SharedString, _label: SharedString| {
            state.apply_catalogue(id.as_str(), false);
        }
    ));
    editor.on_library_apply_effect(on_window!(|state, id: SharedString| {
        state.apply_catalogue(id.as_str(), true);
    }));
    editor.on_library_apply_transition(on_window!(|state, id: SharedString| {
        state.apply_transition(id.as_str());
    }));
    editor.on_library_save_template(on_window!(|state| {
        state.save_template();
    }));

    // ── the inspector's effect stacks ──
    editor.on_add_effect(on_window!(|state, _audio: bool| {
        state.notify("Pick an effect or filter from the library", false);
    }));
    editor.on_remove_effect(on_window!(|state, id: i32| {
        state.remove_effect(id);
    }));

    // ── tabs ──
    editor.on_tab_selected(on_window!(|state, index: i32| {
        let Some(id) = state
            .project()
            .timelines
            .get(index.max(0) as usize)
            .map(|timeline| timeline.id.clone())
        else {
            return;
        };
        state.selection.clear();
        state.apply(concat_project::Command::SelectTimeline { timeline_id: id });
    }));
    editor.on_tab_renamed(on_window!(|state, index: i32, name: SharedString| {
        let trimmed = name.trim().to_string();
        let Some(id) = state
            .project()
            .timelines
            .get(index.max(0) as usize)
            .map(|timeline| timeline.id.clone())
        else {
            return;
        };
        if !trimmed.is_empty() {
            state.apply(concat_project::Command::RenameTimeline {
                timeline_id: id,
                name: trimmed,
            });
        }
    }));
    editor.on_tab_added(on_window!(|state| {
        if let Some(id) = state.apply(concat_project::Command::AddTimeline) {
            state.selection.clear();
            state.apply(concat_project::Command::SelectTimeline { timeline_id: id });
        }
    }));
    editor.on_tab_close_requested(on_window!(|state, index: i32| {
        let Some(id) = state
            .project()
            .timelines
            .get(index.max(0) as usize)
            .map(|timeline| timeline.id.clone())
        else {
            return;
        };
        state.selection.clear();
        state.apply(concat_project::Command::RemoveTimeline { timeline_id: id });
    }));

    // ── the tray ──
    editor.on_tool_changed(on_window!(|state, tool: TimelineTool| {
        state.tool = tool;
    }));
    editor.on_snap_changed(on_window!(|state, snap: bool| {
        state.snap = snap;
    }));
    editor.on_add_track(on_window!(|state| {
        state.apply(concat_project::Command::AddTrack);
    }));
    editor.on_delete_selected(on_window!(|state| {
        state.delete_selected();
    }));
    editor.on_split(on_window!(|state| {
        let at = state.playhead;
        state.split_at(at, true);
    }));
    editor.on_merge(on_window!(|state| {
        state.merge();
    }));

    // ── the view ──
    editor.on_scrubbed(on_lanes!(|state, seconds: f32| {
        state.seek(seconds.max(0.0));
    }));
    editor.on_scrolled(on_lanes!(|state, seconds: f32| {
        state.scroll_left = seconds.max(0.0);
    }));
    editor.on_zoom(on_lanes!(|state, factor: f32, anchor: f32| {
        let before = state.seconds_per_pixel;
        let after = (before * factor).clamp(0.000_5, 1.5);
        state.seconds_per_pixel = after;
        if anchor >= 0.0 {
            state.scroll_left = (anchor - (anchor - state.scroll_left) * (after / before)).max(0.0);
        }
    }));
    editor.on_zoom_to_fit(on_lanes!(|state, width: f32| {
        let span = state.duration().max(1.0) * 1.05;
        if width > 1.0 {
            state.seconds_per_pixel = (span / width).clamp(0.000_5, 1.5);
            state.scroll_left = 0.0;
        }
    }));

    // ── lanes ──
    editor.on_track_flag_changed(on_window!(
        |state, row: i32, visible: bool, muted: bool, locked: bool| {
            state.track_flags(row, visible, muted, locked);
        }
    ));
    editor.on_track_renamed(on_window!(|state, row: i32, name: SharedString| {
        let trimmed = name.trim().to_string();
        let Some(id) = state.row_track(row).map(|track| track.id.clone()) else {
            return;
        };
        if !trimmed.is_empty() {
            state.apply(concat_project::Command::RenameTrack {
                track_id: id,
                name: trimmed,
            });
        }
    }));
    editor.on_track_sized(on_window!(|state, row: i32, size: TrackSize| {
        state.set_lane_size(row, size);
    }));
    editor.on_track_removed(on_window!(|state, row: i32| {
        let Some(id) = state.row_track(row).map(|track| track.id.clone()) else {
            return;
        };
        state.apply(concat_project::Command::RemoveTrack { track_id: id });
    }));

    // ── the gestures ──
    editor.on_clip_pressed(on_lanes!(|state,
                                      id: SharedString,
                                      additive: bool,
                                      edge: i32| {
        state.clip_pressed(id.as_str(), additive, edge);
    }));
    editor.on_clip_dragged(on_lanes!(|state, seconds: f32, pixels: f32| {
        state.clip_dragged(seconds, pixels);
    }));
    editor.on_clip_released(on_window!(|state| {
        state.clip_released();
    }));

    // ── drag and drop from the library ──
    editor.on_drag_hovered(on_lanes!(|state,
                                      payload: SharedString,
                                      seconds: f32,
                                      y: f32| {
        let row = state.row_at(y);
        state.drop = state.plan(payload.as_str(), seconds, row);
    }));
    editor.on_dropped(on_window!(|state,
                                  payload: SharedString,
                                  seconds: f32,
                                  y: f32| {
        state.drop = None;
        let row = state.row_at(y);
        if let Some(plan) = state.plan(payload.as_str(), seconds, row) {
            state.place(&plan);
        }
    }));
    editor.on_razored(on_window!(|state, id: SharedString, seconds: f32| {
        let Some(clip) = state.clip(id.as_str()).cloned() else {
            return;
        };
        if state.locked(&clip.track_id) {
            return;
        }
        state.selection = vec![id.to_string()];
        state.split_at(seconds, true);
    }));
    editor.on_band_selected(on_lanes!(
        |state, from: f32, to: f32, from_y: f32, to_y: f32, additive: bool| {
            let (from_row, to_row) = (state.row_at(from_y), state.row_at(to_y));
            let caught: Vec<String> = state
                .timeline()
                .clips
                .iter()
                .filter(|clip| {
                    let row = state.row_of(&clip.track_id);
                    row >= from_row
                        && row <= to_row
                        && (clip.start + clip.duration) as f32 >= from
                        && clip.start as f32 <= to
                        && !state.locked(&clip.track_id)
                })
                .map(|clip| clip.id.clone())
                .collect();
            if additive {
                for id in caught {
                    if !state.selection.contains(&id) {
                        state.selection.push(id);
                    }
                }
            } else {
                state.selection = caught;
            }
        }
    ));

    // ── the inspector ──
    editor.on_clip_set(on_lanes!(|state, field: ClipField, value: f32| {
        state.clip_set(field, value);
    }));
    editor.on_clip_set_text(on_lanes!(
        |state, field: ClipTextField, value: SharedString| {
            state.clip_set_text(field, value.as_str());
        }
    ));
    editor.on_clip_set_colour(on_lanes!(
        |state, field: ClipTextField, value: slint::Color| {
            state.clip_set_colour(field, value);
        }
    ));
    editor.on_clip_commit(on_window!(|state| {
        state.clip_commit();
    }));

    // ── the monitor ──
    editor.on_seek(on_lanes!(|state, seconds: f32| {
        state.pause();
        state.seek(seconds);
    }));
    editor.on_step_frames(on_lanes!(|state, frames: f32| {
        state.pause();
        let fps = state.frame_rate().round().max(1.0);
        let at = (state.playhead * fps).round() + frames;
        state.seek(at / fps);
    }));
    editor.on_ratio_changed(on_window!(|state, index: i32| {
        state.set_output((index.max(0) as usize).min(OUTPUTS.len() - 1));
    }));
    editor.on_quality_changed(on_window!(|state, index: i32| {
        state.quality = (index.max(0) as usize).min(2);
        state.request_preview();
    }));
    editor.on_play_toggled(on_window!(|state| {
        state.play_toggle();
    }));

    // ── the stage ──
    editor.on_stage_pressed(on_window!(|state, x: f32, y: f32, additive: bool| {
        state.stage_pressed(x, y, additive);
    }));
    editor.on_stage_grip_pressed(on_window!(
        |state, id: SharedString, grip: i32, x: f32, y: f32| {
            state.stage_grip_pressed(id.as_str(), grip, x, y);
        }
    ));
    editor.on_stage_dragged(on_lanes!(|state, x: f32, y: f32, snap: bool| {
        state.stage_dragged(x, y, snap);
    }));
    editor.on_stage_released(on_window!(|state| {
        state.stage_released();
    }));

    // ── the context menu ──
    editor.on_clip_context(on_window!(|state, id: SharedString| {
        state.menu_token += 1;
        if state.clip(id.as_str()).is_none() {
            state.menu_target = None;
            return;
        }
        if !state.selection.iter().any(|held| held == id.as_str()) {
            state.selection = vec![id.to_string()];
        }
        state.menu_target = Some(id.to_string());
    }));
    editor.on_menu_selected(on_window!(|state, action: SharedString| {
        // The clip the menu was opened on; failing that, the one clip that
        // is selected, which is what the menu was showing anyway.
        let target = state.menu_target.clone().or_else(|| state.sole_selection());
        let Some(id) = target else {
            return;
        };
        let Some(clip) = state.clip(&id).cloned() else {
            return;
        };
        match action.as_str() {
            "copy" => state.clipboard = Some(clip),
            "duplicate" => state.duplicate(&clip),
            "paste" => {
                if let Some(held) = state.clipboard.clone() {
                    let mut source = held;
                    // Pasted after the clip that was right-clicked, on its lane.
                    source.track_id = clip.track_id.clone();
                    source.start = clip.start + clip.duration - source.duration;
                    state.duplicate(&source);
                }
            }
            "split" => {
                let at = state.playhead;
                state.selection = vec![id];
                state.split_at(at, true);
            }
            "mute" => {
                let volume = if clip.volume <= 0.0 { 1.0 } else { 0.0 };
                state.apply(concat_project::Command::UpdateClip {
                    clip_id: id,
                    patch: concat_project::commands::ClipPatch {
                        volume: Some(volume),
                        ..Default::default()
                    },
                });
            }
            "lock" => state.toggle_lock(&clip.track_id),
            "delete" => {
                state.apply(concat_project::Command::RemoveClips { clip_ids: vec![id] });
                state.menu_target = None;
            }
            _ => {}
        }
    }));

    // ── the dialogs ──
    app.on_export_clicked(on_window!(|state| {
        state.export.open = true;
        state.export.phase = ExportPhase::Idle;
        state.export.message.clear();
    }));
    app.on_open_settings(on_window!(|state| {
        state.refresh_models();
        state.settings.open = true;
    }));
    // The theme is one bool on the Theme global, and every colour in the
    // tree is a binding away from it; it is also remembered.
    app.on_settings_theme_changed({
        move |dark| {
            Shell::with(|shell, app| {
                app.global::<Theme>().set_dark(dark);
                let mut studio = shell.studio.borrow_mut();
                studio.prefs.dark = Some(dark);
                studio.prefs.save(&studio.host.dirs);
            });
        }
    });
    app.on_export_closed(on_window!(|state| {
        state.export.open = false;
    }));
    app.on_settings_closed(on_window!(|state| {
        state.settings.open = false;
    }));
    app.on_export_name_edited(on_window!(|state, name: SharedString| {
        state.export.name = name.to_string();
    }));
    app.on_export_resolution_changed(on_window!(|state, index: i32| {
        state.export.resolution = (index.max(0) as usize).min(3);
    }));
    app.on_export_rate_changed(on_window!(|state, index: i32| {
        state.export.rate = (index.max(0) as usize).min(2);
    }));
    app.on_export_quality_changed(on_window!(|state, index: i32| {
        state.export.quality = (index.max(0) as usize).min(2);
    }));
    app.on_export_again(on_window!(|state| {
        state.export.phase = ExportPhase::Idle;
        state.export.progress = 0.0;
    }));
    app.on_export_browse(on_window!(|state| {
        let mut dialog = rfd::FileDialog::new().set_title("Export to");
        if !state.export.folder.is_empty() {
            dialog = dialog.set_directory(&state.export.folder);
        }
        if let Some(folder) = dialog.pick_folder() {
            state.export.folder = folder.to_string_lossy().into_owned();
        }
    }));
    app.on_export_reveal(on_window!(|state| {
        if !state.export.written.is_empty()
            && let Err(error) = opener::reveal(&state.export.written)
        {
            state.notify(&format!("Could not show the file: {error}"), true);
        }
    }));
    app.on_export_cancel(on_window!(|state| {
        state.export_cancel();
    }));
    app.on_export_start(on_window!(|state| {
        state.export_start();
    }));

    // ── settings ──
    app.on_settings_page_changed(on_window!(|state, index: i32| {
        state.settings.tab = index;
    }));
    app.on_settings_language_changed(on_window!(|state, index: i32| {
        state.settings.language = index.max(0) as usize;
        state.prefs.language = Some(state.settings.language);
        state.prefs.save(&state.host.dirs);
    }));
    app.on_settings_transcribe_language_changed(on_window!(|state, index: i32| {
        state.settings.transcribe_language = index
            .max(0)
            .min(studio::TRANSCRIBE_LANGUAGES.len() as i32 - 1);
        state.prefs.transcribe_language = Some(state.settings.transcribe_language);
        state.prefs.save(&state.host.dirs);
    }));
    app.on_model_activated(on_window!(|state, id: SharedString| {
        state.model_activate(id.as_str());
    }));
    app.on_model_download(on_window!(|state, id: SharedString| {
        state.model_download(id.as_str());
    }));
    app.on_model_cancel(on_window!(|state, id: SharedString| {
        state.model_cancel(id.as_str());
    }));
    app.on_model_remove(on_window!(|state, id: SharedString| {
        state.model_remove(id.as_str());
    }));

    // ── the tray's A/V menu ──
    editor.on_av_tools(on_window!(|state| {
        state.av_token += 1;
    }));
    editor.on_av_selected(on_window!(|state, action: SharedString| {
        let Some(id) = state.selection.first().cloned() else {
            return;
        };
        match action.as_str() {
            "captions" => state.caption_selected(),
            "speak" => state.speak_selected(),
            "detach" => {
                state.apply(concat_project::Command::DetachAudio { clip_id: id });
            }
            "reattach" => {
                state.apply(concat_project::Command::ReattachAudio { clip_id: id });
            }
            _ => {}
        }
    }));

    // ── the title-bar menus ──
    app.on_menu_opened(on_window!(|state, index: i32| {
        state.open_menu = index;
        state.menu_bar_token += 1;
    }));
    app.on_app_menu_selected({
        move |action| {
            Shell::with(|shell, app| {
                {
                    let mut state = shell.studio.borrow_mut();
                    state.open_menu = -1;
                    match action.as_str() {
                        "add-selected" => state.add_selected_media(),
                        "import" => {
                            if let Some(paths) = rfd::FileDialog::new()
                                .set_title("Import media")
                                .pick_files()
                            {
                                state.import(paths);
                            }
                        }
                        "export" => {
                            state.export.open = true;
                            state.export.phase = ExportPhase::Idle;
                            state.export.message.clear();
                        }
                        "template" => state.save_template(),
                        "speech" => state.speak_selected(),
                        "settings" => {
                            state.refresh_models();
                            state.settings.open = true;
                        }
                        "close-project" => state.close_project(),
                        "undo" => state.undo(),
                        "redo" => state.redo(),
                        "snap" => state.snap = !state.snap,
                        "zoom-in" => {
                            state.seconds_per_pixel = (state.seconds_per_pixel / 1.4).max(0.000_5)
                        }
                        "zoom-out" => {
                            state.seconds_per_pixel = (state.seconds_per_pixel * 1.4).min(1.5)
                        }
                        "start" => {
                            state.pause();
                            state.seek(0.0);
                        }
                        "end" => {
                            state.pause();
                            let end = state.duration();
                            state.seek(end);
                        }
                        "delete" => state.delete_selected(),
                        "split" => {
                            let at = state.playhead;
                            state.split_at(at, false);
                        }
                        "save" => state.save(true),
                        _ => {}
                    }
                }
                if action == "close-window" {
                    shell.studio.borrow_mut().close_project();
                    app.window().hide().ok();
                    return;
                }
                shell.studio.borrow_mut().refresh_art();
                shell.studio.borrow().publish(&app, &shell.models);
            });
        }
    });

    // --- the pieces Slint cannot express ---------------------------------
    app.global::<Curves>().on_ease(format::bezier_y_at_x);
    app.global::<Curves>()
        .on_parse(|text, fallback| format::parse_bezier(text.as_str(), fallback));
    app.global::<Fmt>()
        .on_parse_timecode(|text| format::parse_timecode(text.as_str()));
    app.global::<Fmt>().on_tick_interval(format::tick_interval);
    app.global::<Fmt>()
        .on_parse_frames(|text, rate| format::parse_frames(text.as_str(), rate));

    // The drag payloads: plain text, because a drag that says "media:12" is
    // one that can be read in a log.
    app.global::<Payload>().on_of(DataTransfer::from);
    app.global::<Payload>()
        .on_text(|payload| payload.plain_text().unwrap_or_default());
    app.global::<Payload>().on_pane_seat(|text| {
        text.strip_prefix("pane:")
            .and_then(|rest| rest.split(':').next())
            .and_then(|seat| seat.parse().ok())
            .unwrap_or(-1)
    });

    // The picture the cursor carries, resolved through the same `incoming`
    // the drop uses, memoised by theme and payload.
    app.global::<Payload>().on_preview({
        let chips: RefCell<HashMap<String, slint::Image>> = RefCell::new(HashMap::new());
        move |payload| {
            let mut result = slint::Image::default();
            Shell::with(|shell, app| {
                let theme = app.global::<Theme>();
                let key = format!("{}{payload}", if theme.get_dark() { 'd' } else { 'l' });
                if let Some(chip) = chips.borrow().get(&key) {
                    result = chip.clone();
                    return;
                }
                if let Some(rest) = payload.strip_prefix("pane:") {
                    let mut fields = rest.splitn(3, ':').skip(1);
                    let label = fields.next().unwrap_or_default();
                    let slug = fields.next().unwrap_or_default();
                    let chip = slint::Image::load_from_svg_data(
                        chips::drag_chip_svg(
                            chips::pane_glyph(slug),
                            label,
                            "",
                            theme.get_accent(),
                            theme.get_field(),
                            theme.get_raised(),
                            theme.get_fg(),
                        )
                        .as_bytes(),
                    )
                    .unwrap_or_default();
                    chips.borrow_mut().insert(key, chip.clone());
                    result = chip;
                    return;
                }
                let studio = shell.studio.borrow();
                let Some(plan) = studio.incoming(payload.as_str()) else {
                    return;
                };
                let (mark, well) = match plan.kind {
                    ClipKind::Video => (theme.get_kind_video(), theme.get_kind_video_well()),
                    ClipKind::Audio => (theme.get_kind_audio(), theme.get_kind_audio_well()),
                    ClipKind::Image => (theme.get_kind_image(), theme.get_kind_image_well()),
                    ClipKind::Text => (theme.get_kind_text(), theme.get_kind_text_well()),
                    ClipKind::Filter => (theme.get_kind_filter(), theme.get_kind_filter_well()),
                };
                let wave = studio
                    .peaks
                    .get(&plan.media)
                    .filter(|_| plan.kind == ClipKind::Audio)
                    .map(|peaks| format::wave_path(peaks, 0.0, plan.duration, 1.0))
                    .unwrap_or_default();
                let document = chips::drag_chip_svg(
                    chips::chip_glyph(plan.kind),
                    &plan.label,
                    &wave,
                    mark,
                    well,
                    theme.get_raised(),
                    theme.get_fg(),
                );
                let chip =
                    slint::Image::load_from_svg_data(document.as_bytes()).unwrap_or_default();
                chips.borrow_mut().insert(key, chip.clone());
                result = chip;
            });
            result
        }
    });

    // The programme level meter has no feed yet: playback's mix does not
    // report levels. It stays parked at silence.
    app.on_meter_watched_changed({
        let weak = app.as_weak();
        move |_watched| {
            if let Some(app) = weak.upgrade() {
                app.global::<Editor>().set_level(0.0);
                app.global::<Editor>().set_peak(-1.0);
            }
        }
    });

    {
        shell.studio.borrow_mut().refresh_art();
        shell.studio.borrow().publish(&app, &shell.models);
    }

    app.run()
}
