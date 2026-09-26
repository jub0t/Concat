// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the window asks of the platform it runs on.
//!
//! Four things differ between a desk and a phone: how the backend is
//! chosen, how a file or folder is picked, whether the window has a title
//! strip of its own to drag, and whether a file dragged in from outside the
//! window is even a thing that can happen. Everything else in the crate is
//! the same tree, the same state and the same callbacks, so the differences
//! live here and nowhere else.
//!
//! Desktop and iOS draw through the winit backend; Android through Slint's
//! android-activity backend, which the activity sets up before [`crate::run`]
//! is called. File dialogs are the desktop's: on a phone a pick goes through
//! the system's document picker, which arrives with the phone layout. A drag
//! in from the OS is a desktop thing for the same reason: winit only reports
//! `DroppedFile` on macOS, Windows and X11 - not Wayland, which has no such
//! event as of this winit, and not iOS, which has no such gesture.

use std::path::PathBuf;

use slint::PlatformError;
// The winit backend, and so these, exist everywhere but Android, which
// draws through Slint's android-activity backend and has no winit at all.
#[cfg(not(target_os = "android"))]
use slint::winit_030::winit::event::{ElementState, WindowEvent};
#[cfg(not(target_os = "android"))]
use slint::winit_030::winit::event_loop::ActiveEventLoop;
#[cfg(not(target_os = "android"))]
use slint::winit_030::winit::window::{Window as WinitWindow, WindowId};
#[cfg(not(target_os = "android"))]
use slint::winit_030::{CustomApplicationHandler, EventResult};

use crate::gpu::Gpu;
/// The window's raw events, seen before Slint routes them. Two jobs:
///
/// Dropped files. winit reports one path per event, in a burst, and the
/// burst is over when the loop is about to wait, so the paths are held
/// and handed over together: one probe, one notice, however many files.
///
/// A press away from the field being typed into. Slint keeps the focus
/// where it is until something else takes it, so a number field stayed in
/// its editing state - ring and all - through a click on a clip, a knob or
/// the monitor. Every press now passes through here first: if a text input
/// has the focus and the press is not on it, the window is asked to take
/// the focus back (App.blur) before Slint routes the press, and whatever
/// was pressed then takes the focus itself if it wants it - another input
/// does, so a click from one field into the next still lands a caret.
#[cfg(not(target_os = "android"))]
struct DropHandler {
    pending: Vec<PathBuf>,
    on_dropped: Box<dyn Fn(Vec<PathBuf>)>,
    /// Where the pointer last was, in logical pixels of the window: winit
    /// says where it moved, and a press says only that it happened.
    cursor: Option<(f32, f32)>,
    on_pressed_away: Box<dyn Fn()>,
}

#[cfg(not(target_os = "android"))]
impl DropHandler {
    fn new(
        on_dropped: impl Fn(Vec<PathBuf>) + 'static,
        on_pressed_away: impl Fn() + 'static,
    ) -> Self {
        Self {
            pending: Vec::new(),
            on_dropped: Box::new(on_dropped),
            cursor: None,
            on_pressed_away: Box::new(on_pressed_away),
        }
    }
}

/// Whether a press at (`x`, `y`), in logical pixels of `window`, lands
/// away from the text input that has the focus. False when nothing has
/// it, when what has it is not a text input - a button keeps its focus
/// for the keyboard's sake - and when the press is on the input itself,
/// give or take a few pixels of its padding, which is where a caret is
/// placed rather than where a field is left.
#[cfg(not(target_os = "android"))]
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

#[cfg(not(target_os = "android"))]
impl CustomApplicationHandler for DropHandler {
    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        winit_window: Option<&WinitWindow>,
        slint_window: Option<&slint::Window>,
        event: &WindowEvent,
    ) -> EventResult {
        match event {
            WindowEvent::DroppedFile(path) => self.pending.push(path.clone()),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = slint_window
                    .map(|window| f64::from(window.scale_factor()))
                    .or_else(|| winit_window.map(|window| window.scale_factor()))
                    .unwrap_or(1.0);
                self.cursor = Some(((position.x / scale) as f32, (position.y / scale) as f32));
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
        if !self.pending.is_empty() {
            (self.on_dropped)(std::mem::take(&mut self.pending));
        }
        EventResult::Propagate
    }
}

/// Whether the window draws the macOS traffic lights over its own strip.
pub const MACOS: bool = cfg!(target_os = "macos");

/// Whether the window is a phone's: one screen, held upright, driven by a
/// finger. The editor then wears the phone shell (ui/phone/) rather than
/// the strip and the dock, whatever the screen's size - a tablet is a big
/// phone here, not a small desk. True on Android and iOS; on a desk,
/// `CONCAT_PHONE=1` in the environment asks for the same shell in a window,
/// which is how the phone layout is worked on without a phone. That window
/// keeps the platform's own title bar - the phone shell has no strip to
/// drag by and no window buttons, and a phone has no such buttons to show
/// over its screen - so the desk's chrome sits above the phone's screen,
/// the way a simulator's does, and never on it.
pub fn phone() -> bool {
    cfg!(any(target_os = "android", target_os = "ios"))
        || std::env::var_os("CONCAT_PHONE").is_some_and(|value| !value.is_empty() && value != "0")
}

/// Chooses and installs the backend, and hands back the device the
/// renderer and the engine's compositor share, when there is one.
///
/// `on_files_dropped` fires on the event-loop thread with the paths of a
/// file (or several) dragged in from outside the window - Finder, Explorer,
/// a file manager - batched into one call per drag. It is taken here,
/// before the window exists, because the backend - and the hook into its
/// event loop that OS drops arrive through - has to be selected before
/// anything is built on top of it; see [`DropHandler`].
#[cfg(not(target_os = "android"))]
pub fn select_backend(
    on_files_dropped: impl Fn(Vec<PathBuf>) + 'static,
    on_pressed_away: impl Fn() + 'static,
) -> Result<Option<Gpu>, PlatformError> {
    // The device the renderer and the monitor share. Taken first, because
    // the backend is selected with it.
    let gpu = Gpu::acquire();
    match &gpu {
        None => log::warn!("no GPU adapter; the monitor composites on the CPU"),
        // A software adapter is refused by Slint's selector unless this
        // variable says otherwise: it enumerates the adapters on the instance
        // it was handed, finds none that is a GPU, and `select` fails with
        // "no GPU-backed WGPU adapter is available". From a double-clicked
        // GUI build that failure went to a console nobody could see, and the
        // app just never appeared. Setting it here is what the reporter did
        // by hand, and the window, the editing and the export all worked on
        // WARP - so the app does it for them and says so in the log.
        // https://github.com/jub0t/Concat/issues/135
        Some(gpu) if gpu.is_software() => {
            log::warn!("the GPU adapter is a software rasteriser; the window will render slowly");
            // SAFETY: the process is single-threaded at this point - the
            // backend, the engine's threads and the window all come after
            // this function - so nothing reads the environment concurrently.
            unsafe { std::env::set_var("SLINT_WGPU_CPU", "1") };
        }
        Some(_) => {}
    }

    let mut selector = slint::BackendSelector::new()
        .backend_name("winit".into())
        .with_winit_custom_application_handler(DropHandler::new(on_files_dropped, on_pressed_away));
    selector = match &gpu {
        Some(gpu) => selector.require_wgpu_30(gpu.configuration()),
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
    // when the caption goes, so the edges still resize, and the undecorated
    // shadow keeps the DWM drop shadow the caption would otherwise have
    // taken with it.
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
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "android")))]
    if desk {
        selector = selector
            .with_winit_window_attributes_hook(|attributes| attributes.with_decorations(false));
    }
    selector.select()?;
    Ok(gpu)
}

/// Tells the user the window could not start, in a dialog of the platform's
/// own, because the console the error would otherwise go to is not one a
/// double-clicked build has: on Windows a GUI subsystem binary has no
/// standard error, and the app was exiting with the reason written to
/// nowhere. The log file is named so the reader has something to attach.
/// https://github.com/jub0t/Concat/issues/135
pub fn report_startup_failure(error: &str) {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
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
    #[cfg(any(target_os = "android", target_os = "ios"))]
    let _ = error;
}

/// Whether the strip should draw its own window buttons: everywhere the
/// platform's decorations were taken off, which is everywhere but macOS,
/// where the traffic lights stay the window's.
pub const OWN_WINDOW_BUTTONS: bool = !MACOS;

/// Minimises the window: the strip's first button.
pub fn minimize(window: &slint::Window) {
    #[cfg(not(target_os = "android"))]
    {
        use slint::winit_030::WinitWindowAccessor;
        window.with_winit_window(|window| {
            window.set_minimized(true);
        });
    }
    #[cfg(target_os = "android")]
    let _ = window;
}

/// Whether the window is currently maximised, for the strip to pick the
/// maximise or the restore glyph. False where there is no such state.
pub fn is_maximized(window: &slint::Window) -> bool {
    #[cfg(not(target_os = "android"))]
    {
        use slint::winit_030::WinitWindowAccessor;
        window
            .with_winit_window(|window| window.is_maximized())
            .unwrap_or(false)
    }
    #[cfg(target_os = "android")]
    {
        let _ = window;
        false
    }
}

/// On Android the activity installed the backend before calling in, and
/// that backend draws on a device of its own; the monitor composites on the
/// CPU and hands the renderer finished pixels. Android has no OS drag to
/// wire up - a file arrives through the document picker instead - so
/// `on_files_dropped` is taken only to keep the signature the same as the
/// desktop's and is never called.
#[cfg(target_os = "android")]
pub fn select_backend(
    on_files_dropped: impl Fn(Vec<PathBuf>) + 'static,
    // Android has no pointer to press away with; the screen's own keyboard
    // dismisses, and the field goes with it.
    _on_pressed_away: impl Fn() + 'static,
) -> Result<Option<Gpu>, PlatformError> {
    let _ = on_files_dropped;
    Ok(None)
}

/// Starts a window drag from the title strip.
pub fn begin_drag(window: &slint::Window) {
    #[cfg(not(target_os = "android"))]
    {
        use slint::winit_030::WinitWindowAccessor;
        window.with_winit_window(|window| {
            let _ = window.drag_window();
        });
    }
    #[cfg(target_os = "android")]
    let _ = window;
}

/// Maximises the window, or restores it: the title strip's double-click.
pub fn toggle_maximize(window: &slint::Window) {
    #[cfg(not(target_os = "android"))]
    {
        use slint::winit_030::WinitWindowAccessor;
        window.with_winit_window(|window| {
            window.set_maximized(!window.is_maximized());
        });
    }
    #[cfg(target_os = "android")]
    let _ = window;
}

/// Asks for a folder, starting at `start` when there is one.
pub fn pick_folder(title: &str, start: &str) -> Option<PathBuf> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if !start.is_empty() {
            dialog = dialog.set_directory(start);
        }
        dialog.pick_folder()
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (title, start);
        None
    }
}

/// What a phone does when asked for files: shows the system's picker and
/// calls back, later, with what was chosen - an empty list for nothing.
/// Installed by the phone's own crate before the window runs; see
/// [`install_file_picker`].
#[cfg(any(target_os = "android", target_os = "ios"))]
pub type FilePicker = Box<dyn Fn(Box<dyn FnOnce(Vec<PathBuf>) + Send>) + Send + Sync>;

#[cfg(any(target_os = "android", target_os = "ios"))]
static FILE_PICKER: std::sync::OnceLock<FilePicker> = std::sync::OnceLock::new();

/// Installs the picker a phone answers [`pick_files_async`] with. Once;
/// a second call is ignored.
#[cfg(any(target_os = "android", target_os = "ios"))]
pub fn install_file_picker(picker: FilePicker) {
    let _ = FILE_PICKER.set(picker);
}

/// Asks for files and calls `on_picked` with them, on whichever thread the
/// platform answers from - the caller hops to the window's thread itself.
///
/// On a desktop the dialog blocks and the callback runs before this
/// returns. On a phone the system's picker is another screen: this returns
/// at once and the callback comes when the picker is dismissed, through
/// the picker the phone's crate installed. `filter` is the desktop
/// dialog's; a phone's picker offers every kind of media on its own.
pub fn pick_files_async(
    title: &str,
    filter: Option<(&str, &[&str])>,
    on_picked: impl FnOnce(Vec<PathBuf>) + Send + 'static,
) {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        if let Some(paths) = pick_files(title, filter) {
            on_picked(paths);
        }
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (title, filter);
        match FILE_PICKER.get() {
            Some(picker) => picker(Box::new(on_picked)),
            None => log::warn!("no file picker on this platform yet"),
        }
    }
}

/// Asks where to write a file, starting in `directory` with `name` filled
/// in, limited to the family `filter` names. None when the dialog was
/// dismissed - and on a phone, which has no such dialog: the caller writes
/// where it would have suggested, and says where.
pub fn save_file(
    title: &str,
    directory: &std::path::Path,
    name: &str,
    filter: (&str, &[&str]),
) -> Option<PathBuf> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let (family, extensions) = filter;
        rfd::FileDialog::new()
            .set_title(title)
            .set_directory(directory)
            .set_file_name(name)
            .add_filter(family, extensions)
            .save_file()
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (title, directory, name, filter);
        None
    }
}

/// Asks for files. `filter` names a family and its extensions, and limits
/// the dialog to them.
pub fn pick_files(title: &str, filter: Option<(&str, &[&str])>) -> Option<Vec<PathBuf>> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let mut dialog = rfd::FileDialog::new().set_title(title);
        if let Some((name, extensions)) = filter {
            dialog = dialog.add_filter(name, extensions);
        }
        dialog.pick_files()
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (title, filter);
        None
    }
}

/// Shows a written file in the platform's file manager.
///
/// A phone has no file manager to hand a path to, and says so rather than
/// appearing to work: a control that silently does nothing is worse than one
/// that explains itself. The path is in the message, which is the part a
/// developer on a cable can still use.
pub fn reveal(path: &str) -> Result<(), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        opener::reveal(path).map_err(|error| error.to_string())
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Err(format!(
            "this device has no file manager to open {path} with"
        ))
    }
}
