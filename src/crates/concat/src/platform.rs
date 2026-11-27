// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! What the window asks of the platform it runs on: how the backend is
//! chosen, how a file or folder is picked, whether the window has a title
//! strip of its own to drag, and how a file dragged in from outside the
//! window arrives. Everything else in the crate is the same tree, the same
//! state and the same callbacks, so the differences live here and nowhere
//! else.
//!
//! Every desktop draws through the winit backend, and its file dialogs are
//! the platform's own. winit reports a file dragged in as `DroppedFile` on
//! macOS, Windows and X11, and not on Wayland, where the window listens for
//! the drop itself; see [`crate::wayland_drop`].

use std::path::PathBuf;

use slint::PlatformError;
use slint::winit_030::winit::event::{ElementState, MouseButton, WindowEvent};
use slint::winit_030::winit::event_loop::ActiveEventLoop;
use slint::winit_030::winit::window::{Window as WinitWindow, WindowId};
use slint::winit_030::{CustomApplicationHandler, EventResult};

use crate::gpu::Gpu;

/// The interface scale the person chose, as a percent of the display's
/// own; 0 follows the display. See [`apply_ui_scale`].
static UI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Remembers the interface scale [`apply_ui_scale`] applies: a percent of
/// the display's own scale, or `None` for the display's.
pub fn set_ui_scale(percent: Option<u32>) {
    UI_SCALE.store(percent.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);
}

/// Scales `window` to the display's own scale times the chosen percent.
///
/// Not through SLINT_SCALE_FACTOR, as it was: Slint takes that as the
/// whole factor, not a multiplier, and ignores the display's scale while
/// it is set, so "100 %" made the interface smaller on a Windows laptop at
/// 150 % and halved it on a Retina Mac, and moving the window to another
/// monitor no longer rescaled it. The display's scale is winit's, which no
/// dispatch here changes; this runs when the window first shows, when the
/// choice changes, and after every move to a monitor of another scale. A
/// SLINT_SCALE_FACTOR set by hand still wins, and then nothing is done.
/// https://github.com/jub0t/Concat/issues/320
pub fn apply_ui_scale(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    if std::env::var_os("SLINT_SCALE_FACTOR").is_some() {
        return;
    }
    let Some(native) = window.with_winit_window(|winit| winit.scale_factor()) else {
        // Not up yet, as at startup: try again shortly, a few times over,
        // or the chosen scale waits for the next pick in Settings (#462).
        static TRIES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if TRIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 40 {
            reapply_ui_scale(std::time::Duration::from_millis(50));
        } else {
            log::warn!(
                "display: wanted the chosen interface scale applied, but the window never came up to take it; it waits for the next pick in Settings"
            );
        }
        return;
    };
    let percent = UI_SCALE.load(std::sync::atomic::Ordering::Relaxed);
    let chosen = if percent == 0 {
        1.0
    } else {
        crate::prefs::Preferences::scale_factor(percent)
    };
    let factor = (native * chosen) as f32;
    if (window.scale_factor() - factor).abs() < 1e-3 {
        return;
    }
    log::info!(
        "display: interface scale {factor:.2} · the display's {native:.2} times the chosen {chosen:.2}"
    );
    let physical = window.size();
    window.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged {
        scale_factor: factor,
    });
    window.dispatch_event(slint::platform::WindowEvent::Resized {
        size: physical.to_logical(factor),
    });
}

/// [`apply_ui_scale`] on the open window, once the event in hand is done:
/// Slint applies a display's own scale as it handles the event, and the
/// chosen percent goes on top of that afterwards.
fn reapply_ui_scale(delay: std::time::Duration) {
    slint::Timer::single_shot(delay, || {
        crate::host::Shell::with(|_, app| {
            apply_ui_scale(slint::ComponentHandle::window(&app));
        });
    });
}
/// The window's raw events, seen before Slint routes them. Two jobs:
///
/// Dropped files. winit reports one path per event, in a burst, and the
/// burst is over when the loop is about to wait, so the paths are held
/// and handed over together: one probe, one notice, however many files.
/// On Wayland winit reports nothing, and the window's own listener puts
/// the paths of a drop in `arrived` from its thread and wakes the loop,
/// which hands them over the same way.
///
/// A press away from the field being typed into. Slint keeps the focus
/// where it is until something else takes it, so a number field stayed in
/// its editing state - ring and all - through a click on a clip, a knob or
/// the monitor. Every press now passes through here first: if a text input
/// has the focus and the press is not on it, the window is asked to take
/// the focus back (App.blur) before Slint routes the press, and whatever
/// was pressed then takes the focus itself if it wants it - another input
/// does, so a click from one field into the next still lands a caret.
///
/// And on Linux and Windows, the window's edges. X11 and Wayland take the
/// resize borders away with the decorations, and so, in effect, does
/// Windows: winit keeps WS_THICKFRAME on an undecorated window but makes
/// its client area the whole window, so Windows finds client area at
/// every edge and corner and nothing a pixel outside, and the window could
/// be maximised but not resized (#878). A press within a few pixels of the
/// frame is handed to the window manager as a resize (winit's
/// `drag_resize_window`) before Slint sees it, and the pointer wears the
/// matching arrow while it is there.
struct DropHandler {
    pending: Vec<PathBuf>,
    on_dropped: Box<dyn Fn(Vec<PathBuf>)>,
    /// Paths a Wayland drop delivered, from the listener's thread.
    arrived: std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
    /// The Wayland listener, once the window is up and is a Wayland one;
    /// `wayland_tried` says the question has been asked, whatever the
    /// answer, so it is asked once.
    wayland: Option<crate::wayland_drop::Listener>,
    wayland_tried: bool,
    /// Whether the window has been held to its screen yet: once, at its
    /// first event.
    fitted: bool,
    /// Where the pointer last was, in logical pixels of the window: winit
    /// says where it moved, and a press says only that it happened.
    cursor: Option<(f32, f32)>,
    on_pressed_away: Box<dyn Fn()>,
    /// The window came back to the front: the moment a person returns from
    /// moving a file, and the one where a file that went missing can be
    /// noticed. See `Studio::check_missing_media`.
    on_focused: Box<dyn Fn()>,
    /// The event loop is ending, by any road: see `start` in lib.rs.
    on_exiting: Box<dyn Fn()>,
    /// Whether this window draws its own resize edges: Linux and Windows,
    /// undecorated.
    edges: bool,
    /// The edge the pointer is on, while it is on one.
    edge: Option<ResizeDirection>,
}

use slint::winit_030::winit::window::{CursorIcon, ResizeDirection};

/// How far in from the frame a press still resizes, in logical pixels.
/// Wide enough to find without aiming, narrow enough to leave the strip's
/// buttons and the panes' own edges theirs.
const EDGE: f64 = 6.0;

/// The edge of a `width` × `height` window that (`x`, `y`) is on, all in
/// physical pixels, with `band` the depth of an edge. Corners first, so the
/// diagonal wins where two edges meet.
fn edge_at(x: f64, y: f64, width: f64, height: f64, band: f64) -> Option<ResizeDirection> {
    let (west, east) = (x < band, x >= width - band);
    let (north, south) = (y < band, y >= height - band);
    Some(match (north, south, west, east) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (true, _, _, true) => ResizeDirection::NorthEast,
        (_, true, true, _) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::North,
        (_, true, ..) => ResizeDirection::South,
        (_, _, true, _) => ResizeDirection::West,
        (_, _, _, true) => ResizeDirection::East,
        _ => return None,
    })
}

fn edge_cursor(edge: ResizeDirection) -> CursorIcon {
    match edge {
        ResizeDirection::North => CursorIcon::NResize,
        ResizeDirection::South => CursorIcon::SResize,
        ResizeDirection::West => CursorIcon::WResize,
        ResizeDirection::East => CursorIcon::EResize,
        ResizeDirection::NorthWest => CursorIcon::NwResize,
        ResizeDirection::NorthEast => CursorIcon::NeResize,
        ResizeDirection::SouthWest => CursorIcon::SwResize,
        ResizeDirection::SouthEast => CursorIcon::SeResize,
    }
}

impl DropHandler {
    fn new(
        on_dropped: impl Fn(Vec<PathBuf>) + 'static,
        on_pressed_away: impl Fn() + 'static,
        on_focused: impl Fn() + 'static,
        on_exiting: impl Fn() + 'static,
    ) -> Self {
        Self {
            pending: Vec::new(),
            on_dropped: Box::new(on_dropped),
            arrived: Default::default(),
            wayland: None,
            wayland_tried: false,
            fitted: false,
            cursor: None,
            on_pressed_away: Box::new(on_pressed_away),
            on_focused: Box::new(on_focused),
            on_exiting: Box::new(on_exiting),
            edges: cfg!(not(target_os = "macos")) && !phone(),
            edge: None,
        }
    }
}

/// Whether a press at (`x`, `y`), in logical pixels of `window`, lands
/// away from the text input that has the focus. False when nothing has
/// it, when what has it is not a text input - a button keeps its focus
/// for the keyboard's sake - and when the press is on the input itself,
/// give or take a few pixels of its padding, which is where a caret is
/// placed rather than where a field is left.
fn press_misses_focused_input(window: &slint::Window, x: f32, y: f32) -> bool {
    use slint::private_unstable_api::re_exports::{LogicalPoint, TextInput, WindowInner};

    let focused = WindowInner::from_pub(window).focus_item.borrow().upgrade();
    let Some(item) = focused else {
        return false;
    };
    if item.downcast::<TextInput>().is_none() {
        return false;
    }
    const SLACK: f32 = 8.0;
    let origin = item.map_to_window(LogicalPoint::default());
    let size = item.geometry().size;
    let inside = x >= origin.x - SLACK
        && x <= origin.x + size.width + SLACK
        && y >= origin.y - SLACK
        && y <= origin.y + size.height + SLACK;
    !inside
}

impl CustomApplicationHandler for DropHandler {
    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        winit_window: Option<&WinitWindow>,
        slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        if !self.fitted
            && let Some(window) = winit_window
        {
            self.fitted = true;
            fit_to_screen(window);
        }
        if !self.wayland_tried
            && let Some(window) = winit_window
        {
            self.wayland_tried = true;
            self.wayland = listen_for_wayland_drops(window, &self.arrived);
        }
        // Safe start's two signs that the launch got its window: Slint
        // draws the frame as it handles this redraw, and a person presses
        // only what they can see. Before the match, whose arms may return.
        match event {
            WindowEvent::RedrawRequested => crate::window_drew(),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(..)
            | WindowEvent::KeyboardInput {
                is_synthetic: false,
                ..
            } => crate::window_touched(),
            _ => {}
        }
        match event {
            WindowEvent::ScaleFactorChanged { .. } => reapply_ui_scale(std::time::Duration::ZERO),
            WindowEvent::DroppedFile(path) => self.pending.push(path.clone()),
            // Back at the front, after Explorer or the file manager: what a
            // dropped file is to an import, a return is to a look for one
            // that went away while the window was open.
            WindowEvent::Focused(true) => (self.on_focused)(),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = slint_window
                    .map(|window| f64::from(window.scale_factor()))
                    .or_else(|| winit_window.map(|window| window.scale_factor()))
                    .unwrap_or(1.0);
                self.cursor = Some(((position.x / scale) as f32, (position.y / scale) as f32));
                if let Some(window) = winit_window.filter(|_| self.edges) {
                    let size = window.inner_size();
                    let edge = (window.is_resizable()
                        && !window.is_maximized()
                        && window.fullscreen().is_none())
                    .then(|| {
                        edge_at(
                            position.x,
                            position.y,
                            f64::from(size.width),
                            f64::from(size.height),
                            EDGE * scale,
                        )
                    })
                    .flatten();
                    if edge != self.edge {
                        // Off an edge the arrow goes back to the default and
                        // Slint puts its own on at the next change of hover.
                        window.set_cursor(edge.map_or(CursorIcon::Default, edge_cursor));
                        self.edge = edge;
                    }
                    if edge.is_some() {
                        return EventResult::PreventDefault;
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => self.edge = None,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.edge.is_some() => {
                if let (Some(window), Some(edge)) = (winit_window, self.edge) {
                    let _ = window.drag_resize_window(edge);
                }
                return EventResult::PreventDefault;
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } => {
                if let (Some(window), Some((x, y))) = (slint_window, self.cursor)
                    && press_misses_focused_input(window, x, y)
                {
                    (self.on_pressed_away)();
                }
            }
            _ => {}
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) -> EventResult {
        if let Ok(mut arrived) = self.arrived.lock() {
            self.pending.append(&mut arrived);
        }
        if !self.pending.is_empty() {
            (self.on_dropped)(std::mem::take(&mut self.pending));
        }
        EventResult::Propagate
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) -> EventResult {
        // Before winit closes its display, which the listener reads.
        self.wayland = None;
        (self.on_exiting)();
        EventResult::Propagate
    }
}

/// The Wayland drop listener for `window`, when the window is a Wayland
/// one: it puts each drop's paths in `arrived` and wakes the event loop,
/// whose next `about_to_wait` hands them over. None on any other display.
fn listen_for_wayland_drops(
    window: &WinitWindow,
    arrived: &std::sync::Arc<std::sync::Mutex<Vec<PathBuf>>>,
) -> Option<crate::wayland_drop::Listener> {
    use slint::winit_030::winit::raw_window_handle::{
        HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
    };
    let display = window.display_handle().ok()?.as_raw();
    let surface = window.window_handle().ok()?.as_raw();
    let (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(surface)) =
        (display, surface)
    else {
        return None;
    };
    let arrived = std::sync::Arc::clone(arrived);
    let sink = move |paths: Vec<PathBuf>| {
        if let Ok(mut held) = arrived.lock() {
            held.extend(paths);
        }
        // An empty errand, run on the loop's thread: what wakes it.
        let _ = slint::invoke_from_event_loop(|| {});
    };
    // SAFETY: winit keeps the display for the life of the event loop, and
    // the listener is dropped in `exiting`, before the loop is.
    unsafe { crate::wayland_drop::Listener::start(display.display, surface.surface, sink) }
}

/// Whether the window draws the macOS traffic lights over its own strip.
pub const MACOS: bool = cfg!(target_os = "macos");

/// Set when the start found only a software adapter (Microsoft Basic
/// Render Driver, llvmpipe) though nothing asked for one: the machine has
/// no graphics driver Concat can draw with. A safe start's or the
/// person's own CONCAT_GPU=software is not this; those have notices of
/// their own.
static NO_DRIVER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether this machine has no graphics driver Concat can use, so
/// everything draws on the processor (see [`select_backend`]): the window
/// says so once at the start, with what fixes it, rather than leaving a
/// slow editor unexplained.
pub fn no_graphics_driver() -> bool {
    NO_DRIVER.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether the window wears the phone shell (ui/phone/) rather than the
/// strip and the dock: one screen, held upright, driven by a finger.
/// `CONCAT_PHONE=1` in the environment asks for it, which is how the phone
/// layout is worked on without a phone. That window keeps the platform's
/// own title bar - the phone shell has no strip to drag by and no window
/// buttons - so the desk's chrome sits above the phone's screen, the way a
/// simulator's does, and never on it.
pub fn phone() -> bool {
    std::env::var_os("CONCAT_PHONE").is_some_and(|value| !value.is_empty() && value != "0")
}

/// Chooses and installs the backend, and hands back the device the
/// renderer and the engine's compositor share, when there is one.
///
/// `on_files_dropped` fires on the event-loop thread with the paths of a
/// file (or several) dragged in from outside the window - Finder, Explorer,
/// a file manager - batched into one call per drag. It is taken here,
/// before the window exists, because the backend - and the hook into its
/// event loop that OS drops arrive through - has to be selected before
/// anything is built on top of it; see [`DropHandler`]. `on_focused` fires
/// the same way each time the window is brought back to the front.
pub fn select_backend(
    on_files_dropped: impl Fn(Vec<PathBuf>) + 'static,
    on_pressed_away: impl Fn() + 'static,
    on_focused: impl Fn() + 'static,
    on_exiting: impl Fn() + 'static,
) -> Result<Option<Gpu>, PlatformError> {
    // Software asked for before anything was found - a safe start, or the
    // person - rather than the only adapter there is.
    let chosen = std::env::var("CONCAT_GPU")
        .is_ok_and(|value| value.trim().eq_ignore_ascii_case("software"));
    // The device the renderer and the monitor share. Taken first, because
    // the backend is selected with it.
    let mut gpu = Gpu::acquire();
    // A software adapter is not shared. Skia drew the window through
    // Direct3D 12 on WARP, and the monitor composited on the same device:
    // when WARP removed it (0x887A0005, as a clip went onto the timeline),
    // wgpu panicked in its own queue and took the window, the project and
    // the export with it (#480, #503). Skia's own CPU rasteriser draws the
    // window instead - with no device to lose, and faster than Skia's GPU
    // path run on a CPU - and the monitor composites on a device of its
    // own, on the software adapter too (CONCAT_GPU, which the compositor
    // reads), handing the window pixels as it does with no GPU at all.
    // A Mac asked to draw in software has no software adapter to find
    // (`Gpu::acquire` comes back empty), and Slint's own choice would be
    // Skia on Metal again - the device that would not open (#600). Skia's
    // own rasteriser draws the window instead, as on WARP.
    let asked = cfg!(target_vendor = "apple")
        && std::env::var("CONCAT_GPU")
            .is_ok_and(|value| value.trim().eq_ignore_ascii_case("software"));
    let software = asked || gpu.as_ref().is_some_and(Gpu::is_software);
    if asked {
        log::warn!(
            "gpu: the window draws on the CPU, as asked at start, and the monitor on a Metal device of its own"
        );
    } else if software {
        log::warn!(
            "gpu: wanted a GPU and the adapter is a software rasteriser; the window draws on the CPU and the monitor on a device of its own"
        );
        if !chosen {
            NO_DRIVER.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    if software {
        // SAFETY: the process is single-threaded at this point - the
        // backend, the engine's threads and the window all come after
        // this function - so nothing reads the environment concurrently.
        unsafe { std::env::set_var("CONCAT_GPU", "software") };
        gpu = None;
    } else if gpu.is_none() {
        log::warn!("gpu: wanted a GPU and found no adapter; the monitor composites on the CPU");
    }

    let mut selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_custom_application_handler(DropHandler::new(
            on_files_dropped,
            on_pressed_away,
            on_focused,
            on_exiting,
        ));
    selector = match &gpu {
        Some(gpu) => selector.require_wgpu_30(gpu.configuration()),
        None if software => selector.renderer_name("skia-software".into()),
        None => {
            // Without a shared device, ask for the platform's own API by
            // name: Skia picks its surface from a cfg chain, and requiring
            // one turns a silent fall back to the CPU rasteriser into a
            // refusal to start, which is a fault you can see.
            #[cfg(target_vendor = "apple")]
            {
                selector.require_metal()
            }
            #[cfg(target_family = "windows")]
            {
                selector.require_d3d()
            }
            #[cfg(not(any(target_vendor = "apple", target_family = "windows")))]
            {
                selector
            }
        }
    };

    // The custom title bar. The window draws its own strip, so the
    // platform's is not wanted - but each platform is asked in its own way.
    //
    // macOS keeps the real title bar and makes it invisible: transparent,
    // untitled, with the content view under it. That is what keeps the
    // traffic lights, which are the window's and not ours to draw, and the
    // strip leaves 80px for them (title-bar.slint).
    //
    // Everywhere else the decorations go entirely and the strip carries its
    // own minimise, maximise and close. On Windows winit keeps WS_SIZEBOX
    // when the caption goes, but no border of the window is left outside
    // its client area for it to act on, so the window resizes from its own
    // edges as on Linux (see `DropHandler`, #878); the undecorated shadow
    // keeps the DWM drop shadow the caption would otherwise have taken
    // with it.
    //
    // The attributes below are the window's first state; what keeps the
    // decorations off is the Slint window's `no-frame` (app.slint), which
    // the winit backend re-applies after the window is made. On Wayland
    // without it GNOME's bar came back above the strip:
    // https://github.com/jub0t/Concat/issues/97
    // https://github.com/jub0t/Concat/issues/145
    //
    // None of it for the phone shell shown on a desk: that has no strip of
    // its own, so the platform's title bar stays, buttons and all, above
    // the phone's screen rather than over it. See `phone`.
    let desk = !phone();
    #[cfg(target_os = "macos")]
    if desk {
        use slint::winit_030::winit::platform::macos::WindowAttributesExtMacOS;
        selector = selector.with_winit_window_attributes_hook(|attributes| {
            attributes
                .with_titlebar_transparent(true)
                .with_title_hidden(true)
                .with_fullsize_content_view(true)
        });
    }
    #[cfg(target_os = "windows")]
    if desk {
        use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
        selector = selector.with_winit_window_attributes_hook(|attributes| {
            attributes
                .with_decorations(false)
                .with_undecorated_shadow(true)
        });
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    if desk {
        selector = selector
            .with_winit_window_attributes_hook(|attributes| attributes.with_decorations(false));
    }
    selector.select()?;
    // The desktop entry's name (concat.desktop, in every Linux package and
    // the flake), as the window's Wayland app_id and X11 class, so a dock
    // or a task switcher files the running window under the launcher it
    // came from - its icon, its pin - rather than as an unknown app. Slint
    // ignores it off Linux and the BSDs.
    let _ = slint::set_xdg_app_id("concat");
    Ok(gpu)
}

/// Tells the user the window could not start, in a dialog of the platform's
/// own, because the console the error would otherwise go to is not one a
/// double-clicked build has: on Windows a GUI subsystem binary has no
/// standard error, and the app was exiting with the reason written to
/// nowhere. The log file is named so the reader has something to attach.
/// https://github.com/jub0t/Concat/issues/135
pub fn report_startup_failure(error: &str) {
    let mut description = format!("Concat could not start.\n\n{error}");
    if let Some(path) = concat_host::logs::current() {
        description.push_str(&format!("\n\nLog: {}", path.display()));
    }
    rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Concat")
        .set_description(description)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// Whether the strip should draw its own window buttons: everywhere the
/// platform's decorations were taken off, which is everywhere but macOS,
/// where the traffic lights stay the window's.
pub const OWN_WINDOW_BUTTONS: bool = !MACOS;

/// A shortcut as this platform writes it, from the parts a `@keys` takes:
/// `["Control", "Shift", "Z"]` reads ⇧⌘Z on a Mac and Ctrl+Shift+Z on
/// Windows and Linux, because Slint takes `Control` for ⌘ there. The same
/// parts as the binding in app.slint, so a menu row names the keys that
/// work on the machine it is shown on.
pub fn keys(parts: &[&str]) -> String {
    slint::Keys::from_parts(parts.iter().copied())
        .map(|keys| keys.to_string())
        .unwrap_or_default()
}

/// The delete key as this platform writes it: ⌫ on a Mac, Del elsewhere,
/// with Shift in front for the ripple delete. Either key deletes
/// (app.slint); these are the names each platform's own menus use.
pub fn delete_key(shift: bool) -> String {
    match (MACOS, shift) {
        (true, false) => "⌫".into(),
        (true, true) => "⇧⌫".into(),
        (false, false) => "Del".into(),
        (false, true) => "Shift+Del".into(),
    }
}

/// Minimises the window: the strip's first button.
pub fn minimize(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|window| {
        window.set_minimized(true);
    });
}

/// Maximises a window that opened larger than its screen. The window asks
/// for 1400 by 900 and may not shrink past 900 by 560, both in logical
/// pixels, and a 1920 by 1080 laptop at 150 % has 1280 by 720 of them less
/// its taskbar: the window opened past the screen's edges, with its corner
/// handles out of reach, and could not be moved back or made smaller
/// (#347). Maximised it fills the work area, taskbar and all, and the
/// title strip's double-click restores it.
/// macOS shrinks a window to its screen by itself, so there it is left be.
fn fit_to_screen(window: &WinitWindow) {
    if cfg!(target_os = "macos") {
        return;
    }
    let Some(monitor) = window.current_monitor() else {
        return;
    };
    let (screen, size) = (monitor.size(), window.outer_size());
    // A taskbar or a dock takes up to about a tenth of the height; winit
    // knows the monitor and not its work area.
    let room = f64::from(screen.height) * 0.9;
    if size.width > screen.width || f64::from(size.height) > room {
        log::info!(
            "display: the window's {}x{} does not fit the screen's {}x{} · maximised",
            size.width,
            size.height,
            screen.width,
            screen.height
        );
        window.set_maximized(true);
    }
}

/// Whether the window is currently maximised, for the strip to pick the
/// maximise or the restore glyph. False where there is no such state.
pub fn is_maximized(window: &slint::Window) -> bool {
    use slint::winit_030::WinitWindowAccessor;
    window
        .with_winit_window(|window| window.is_maximized())
        .unwrap_or(false)
}

/// Starts a window drag from the title strip.
pub fn begin_drag(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|window| {
        let _ = window.drag_window();
    });
}

/// Maximises the window, or restores it: the title strip's double-click.
pub fn toggle_maximize(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|window| {
        window.set_maximized(!window.is_maximized());
    });
}

/// Asks for a folder, starting at `start` when there is one.
pub fn pick_folder(title: &str, start: &str) -> Option<PathBuf> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if let Some(folder) = existing_folder(start) {
        dialog = dialog.set_directory(folder);
    }
    dialog.pick_folder()
}

/// Where a folder dialog asked to start at `start` can open: `start`
/// itself, or the nearest folder above it that exists. A new project's
/// default, `<Desktop>/Concat`, is not made until the first project is, and
/// GNOME's chooser (Nautilus, behind the portal) fails on a folder that is
/// not there, where GTK's quietly opens elsewhere.
/// https://github.com/jub0t/Concat/issues/331
fn existing_folder(start: &str) -> Option<&std::path::Path> {
    std::path::Path::new(start)
        .ancestors()
        .find(|folder| folder.is_dir())
}

/// Opens a web address in the system's browser: the launch screen's
/// guides, the repository, the release notes, through the platform's
/// opener. An address that goes nowhere says why.
pub fn open_link(url: &str) -> Result<(), String> {
    opener::open(url).map_err(|error| error.to_string())
}

/// Asks where to write a file, starting in `directory` with `name` filled
/// in, limited to the family `filter` names. None when the dialog was
/// dismissed.
pub fn save_file(
    title: &str,
    directory: &std::path::Path,
    name: &str,
    filter: (&str, &[&str]),
) -> Option<PathBuf> {
    let (family, extensions) = filter;
    rfd::FileDialog::new()
        .set_title(title)
        .set_directory(directory)
        .set_file_name(name)
        .add_filter(family, extensions)
        .save_file()
}

/// Asks for files. `filter` names a family and its extensions, and limits
/// the dialog to them.
pub fn pick_files(title: &str, filter: Option<(&str, &[&str])>) -> Option<Vec<PathBuf>> {
    let mut dialog = rfd::FileDialog::new().set_title(title);
    if let Some((name, extensions)) = filter {
        dialog = dialog.add_filter(name, extensions);
    }
    dialog.pick_files()
}

/// Shows a written file in the platform's file manager.
pub fn reveal(path: &str) -> Result<(), String> {
    opener::reveal(path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A menu row names the keys the way the platform it runs on does:
    /// the Mac's glyphs there, Ctrl and Shift spelled out everywhere else.
    #[test]
    fn shortcuts_read_the_way_the_platform_writes_them() {
        if cfg!(target_os = "macos") {
            assert_eq!(keys(&["Control", "Shift", "Z"]), "⇧⌘Z");
            assert_eq!(keys(&["Control", ","]), "⌘,");
            assert_eq!(delete_key(true), "⇧⌫");
        } else {
            assert_eq!(keys(&["Control", "Shift", "Z"]), "Ctrl+Shift+Z");
            assert_eq!(keys(&["Control", ","]), "Ctrl+,");
            assert_eq!(delete_key(true), "Shift+Del");
        }
    }

    /// The frame's edges resize, corners diagonally, and the middle of the
    /// window is nobody's edge.
    #[test]
    fn a_press_near_the_frame_finds_its_edge() {
        let at = |x, y| edge_at(x, y, 1000.0, 800.0, 6.0);
        assert_eq!(at(500.0, 400.0), None);
        assert_eq!(at(2.0, 400.0), Some(ResizeDirection::West));
        assert_eq!(at(997.0, 400.0), Some(ResizeDirection::East));
        assert_eq!(at(500.0, 0.0), Some(ResizeDirection::North));
        assert_eq!(at(500.0, 799.0), Some(ResizeDirection::South));
        assert_eq!(at(1.0, 1.0), Some(ResizeDirection::NorthWest));
        assert_eq!(at(999.0, 799.0), Some(ResizeDirection::SouthEast));
        assert_eq!(at(6.0, 400.0), None);
    }

    /// A folder dialog opens on a folder that exists: a default location
    /// not made yet opens on the folder it would be made in, one that
    /// exists opens as given, and no location asks for none.
    #[test]
    fn a_folder_dialog_starts_at_a_folder_that_exists() {
        let root = std::env::temp_dir().join(format!("concat-pick-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let missing = root.join("Concat");
        assert_eq!(
            existing_folder(&missing.to_string_lossy()),
            Some(root.as_path())
        );
        assert_eq!(
            existing_folder(&root.to_string_lossy()),
            Some(root.as_path())
        );
        assert_eq!(existing_folder(""), None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
