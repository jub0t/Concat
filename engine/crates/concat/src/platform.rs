// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What the window asks of the platform it runs on.
//!
//! Three things differ between a desk and a phone: how the backend is
//! chosen, how a file or folder is picked, and whether the window has a
//! title strip of its own to drag. Everything else in the crate is the same
//! tree, the same state and the same callbacks, so the differences live here
//! and nowhere else.
//!
//! Desktop and iOS draw through the winit backend; Android through Slint's
//! android-activity backend, which the activity sets up before [`crate::run`]
//! is called. File dialogs are the desktop's: on a phone a pick goes through
//! the system's document picker, which arrives with the phone layout.

use std::path::PathBuf;

use slint::PlatformError;

use crate::gpu::Gpu;

/// Whether the window draws the macOS traffic lights over its own strip.
pub const MACOS: bool = cfg!(target_os = "macos");

/// Chooses and installs the backend, and hands back the device the
/// renderer and the engine's compositor share, when there is one.
#[cfg(not(target_os = "android"))]
pub fn select_backend() -> Result<Option<Gpu>, PlatformError> {
    // The device the renderer and the monitor share. Taken first, because
    // the backend is selected with it.
    let gpu = Gpu::acquire();
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

    // The custom title bar. On macOS the native bar is hidden and the traffic
    // lights are overlaid on the strip the UI draws. Other platforms keep
    // their decorations.
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
    Ok(gpu)
}

/// On Android the activity installed the backend before calling in, and
/// that backend draws on a device of its own; the monitor composites on the
/// CPU and hands the renderer finished pixels.
#[cfg(target_os = "android")]
pub fn select_backend() -> Result<Option<Gpu>, PlatformError> {
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
pub fn reveal(path: &str) -> Result<(), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        opener::reveal(path).map_err(|error| error.to_string())
    }
    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = path;
        Ok(())
    }
}
