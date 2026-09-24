// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Concat's editor window, in Slint.
//!
//! This file is the wiring: it starts the engine's services, builds the
//! window, and binds every callback the `.slint` tree exposes to the state
//! in [`studio`]. The state reads the engine's project and writes commands
//! to it; nothing here decides what an edit means.
//!
//! A library so that every entry point is a few lines: `main.rs` on the
//! desktop and on iOS, and the `concat-android` activity on Android. Each
//! one calls [`run`].

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
mod i18n;
mod platform;
/// What a phone's own crate installs before the window runs: the way to
/// the system's file picker. See `platform::pick_files_async`.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub use platform::{FilePicker, install_file_picker};
mod panes;
mod prefs;
mod presets;
mod studio;
mod sysinfo;

use dock::{Dock, SEAT_MIN_GRAB, SEAT_MIN_H, SEAT_MIN_W};
use host::{Host, Shell, on_ui};
use panes::Msg;
use panes::captions::CaptionsMsg;
use panes::export::ExportMsg;
use panes::media_bin::MediaMsg;
use panes::monitor::MonitorMsg;
use panes::project::ProjectMsg;
use panes::relink::RelinkMsg;
use panes::settings::SettingsMsg;
use panes::speech::SpeechMsg;
use panes::start::StartMsg;
use panes::timeline::TimelineMsg;
use studio::{ASPECTS, Models, OUTPUTS, SIZES, START_RATES, Studio};
use ui::*;

/// Opens this run's log file and makes it where the app writes things down.
///
/// Each entry point calls this before [`run`], because each knows where its
/// console is: a desktop has a standard error, a phone has logcat, and that
/// is what arrives here as `extra`. Everything after it - the engine's
/// warnings, a panic, the lines below - is written down in
/// `<app data>/logs/` as well as said out loud. See `concat_host::logs`.
pub fn open_logging(extra: Option<Box<dyn log::Log>>) {
    concat_host::logs::catch_panics();
    let opened = match concat_host::AppDirs::locate() {
        Ok(dirs) => concat_host::logs::open(&dirs, extra).map(|_| ()),
        // No directory at all: the facade still has to lead somewhere, or
        // every line disappears silently rather than loudly.
        Err(error) => concat_host::logs::open_console(extra).and(Err(error)),
    };
    if let Err(error) = opened {
        log::warn!("this run is not being written to a file: {error}");
    }
}

/// Says why the window could not start, somewhere the user will see it.
///
/// [`run`] returns its failure, and a desktop binary's `main` returns that
/// to the runtime, which prints it on standard error and exits 1 - exactly
/// what a packaged GUI build has no console for. Each entry point calls
/// this on the error before letting it go, so the reason is in the log and
/// in a dialog, not only in a console that was never there.
/// https://github.com/jub0t/Concat/issues/135
pub fn report_startup_failure(error: &slint::PlatformError) {
    let error = error.to_string();
    log::error!("could not start: {error}");
    platform::report_startup_failure(&error);
}

/// Builds the window, binds it to the engine, and runs it until it closes.
pub fn run() -> Result<(), slint::PlatformError> {
    // The shell doesn't exist yet - it needs the window the backend is about
    // to help create - but `Shell::with` is a no-op until `Shell::install`
    // runs, and by the time an OS drop can actually happen, it has. Import
    // goes through the same `Studio::import` the Import menu uses, so a
    // dropped file gets the same probe, the same failure notice, and the
    // same "no project open yet" no-op that a picked one does.
    let gpu = platform::select_backend(|paths| {
        Shell::with(|shell, app| {
            {
                let mut studio = shell.studio.borrow_mut();
                studio.handle(Msg::Media(MediaMsg::Import(paths)));
            }
            shell.studio.borrow_mut().refresh_art();
            shell.studio.borrow().publish(&app, &shell.models);
        });
    })?;

    let host = match Host::start(gpu) {
        Ok(host) => host,
        Err(error) => {
            log::error!("{error}");
            return Err(slint::PlatformError::Other(error));
        }
    };

    let app = App::new()?;
    app.set_macos(platform::MACOS);

    // The user's own packages - imported looks, and any effect folder they
    // or the community wrote - sit beside the built-ins from the first
    // frame. One that will not load is skipped, and its reason is the
    // first thing the window says.
    let mut studio = Studio::new(host);
    studio.reload_packages(false);
    studio.watch_packages();
    let dark = studio.prefs.dark.unwrap_or(true);
    app.global::<Theme>().set_dark(dark);
    // The accent is remembered by name and the names live on the Theme
    // global, so the index is looked up there rather than kept in Rust too.
    // A remembered hex is a colour picked by hand, which is the slot past
    // the names.
    {
        let theme = app.global::<Theme>();
        let names = theme.get_accent_names();
        let count = slint::Model::row_count(&names);
        match studio.prefs.custom_accent().and_then(format::parse_colour) {
            Some(colour) => {
                theme.set_accent_custom(colour);
                theme.set_accent_choice(count as i32);
            }
            None => {
                let index = studio
                    .prefs
                    .accent_index(slint::Model::iter(&names).map(|name| name.to_string()));
                theme.set_accent_choice(index as i32);
            }
        }
    }

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
        editor.set_catalogue_effects(ModelRc::from(models.catalogue_effects.clone()));
        editor.set_catalogue_filters(ModelRc::from(models.catalogue_filters.clone()));
        editor.set_catalogue_audio(ModelRc::from(models.catalogue_audio.clone()));
        editor.set_catalogue_transitions(ModelRc::from(models.catalogue_transitions.clone()));
        editor.set_effect_groups(ModelRc::from(models.effect_groups.clone()));
        editor.set_filter_groups(ModelRc::from(models.filter_groups.clone()));
        editor.set_audio_groups(ModelRc::from(models.audio_groups.clone()));
        editor.set_transition_groups(ModelRc::from(models.transition_groups.clone()));
        editor.set_applied_visual(ModelRc::from(models.applied_visual.clone()));
        editor.set_applied_audio(ModelRc::from(models.applied_audio.clone()));
        editor.set_visual_params(ModelRc::from(models.visual_params.clone()));
        editor.set_audio_params(ModelRc::from(models.audio_params.clone()));
        editor.set_adjust_params(ModelRc::from(models.adjust_params.clone()));
        app.global::<Keyframes>()
            .set_rows(ModelRc::from(models.key_rows.clone()));
        app.global::<KeyEditor>()
            .set_rows(ModelRc::from(models.key_editor_rows.clone()));
        editor.set_key_marks(ModelRc::from(models.key_marks.clone()));
        app.global::<Library>()
            .set_views(ModelRc::from(models.library_views.clone()));
        editor.set_menu_items(ModelRc::from(models.menu.clone()));
        app.set_caption_models(ModelRc::from(models.caption_models.clone()));
        app.set_speech_models(ModelRc::from(models.speech_models.clone()));
        app.set_speech_model_details(ModelRc::from(models.speech_model_details.clone()));
        app.set_speech_voices(ModelRc::from(models.speakers.clone()));
        app.set_speech_voice_details(ModelRc::from(models.speaker_details.clone()));
        app.set_speech_samples(ModelRc::from(models.speech_samples.clone()));
        app.set_speech_sample_waves(ModelRc::from(models.speech_sample_waves.clone()));
        app.set_speech_sample_details(ModelRc::from(models.speech_sample_details.clone()));
        app.set_app_menu_items(ModelRc::from(models.bar.clone()));
        app.set_transcribers(ModelRc::from(models.transcribers.clone()));
        app.set_voices(ModelRc::from(models.voices.clone()));
        editor.set_seats(ModelRc::from(models.seats.clone()));
        editor.set_dividers(ModelRc::from(models.dividers.clone()));
        app.set_recents(ModelRc::from(models.recents.clone()));
        editor.set_text_presets(ModelRc::from(models.text_presets.clone()));
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
    // is what carries the meaning. The shapes carry their ratio as well,
    // because the form draws each one as a box of its own proportions and
    // a label is a string Slint cannot do arithmetic with.
    app.set_start_aspects(ModelRc::from(Rc::new(VecModel::from(
        ASPECTS
            .iter()
            .map(|(label, w, h)| ChipOption {
                label: SharedString::from(*label),
                ratio: *w as f32 / *h as f32,
            })
            .collect::<Vec<_>>(),
    ))));
    // A size and a rate are words and nothing else, so their chips carry no
    // shape: a zero ratio is what the form draws nothing for.
    app.set_start_sizes(ModelRc::from(Rc::new(VecModel::from(
        SIZES
            .iter()
            .map(|(label, _)| ChipOption {
                label: SharedString::from(*label),
                ratio: 0.0,
            })
            .collect::<Vec<_>>(),
    ))));
    app.set_start_rates(ModelRc::from(Rc::new(VecModel::from(
        START_RATES
            .iter()
            .map(|(label, _, _)| ChipOption {
                label: SharedString::from(*label),
                ratio: 0.0,
            })
            .collect::<Vec<_>>(),
    ))));
    app.set_languages(ModelRc::from(Rc::new(VecModel::from(
        shell
            .studio
            .borrow()
            .languages
            .iter()
            .map(|language| SharedString::from(language.name.as_str()))
            .collect::<Vec<_>>(),
    ))));

    // The interface's words. Every `I18n.t` in the tree asks here, with the
    // English as the key; the answer is the active locale's line, or the
    // key. See i18n.rs.
    {
        let words = app.global::<I18n>();
        words.on_lookup(|_, key| i18n::t(&key).into());
        words.on_lookup1(|_, key, a| i18n::tf(&key, &[&a]).into());
        words.on_lookup2(|_, key, a, b| i18n::tf(&key, &[&a, &b]).into());
        // Uppercased here rather than in the tree: Slint has no text
        // transform, and casing is the language's rule and not the
        // interface's to guess at.
        words.on_lookup_upper(|_, key| i18n::t(&key).to_uppercase().into());
        words.set_lang(i18n::current().into());
    }

    // The strip's drag region and double-click. Only the platform's window
    // can do either; the scene graph forwards the gestures here.
    app.on_titlebar_begin_drag({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                platform::begin_drag(app.window());
            }
        }
    });
    app.on_titlebar_toggle_maximize({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                platform::toggle_maximize(app.window());
                app.set_window_maximized(platform::is_maximized(app.window()));
            }
        }
    });
    // The strip's own window buttons, on the platforms whose decorations
    // were taken off. Close goes the way the File menu's Close does - the
    // project is shut first, so an autosave in flight is not orphaned.
    app.on_titlebar_minimize({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                platform::minimize(app.window());
            }
        }
    });
    app.on_titlebar_close(|| {
        log::info!("close: titlebar X pressed");
        Shell::with(|shell, app| {
            shell.studio.borrow_mut().close_project();
            log::info!("close: project closed, hiding window");
            app.window().hide().ok();
            slint::quit_event_loop().ok();
            log::info!("close: quit_event_loop called");
        });
    });
    // System-driven close (Alt+F4, taskbar close): the same road out.
    app.window().on_close_requested(|| {
        log::info!("close: system close request (Alt+F4 / taskbar)");
        Shell::with(|shell, _app| {
            shell.studio.borrow_mut().close_project();
        });
        slint::quit_event_loop().ok();
        slint::CloseRequestResponse::HideWindow
    });
    // Maximised or not is read back on every resize rather than tracked:
    // the platform can maximise the window without us - a drag to the top
    // edge, Win+Up - and the size changing is the one signal every such
    // route has in common.
    app.on_window_resized({
        let weak = app.as_weak();
        move || {
            if let Some(app) = weak.upgrade() {
                let maximized = platform::is_maximized(app.window());
                if app.get_window_maximized() != maximized {
                    app.set_window_maximized(maximized);
                }
            }
        }
    });
    app.set_own_window_buttons(platform::OWN_WINDOW_BUTTONS);

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
        state.handle(Msg::Start(StartMsg::NameEdited(name.to_string())));
    }));
    app.on_start_location_edited(on_window!(|state, path: SharedString| {
        state.handle(Msg::Start(StartMsg::LocationEdited(path.to_string())));
    }));
    app.on_start_aspect_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Start(StartMsg::AspectChanged(index)));
    }));
    app.on_start_size_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Start(StartMsg::SizeChanged(index)));
    }));
    app.on_start_rate_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Start(StartMsg::RateChanged(index)));
    }));
    app.on_start_dismiss_error(on_window!(|state| {
        state.handle(Msg::Start(StartMsg::DismissError));
    }));
    app.on_start_browse(on_window!(|state| {
        state.handle(Msg::Start(StartMsg::Browse));
    }));
    app.on_start_create(on_window!(|state| {
        state.handle(Msg::Start(StartMsg::Create));
    }));
    app.on_start_open(on_window!(|state| {
        state.handle(Msg::Start(StartMsg::Open));
    }));
    app.on_start_open_recent(on_window!(|state, path: SharedString| {
        state.handle(Msg::Start(StartMsg::OpenRecent(path.to_string())));
    }));
    app.on_start_forget_recent(on_window!(|state, path: SharedString| {
        state.handle(Msg::Start(StartMsg::ForgetRecent(path.to_string())));
    }));

    // ── the workspace's arrangement ──
    editor.on_workspace_resized(on_dock!(|state, width: f32, height: f32| {
        state.workspace = (width, height);
        // A narrow window - a phone, a tablet held upright, a desktop
        // window squeezed - gets the compact dock: two seats, the monitor
        // over the timeline, with the switcher on each to bring the
        // library or the inspector into it. The wide dock waits, whole,
        // for the window to widen again; every dock operation works on
        // whichever is showing.
        state.set_compact(width < studio::COMPACT_WIDTH);
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
        state.handle(Msg::Media(MediaMsg::FilterChanged(filter)));
    }));
    editor.on_media_sort_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Media(MediaMsg::SortChanged(index)));
    }));
    editor.on_media_select(on_window!(|state, row: i32, additive: bool| {
        state.handle(Msg::Media(MediaMsg::Select { row, additive }));
    }));
    editor.on_media_band_selected(on_window!(
        |state,
         columns: i32,
         from_col: i32,
         to_col: i32,
         from_row: i32,
         to_row: i32,
         additive: bool| {
            state.handle(Msg::Media(MediaMsg::Band {
                columns,
                from_col,
                to_col,
                from_row,
                to_row,
                additive,
            }));
        }
    ));
    editor.on_media_remove(on_window!(|state, row: i32| {
        state.handle(Msg::Media(MediaMsg::Remove(row)));
    }));
    editor.on_media_remove_selected(on_window!(|state| {
        state.handle(Msg::Media(MediaMsg::RemoveSelected));
    }));
    // Not through the handler macro: nothing here changes the state, and
    // the import is asynchronous - a phone's picker is another screen, and
    // the files come back later, on whatever thread, to be imported on the
    // window's. A desktop's dialog answers before this returns.
    editor.on_import_media(|| {
        Shell::with(|shell, _| {
            if shell.studio.borrow().session.is_none() {
                return;
            }
            platform::pick_files_async(
                &i18n::t("Import media"),
                Some((
                    i18n::t("Media").as_str(),
                    &[
                        "mp4", "mov", "mkv", "webm", "avi", "m4v", "mp3", "wav", "aac", "m4a",
                        "flac", "ogg", "png", "jpg", "jpeg", "webp", "gif", "bmp", "tif", "tiff",
                    ],
                )),
                |paths| {
                    on_ui(move |studio, _, _| studio.handle(Msg::Media(MediaMsg::Import(paths))))
                },
            );
        });
    });
    editor.on_media_activate(on_window!(|state, id: i32| {
        state.place_at_playhead(&format!("media:{id}"));
    }));
    editor.on_library_add_text(on_window!(|state, preset: SharedString| {
        state.place_at_playhead(&format!("text:{preset}:Title"));
    }));
    // A filter is a layer over a span of the timeline; an effect goes on
    // the selected clip's chain, and audio on the sound's.
    editor.on_library_apply_filter(on_window!(
        |state, id: SharedString, label: SharedString| {
            state.place_filter_layer(id.as_str(), label.as_str());
        }
    ));
    editor.on_library_audition_filter(on_window!(|state, id: SharedString| {
        state.audition_catalogue(id.as_str());
    }));
    editor.on_library_apply_effect(on_window!(|state, id: SharedString| {
        state.apply_catalogue(id.as_str(), true);
    }));
    editor.on_library_apply_audio(on_window!(|state, id: SharedString| {
        state.apply_catalogue(id.as_str(), false);
    }));
    editor.on_library_apply_transition(on_window!(|state, id: SharedString| {
        state.apply_transition(id.as_str());
    }));
    editor.on_library_save_template(on_window!(|state| {
        state.save_template();
    }));
    editor.on_library_import_lut(on_window!(|state| {
        state.import_lut();
    }));
    editor.on_library_reload_packages(on_window!(|state| {
        state.reload_packages(true);
    }));

    // ── the inspector's effect stacks ──
    editor.on_add_effect(on_window!(|state, _audio: bool| {
        state.notify(&i18n::t("Pick an effect or filter from the library"), false);
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
    editor.on_tab_moved(on_window!(|state, from: i32, to: i32| {
        state.move_timeline(from, to);
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
        state.handle(Msg::Timeline(TimelineMsg::ToolChanged(tool)));
    }));
    editor.on_pan_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Timeline(TimelineMsg::PanChanged(on)));
    }));
    editor.on_snap_changed(on_window!(|state, snap: bool| {
        state.handle(Msg::Timeline(TimelineMsg::SnapChanged(snap)));
    }));
    editor.on_magnetic_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Timeline(TimelineMsg::MagneticChanged(on)));
    }));
    editor.on_preview_axis_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Timeline(TimelineMsg::PreviewAxisChanged(on)));
    }));
    editor.on_preview_axis_audio_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Timeline(TimelineMsg::PreviewAxisAudioChanged(on)));
    }));
    // The preview axis: on every move of the pointer across the lanes, so
    // it publishes the lanes and no more, the way a scrub does.
    editor.on_hovered(on_lanes!(|state, seconds: f32| {
        state.handle(Msg::Timeline(TimelineMsg::Hovered(seconds)));
    }));
    editor.on_hover_ended(on_lanes!(|state| {
        state.handle(Msg::Timeline(TimelineMsg::HoverEnded));
    }));
    editor.on_add_track(on_window!(|state| {
        state.apply(concat_project::Command::AddTrack);
    }));
    editor.on_delete_selected(on_window!(|state| {
        state.delete_selected();
    }));
    // The tray's buttons; the menu row and ⌘Z reach the same two methods
    // through `app_menu_selected` below.
    editor.on_undo(on_window!(|state| {
        state.undo();
    }));
    editor.on_redo(on_window!(|state| {
        state.redo();
    }));
    editor.on_split(on_window!(|state| {
        let at = state.playhead;
        state.split_at(at, true);
    }));
    editor.on_merge(on_window!(|state| {
        state.merge();
    }));

    // ── the view ──
    editor.on_key_mark_moved(on_lanes!(|state, from: f32, to: f32| {
        state.move_keys_at(from, to);
    }));
    editor.on_scrubbed(on_lanes!(|state, seconds: f32| {
        state.seek(seconds.max(0.0));
    }));
    editor.on_scrolled(on_lanes!(|state, seconds: f32| {
        state.handle(Msg::Timeline(TimelineMsg::Scrolled(seconds)));
    }));
    editor.on_zoom(on_lanes!(|state, factor: f32, anchor: f32| {
        state.handle(Msg::Timeline(TimelineMsg::Zoomed { factor, anchor }));
    }));
    editor.on_zoom_to_fit(on_lanes!(|state, width: f32| {
        state.handle(Msg::Timeline(TimelineMsg::ZoomToFit(width)));
    }));
    editor.on_lanes_resized(on_lanes!(|state, width: f32| {
        state.handle(Msg::Timeline(TimelineMsg::Resized(width)));
    }));

    // ── lanes ──
    editor.on_track_flag_changed(on_window!(
        |state, row: i32, visible: bool, muted: bool, locked: bool| {
            state.track_flags(row, visible, muted, locked);
        }
    ));
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
    editor.on_band_selected(on_lanes!(
        |state, from: f32, to: f32, from_y: f32, to_y: f32, additive: bool| {
            // As on a clip press and on the stage: what the inspector still
            // holds for the selection lands before the selection moves. A
            // press on the lanes' floor is the commonest way out of a
            // title's text box, and it ends here, with the band it drew -
            // usually an empty one that clears the selection.
            state.flush_commit();
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
    // The keyframe cluster. Its own global, so a row deep in a panel does
    // not have to be threaded a callback to reach here.
    {
        let keys = app.global::<Keyframes>();
        keys.on_toggle(on_lanes!(|state, field: ClipField| {
            state.toggle_key(field);
        }));
        keys.on_step(on_lanes!(|state, field: ClipField, delta: i32| {
            state.step_key(field, delta);
        }));
        keys.on_clear(on_lanes!(|state, field: ClipField| {
            state.clear_keys_on(field);
        }));
        // The same three verbs for the Adjust panel's knobs, which are
        // named rather than enumerated.
        keys.on_toggle_param(on_lanes!(|state, key: SharedString| {
            state.toggle_adjust_key(key.as_str());
        }));
        keys.on_step_param(on_lanes!(|state, key: SharedString, delta: i32| {
            state.step_adjust_key(key.as_str(), delta);
        }));
        keys.on_clear_param(on_lanes!(|state, key: SharedString| {
            state.clear_adjust_keys(key.as_str());
        }));
    }
    // The Keyframes panel's verbs; its rows go out with every publish.
    {
        let editor = app.global::<KeyEditor>();
        editor.on_jump(on_lanes!(|state, at: f32| {
            state.jump_to_key(at);
        }));
        editor.on_move_key(on_lanes!(|state,
                                      field: ClipField,
                                      param: SharedString,
                                      from: f32,
                                      to: f32| {
            state.move_key(field, param.as_str(), from, to);
        }));
        editor.on_set_ease(on_lanes!(|state,
                                      field: ClipField,
                                      param: SharedString,
                                      at: f32,
                                      x1: f32,
                                      y1: f32,
                                      x2: f32,
                                      y2: f32| {
            state.set_key_ease(field, param.as_str(), at, [x1, y1, x2, y2]);
        }));
        editor.on_step_all(on_lanes!(|state, delta: i32| {
            state.step_any_key(delta);
        }));
        editor.on_clear_all(on_lanes!(|state| {
            state.clear_all_keys();
        }));
    }

    // The effect libraries' search, shelves and stars. Rust does the
    // filtering, so the panel only reports what was pressed.
    {
        let library = app.global::<Library>();
        library.on_query_changed(on_window!(|state, shelf: i32, text: SharedString| {
            state.library_query(shelf, &text);
        }));
        library.on_group_changed(on_window!(|state, shelf: i32, index: i32| {
            state.library_group(shelf, index);
        }));
        library.on_favourites_changed(on_window!(|state, shelf: i32, on: bool| {
            state.library_favourites(shelf, on);
        }));
        library.on_category_changed(on_window!(|state, shelf: i32, category: SharedString| {
            state.library_category(shelf, &category);
        }));
        library.on_favourite(on_window!(|state, id: SharedString, on: bool| {
            state.library_favourite(&id, on);
        }));
    }

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
    // Lanes only: the commit is held and lands with a full publish of its
    // own once the control's moves pause; see `Studio::clip_commit`.
    editor.on_clip_commit(on_lanes!(|state| {
        state.clip_commit();
    }));

    // ── the chains ──
    editor.on_chain_add(on_window!(|state, audio: bool, id: SharedString| {
        state.chain_add(audio, id.as_str());
    }));
    editor.on_chain_toggle(on_window!(|state, audio: bool, index: i32| {
        state.chain_toggle(audio, index);
    }));
    editor.on_chain_move_by(on_window!(|state, audio: bool, index: i32, delta: i32| {
        state.chain_move(audio, index, delta);
    }));
    editor.on_chain_remove(on_window!(|state, audio: bool, index: i32| {
        state.chain_remove(audio, index);
    }));
    editor.on_chain_set_param(on_lanes!(
        |state, audio: bool, index: i32, key: SharedString, value: f32| {
            state.chain_set_param(audio, index, key.as_str(), value);
        }
    ));

    editor.on_adjust_set(on_lanes!(|state, key: SharedString, value: f32| {
        state.adjust_set(key.as_str(), value);
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
        state.handle(Msg::Monitor(MonitorMsg::QualityChanged(index)));
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

    // ── the cutout ──
    editor.on_cutout_mode(on_window!(|state, mode: i32| {
        state.cutout_mode(mode);
    }));
    editor.on_cutout_tool(on_window!(|state, index: i32| {
        state.cutout_tool(index);
    }));
    editor.on_cutout_size(on_lanes!(|state, size: f32| {
        state.cutout_size(size);
    }));
    editor.on_cutout_painting(on_window!(|state, on: bool| {
        state.cutout_painting(on);
    }));
    editor.on_cutout_subject(on_window!(|state, index: i32| {
        state.cutout_subject(index);
    }));
    editor.on_cutout_clear(on_window!(|state| {
        state.cutout_clear();
    }));
    editor.on_transition_remove(on_window!(|state| {
        state.remove_transition();
    }));
    editor.on_transition_duration_set(on_window!(|state, seconds: f32| {
        state.set_transition_duration(seconds as f64);
    }));

    // ── the context menu ──
    editor.on_clip_context(on_window!(|state, id: SharedString| {
        state.menu_token += 1;
        state.menu_media = None;
        if state.clip(id.as_str()).is_none() {
            state.menu_target = None;
            return;
        }
        if !state.selection.iter().any(|held| held == id.as_str()) {
            state.selection = vec![id.to_string()];
        }
        state.menu_target = Some(id.to_string());
    }));
    // The bin's cards share the menu's rows and token with the clips; which
    // one was asked for is what the two targets below remember.
    editor.on_media_context(on_window!(|state, row: i32| {
        state.menu_token += 1;
        state.menu_target = None;
        state.menu_media = state
            .media
            .by_row(state.project(), row)
            .map(|item| item.id.clone());
    }));
    editor.on_menu_selected(on_window!(|state, action: SharedString| {
        if let Some(id) = state.menu_media.clone() {
            state.media_action(&id, action.as_str());
            return;
        }
        // The clip the menu was opened on; failing that, the one clip that
        // is selected, which is what the menu was showing anyway.
        let target = state.menu_target.clone().or_else(|| state.sole_selection());
        if let Some(id) = target {
            state.clip_action(&id, action.as_str());
        }
    }));

    // ── the keyboard ──
    //
    // A press on any pane's floor takes focus back from the field that had
    // it; see Editor.blur. The chords that are also menu rows go through the
    // menu's handler, so the key and the row cannot come apart.
    editor.on_blur(|| Shell::with(|_, app| app.invoke_blur()));
    // A field that is done being typed into - Enter, Escape - releases the
    // focus the same way, rather than clearing it: a cleared focus is a
    // window where no key reaches anything. See Focus in util.slint.
    app.global::<Focus>()
        .on_release(|| Shell::with(|_, app| app.invoke_blur()));
    editor.on_shortcut(move |action: SharedString| match action.as_str() {
        "import" | "export" | "settings" | "zoom-in" | "zoom-out" | "start" | "end" | "snap"
        | "magnetic" | "pan" | "preview-axis" => {
            Shell::with(|_, app| app.invoke_app_menu_selected(action.clone()));
        }
        _ => Shell::with(|shell, app| {
            shell.studio.borrow_mut().shortcut(action.as_str());
            shell.studio.borrow_mut().refresh_art();
            shell.studio.borrow().publish(&app, &shell.models);
        }),
    });

    // ── the project sheet ──
    editor.on_modify_project(on_window!(|state| {
        state.handle(Msg::Project(ProjectMsg::Open));
    }));
    app.on_project_closed(on_window!(|state| {
        state.handle(Msg::Project(ProjectMsg::Close));
    }));
    app.on_project_name_edited(on_window!(|state, name: SharedString| {
        state.handle(Msg::Project(ProjectMsg::NameEdited(name.to_string())));
    }));
    app.on_project_size_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Project(ProjectMsg::SizeChanged(index)));
    }));
    app.on_project_rate_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Project(ProjectMsg::RateChanged(index)));
    }));
    app.on_project_apply(on_window!(|state| {
        state.handle(Msg::Project(ProjectMsg::Apply));
    }));

    app.on_relink_all(on_window!(|state| {
        state.handle(Msg::Relink(RelinkMsg::RelinkAll));
    }));
    app.on_relink_dismiss(on_window!(|state| {
        state.handle(Msg::Relink(RelinkMsg::Dismiss));
    }));

    // ── the dialogs ──
    app.on_export_clicked(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Open));
    }));
    app.on_open_settings(on_window!(|state| {
        state.handle(Msg::Settings(SettingsMsg::Open));
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
    // The accent is the same shape of thing: one int on the Theme global,
    // remembered under the name the global lists it by.
    app.on_settings_accent_changed({
        move |index| {
            Shell::with(|shell, app| {
                let theme = app.global::<Theme>();
                let names = theme.get_accent_names();
                let Some(name) = usize::try_from(index)
                    .ok()
                    .and_then(|row| slint::Model::row_data(&names, row))
                else {
                    return;
                };
                theme.set_accent_choice(index);
                let mut studio = shell.studio.borrow_mut();
                studio.prefs.accent = Some(prefs::Preferences::accent_id(&name));
                studio.prefs.save(&studio.host.dirs);
            });
        }
    });
    // A picked colour is remembered as its hex, which is what tells it from
    // a name when the file is read back. Every drag on the picker's square
    // lands here, so the file is written often for a moment; it is small.
    app.on_settings_accent_custom_changed({
        move |colour| {
            Shell::with(|shell, app| {
                let theme = app.global::<Theme>();
                let count = slint::Model::row_count(&theme.get_accent_names());
                theme.set_accent_custom(colour);
                theme.set_accent_choice(count as i32);
                let mut studio = shell.studio.borrow_mut();
                studio.prefs.accent = Some(format::hex_of(colour));
                studio.prefs.save(&studio.host.dirs);
            });
        }
    });
    app.on_export_closed(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Close));
    }));
    app.on_settings_closed(on_window!(|state| {
        state.handle(Msg::Settings(SettingsMsg::Close));
    }));
    app.on_export_name_edited(on_window!(|state, name: SharedString| {
        state.handle(Msg::Export(ExportMsg::NameEdited(name.to_string())));
    }));
    app.on_export_resolution_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Export(ExportMsg::ResolutionChanged(index)));
    }));
    app.on_export_rate_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Export(ExportMsg::RateChanged(index)));
    }));
    app.on_export_quality_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Export(ExportMsg::QualityChanged(index)));
    }));
    app.on_export_codec_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Export(ExportMsg::CodecChanged(index)));
    }));
    app.on_export_ten_bit_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Export(ExportMsg::TenBitChanged(on)));
    }));
    app.on_export_color_range_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Export(ExportMsg::ColorRangeChanged(index)));
    }));
    app.on_export_advanced_toggled(on_window!(|state, on: bool| {
        state.handle(Msg::Export(ExportMsg::AdvancedToggled(on)));
    }));
    app.on_export_rate_mode_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Export(ExportMsg::RateModeChanged(index)));
    }));
    app.on_export_bitrate_changed(on_window!(|state, text: SharedString| {
        state.handle(Msg::Export(ExportMsg::BitrateChanged(text.to_string())));
    }));
    app.on_export_again(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Again));
    }));
    app.on_export_browse(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Browse));
    }));
    app.on_export_reveal(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Reveal));
    }));
    app.on_export_cancel(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Cancel));
    }));
    app.on_export_start(on_window!(|state| {
        state.handle(Msg::Export(ExportMsg::Start));
    }));

    // ── settings ──
    app.on_settings_page_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Settings(SettingsMsg::PageChanged(index)));
    }));
    app.on_settings_show_log(on_window!(|state| {
        state.handle(Msg::Settings(SettingsMsg::ShowLog));
    }));
    app.on_settings_language_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Settings(SettingsMsg::LanguageChanged(index)));
    }));
    app.on_settings_playhead_stops_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Settings(SettingsMsg::PlayheadStopsChanged(on)));
    }));
    app.on_settings_custom_context_actions_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Settings(SettingsMsg::CustomContextActionsChanged(on)));
    }));
    app.on_settings_magnetic_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Settings(SettingsMsg::MagneticChanged(on)));
    }));
    app.on_settings_hardware_decode_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Settings(SettingsMsg::HardwareDecodeChanged(on)));
    }));
    app.on_settings_speech_accelerated_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Settings(SettingsMsg::SpeechAcceleratedChanged(on)));
    }));
    app.on_settings_download_source_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Settings(SettingsMsg::DownloadSourceChanged(index)));
    }));
    app.on_settings_download_base_edited(on_window!(|state, text: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::DownloadBaseEdited(
            text.to_string(),
        )));
    }));
    app.on_settings_server_enabled_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Settings(SettingsMsg::ServerEnabledChanged(on)));
    }));
    app.on_settings_server_host_edited(on_window!(|state, text: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::ServerHostEdited(
            text.to_string(),
        )));
    }));
    app.on_settings_server_port_edited(on_window!(|state, port: i32| {
        state.handle(Msg::Settings(SettingsMsg::ServerPortEdited(port)));
    }));
    app.on_settings_server_token_edited(on_window!(|state, text: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::ServerTokenEdited(
            text.to_string(),
        )));
    }));
    app.on_settings_server_token_generated(on_window!(|state| {
        state.handle(Msg::Settings(SettingsMsg::ServerTokenGenerated));
    }));
    app.on_model_activated(on_window!(|state, id: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::ModelActivated(id.to_string())));
    }));
    app.on_model_download(on_window!(|state, id: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::ModelDownload(id.to_string())));
    }));
    app.on_model_cancel(on_window!(|state, id: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::ModelCancel(id.to_string())));
    }));
    app.on_model_remove(on_window!(|state, id: SharedString| {
        state.handle(Msg::Settings(SettingsMsg::ModelRemove(id.to_string())));
    }));

    // ── the tray's sound and word tools ──
    editor.on_captions(on_window!(|state| {
        state.handle(Msg::Captions(CaptionsMsg::Open));
    }));
    editor.on_speak(on_window!(|state| {
        state.handle(Msg::Speech(SpeechMsg::Open));
    }));

    app.on_captions_closed(on_window!(|state| {
        state.handle(Msg::Captions(CaptionsMsg::Close));
    }));
    app.on_captions_text_edited(on_window!(|state, text: SharedString| {
        state.handle(Msg::Captions(CaptionsMsg::TextEdited(text.to_string())));
    }));
    app.on_captions_model_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Captions(CaptionsMsg::ModelChanged(index)));
    }));
    app.on_captions_placement_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Captions(CaptionsMsg::PlacementChanged(index)));
    }));
    app.on_captions_size_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Captions(CaptionsMsg::SizeChanged(index)));
    }));
    app.on_captions_begin(on_window!(|state| {
        state.handle(Msg::Captions(CaptionsMsg::Begin));
    }));
    app.on_captions_cancel(on_window!(|state| {
        state.handle(Msg::Captions(CaptionsMsg::Cancel));
    }));
    app.on_speech_closed(on_window!(|state| {
        state.handle(Msg::Speech(SpeechMsg::Close));
    }));
    app.on_speech_text_edited(on_window!(|state, text: SharedString| {
        state.handle(Msg::Speech(SpeechMsg::TextEdited(text.to_string())));
    }));
    app.on_speech_voice_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Speech(SpeechMsg::VoiceChanged(index)));
    }));
    app.on_speech_model_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Speech(SpeechMsg::ModelChanged(index)));
    }));
    app.on_speech_speed_changed(on_window!(|state, value: f32| {
        state.handle(Msg::Speech(SpeechMsg::SpeedChanged(value)));
    }));
    app.on_speech_quality_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Speech(SpeechMsg::QualityChanged(index)));
    }));
    app.on_speech_pauses_changed(on_window!(|state, value: f32| {
        state.handle(Msg::Speech(SpeechMsg::PausesChanged(value)));
    }));
    app.on_speech_reference_start_changed(on_window!(|state, seconds: f32| {
        state.handle(Msg::Speech(SpeechMsg::ReferenceStartChanged(seconds)));
    }));
    app.on_speech_use_sample_changed(on_window!(|state, on: bool| {
        state.handle(Msg::Speech(SpeechMsg::UseSampleChanged(on)));
    }));
    app.on_speech_sample_changed(on_window!(|state, index: i32| {
        state.handle(Msg::Speech(SpeechMsg::SampleChanged(index)));
    }));
    // A string cut, and nothing of the window's: the sheet keeps the text,
    // Rust only knows where a character starts.
    app.on_speech_insert_tag(|text: SharedString, tag: SharedString, at: i32| {
        panes::speech::insert_tag(text.as_str(), tag.as_str(), at.max(0) as usize).into()
    });
    app.on_speech_begin(on_window!(|state| {
        state.handle(Msg::Speech(SpeechMsg::Begin));
    }));
    app.on_speech_cancel(on_window!(|state| {
        state.handle(Msg::Speech(SpeechMsg::Cancel));
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
                        "add-selected" => state.handle(Msg::Media(MediaMsg::AddSelectedAtPlayhead)),
                        // The launch screen's own verb, from the menu bar:
                        // the picker, the check and the notice are all one
                        // implementation in panes/start.rs, so the two
                        // cannot drift into disagreeing about what a project
                        // folder is.
                        "open" => state.handle(Msg::Start(StartMsg::Open)),
                        "import" => {
                            platform::pick_files_async(&i18n::t("Import media"), None, |paths| {
                                on_ui(move |studio, _, _| {
                                    studio.handle(Msg::Media(MediaMsg::Import(paths)))
                                })
                            });
                        }
                        "export" => state.handle(Msg::Export(ExportMsg::Open)),
                        "template" => state.save_template(),
                        "speech" => state.handle(Msg::Speech(SpeechMsg::Open)),
                        "clear-cache" => state.clear_project_cache(),
                        "settings" => state.handle(Msg::Settings(SettingsMsg::Open)),
                        "close-project" => state.close_project(),
                        "undo" => state.undo(),
                        "redo" => state.redo(),
                        "snap" => state.handle(Msg::Timeline(TimelineMsg::SnapToggled)),
                        "pan" => {
                            let on = !state.lanes.pan_mode;
                            state.handle(Msg::Timeline(TimelineMsg::PanChanged(on)))
                        }
                        "preview-axis" => {
                            let on = !state.prefs.preview_axis;
                            state.handle(Msg::Timeline(TimelineMsg::PreviewAxisChanged(on)))
                        }
                        "magnetic" => state.handle(Msg::Timeline(TimelineMsg::MagneticToggled)),
                        "sort-added" => state.handle(Msg::Media(MediaMsg::SortChanged(0))),
                        "sort-name" => state.handle(Msg::Media(MediaMsg::SortChanged(1))),
                        "sort-kind" => state.handle(Msg::Media(MediaMsg::SortChanged(2))),
                        "zoom-in" => state.handle(Msg::Timeline(TimelineMsg::ZoomIn)),
                        "zoom-out" => state.handle(Msg::Timeline(TimelineMsg::ZoomOut)),
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
                        "ripple-delete" => state.ripple_delete_selected(),
                        "split" => {
                            let at = state.playhead;
                            state.split_at(at, false);
                        }
                        "save" => state.save(true),
                        _ => {}
                    }
                }
                if action == "close-window" {
                    log::info!("close: File > Close Window");
                    shell.studio.borrow_mut().close_project();
                    app.window().hide().ok();
                    slint::quit_event_loop().ok();
                    log::info!("close: quit_event_loop called (menu)");
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
    app.global::<Fmt>().on_hex_of(|value, with_alpha| {
        if with_alpha {
            format::hex_rgba(value).into()
        } else {
            format::hex_of(value).into()
        }
    });
    app.global::<Fmt>()
        .on_color_of(|text, fallback| format::parse_colour(text.as_str()).unwrap_or(fallback));
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
    app.global::<Payload>().on_tab_index(|text| {
        text.strip_prefix("tab:")
            .and_then(|rest| rest.split(':').next())
            .and_then(|index| index.parse().ok())
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
                // A timeline tab in flight wears the timeline pane's mark.
                if let Some(rest) = payload.strip_prefix("tab:") {
                    let name = rest.split_once(':').map_or("", |(_, name)| name);
                    let chip = slint::Image::load_from_svg_data(
                        chips::drag_chip_svg(
                            chips::pane_glyph("timeline"),
                            name,
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
                    .map(|peaks| format::wave_path(peaks, 0.0, plan.duration, 32, 0.75))
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

    let result = app.run();
    log::info!("close: event loop exited (ok={})", result.is_ok());
    std::process::exit(0);
}
