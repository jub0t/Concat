// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Headless API server entry point for Android.
//!
//! Runs the Concat JSON-RPC API server without a window.
//! Can be used when you want just the API server, not the full editor.
//!
//! Environment variables:
//! - CONCAT_API_ADDRESS=0.0.0.0:7420 (default)
//! - CONCAT_API_TOKEN=... (optional, auto-generated)
//! - CONCAT_API_ROOTS=: separated paths (optional, defaults to app dirs)

use std::env;
use std::net::SocketAddr;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set up logging to logcat
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("concat-api"),
    );

    log::info!("Concat API server starting (Android headless)");

    // Set up directories for the host crate
    let app = slint::android::AndroidApp::from_env().ok_or("AndroidApp not available")?;
    setup_android_directories(&app);

    // Parse configuration
    let address: SocketAddr = env::var("CONCAT_API_ADDRESS")
        .unwrap_or_else(|_| "0.0.0.0:7420".to_string())
        .parse()
        .expect("CONCAT_API_ADDRESS must be a valid socket address like 0.0.0.0:7420");

    let token = env::var("CONCAT_API_TOKEN")
        .ok()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| {
            concat_server::mint_token().unwrap_or_else(|_| {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                use std::time::{SystemTime, UNIX_EPOCH};
                let mut hasher = DefaultHasher::new();
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
                    .hash(&mut hasher);
                format!("{:x}", hasher.finish())
            })
        });

    let config_dir = app
        .internal_data_path()
        .map(|p| p.join("config"))
        .expect("internal_data_path not available");

    let _ = std::fs::create_dir_all(&config_dir);

    let config = concat_server::Config {
        json: Some(address),
        token: Some(token.clone()),
        discovery: Some(concat_server::discovery::path_in(&config_dir)),
        roots: Some(vec![
            app.internal_data_path()
                .unwrap_or_else(|| config_dir.clone()),
            app.external_data_path()
                .unwrap_or_else(|| config_dir.clone()),
        ]),
        ..Default::default()
    };

    let exporter = concat_host::Exporter::new();
    let open_projects = concat_api::OpenProjects::default();

    log::info!(
        "Starting Concat API server on {} (token: {})",
        address,
        token
    );

    // Run server (blocks forever)
    let _ = concat_server::Server::start(config, move |events| {
        let mut api = concat_api::Api::new(events).unwrap();
        api.share_exporter(exporter.clone());
        api.share_open_projects(open_projects.clone());
        Ok(api)
    });

    Ok(())
}

fn setup_android_directories(app: &slint::android::AndroidApp) {
    use std::env;

    let Some(internal) = app.internal_data_path() else {
        log::error!("internal_data_path not available");
        return;
    };

    unsafe {
        env::set_var("XDG_CONFIG_HOME", &internal);
        env::set_var("XDG_DATA_HOME", &internal);
        let home = app.external_data_path().unwrap_or(internal);
        env::set_var("HOME", home);
    }
}
