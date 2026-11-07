// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Headless MCP server entry point for Android.
//!
//! This binary runs `concat --mcp` without a window, on Android.
//! It can be invoked via `adb shell` or by another app using `run-as`.
//!
//! Environment variables:
//! - CONCAT_MCP_MODE: auto|window|headless (default: auto)
//! - CONCAT_API_ADDRESS: address of window's server (for window mode)
//! - CONCAT_API_TOKEN: token for window's server
//! - CONCAT_MCP_ROOTS: colon-separated paths for headless mode
//! - CONCAT_MCP_ADDRESS: bind address for the MCP HTTP server (default 127.0.0.1:7420)

use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up logging to logcat
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("concat-mcp"),
    );

    log::info!("Concat MCP server starting (Android headless)");

    // Set up directories for the host crate
    let app = slint::android::AndroidApp::from_env().ok_or("AndroidApp not available")?;
    setup_android_directories(&app);

    // Determine MCP mode
    let mode = env::var("CONCAT_MCP_MODE")
        .ok()
        .and_then(|word| concat_mcp::Mode::parse(&word))
        .unwrap_or_default();

    let address = env::var("CONCAT_MCP_ADDRESS")
        .ok()
        .and_then(|text| text.parse().ok())
        .unwrap_or_else(|| "127.0.0.1:7420".parse().expect("default address parses"));

    log::info!("MCP mode: {:?}", mode);
    log::info!("MCP address: {}", address);

    // Run MCP server over HTTP/SSE
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start tokio runtime: {error}"))?
        .block_on(concat_mcp::run_http(address, mode))
        .map_err(|e| {
            log::error!("MCP server error: {}", e);
            e.into()
        })
}

fn setup_android_directories(app: &slint::android::AndroidApp) {
    use std::env;

    let Some(internal) = app.internal_data_path() else {
        log::error!("internal_data_path not available");
        return;
    };

    // SAFETY: Called before any other thread starts
    unsafe {
        env::set_var("XDG_CONFIG_HOME", &internal);
        env::set_var("XDG_DATA_HOME", &internal);
        let home = app.external_data_path().unwrap_or(internal);
        env::set_var("HOME", home);
    }

    log::info!(
        "Android directories configured: config={}, data={}",
        internal.display(),
        app.external_data_path().unwrap_or(internal).display()
    );
}
