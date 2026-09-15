// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The editor window as a program: what a desktop or an iPhone launches.
//! Everything is in the library; see `lib.rs`.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> Result<(), slint::PlatformError> {
    // A desktop's console is its standard error, which a packaged build
    // has wired to nowhere; hence the file underneath it.
    concat::open_logging(None);
    concat::run()
}
