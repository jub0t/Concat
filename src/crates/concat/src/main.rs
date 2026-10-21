// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The editor window as a program: what a desktop or an iPhone launches.
//! Everything is in the library; see `lib.rs`.

// Hide the console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod launch;

use launch::Launch;

fn main() -> Result<(), slint::PlatformError> {
    // First of all, so that even `--version` says where it crashed if it
    // does: v0.2.7-rc.4's Windows on ARM build died there with nothing
    // but an exit code (#326). It writes to standard error until the log
    // opens, and to both after.
    concat_host::crash::catch();
    let launch = launch::parse(std::env::args_os().skip(1));
    // Before the log, the GPU and the window: all this proves is that the
    // executable and the libraries it links were loaded, and it should
    // prove nothing else. A write that fails - a closed pipe, a GUI build
    // with no standard output - is not a reason to panic.
    if launch == Ok(Launch::Version) {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "concat {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    // Before the log and the window too: what this proves is that whisper's
    // CPU kernels load on this CPU, and nothing else should be in the way.
    if launch == Ok(Launch::WhisperCpu) {
        use std::io::Write;
        let cpu = concat_speech::transcribe::cpu_backend().map_err(slint::PlatformError::Other)?;
        let _ = writeln!(std::io::stdout(), "whisper runs on {cpu}");
        return Ok(());
    }
    // An agent's MCP server: no window, no GPU, and standard output kept
    // for the protocol alone. The engine's log goes to standard error,
    // which the agent's client shows; not to a file, where every agent
    // session would push one of the window's own logs out of the folder.
    if let Ok(Launch::Mcp(mode)) = launch {
        let mode = mode
            .or_else(|| {
                std::env::var("CONCAT_MCP_MODE")
                    .ok()
                    .and_then(|word| concat_mcp::Mode::parse(&word))
            })
            .unwrap_or_default();
        let _ = concat_host::logs::open_console(None);
        return concat_mcp::run(mode).map_err(|error| {
            log::error!("mcp: {error}");
            slint::PlatformError::Other(error)
        });
    }
    // A desktop's console is its standard error, which a packaged build
    // has wired to nowhere; hence the file underneath it.
    concat::open_logging(None);
    // A failure here is the one this binary would otherwise swallow: with
    // no console, the runtime's "Error: ..." on standard error is lost.
    // https://github.com/jub0t/Concat/issues/135
    let launch = launch
        .map_err(slint::PlatformError::Other)
        .inspect_err(concat::report_startup_failure)?;
    match launch {
        Launch::Normal(project) => {
            concat::run_with_project(project).inspect_err(concat::report_startup_failure)
        }
        Launch::Software => {
            concat::draw_in_software();
            concat::run_with_project(None).inspect_err(concat::report_startup_failure)
        }
        // Logged and returned, never put up in a dialog: a smoke run is a
        // machine's, and a modal box with nobody to press OK would hang it
        // rather than fail it. Returning the error exits non-zero.
        Launch::Smoke => concat::smoke().inspect_err(|error| {
            log::error!("smoke: could not start: {error}");
        }),
        // Answered above, before the log was opened.
        Launch::Version | Launch::WhisperCpu | Launch::Mcp(_) => Ok(()),
    }
}
