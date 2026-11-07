// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The MCP server on its own, for development and the end-to-end test.
//! What ships is the same server inside the app: `concat --mcp`.
//!
//! usage: concat-mcp [auto|window|headless]   (or CONCAT_MCP_MODE)
//!          CONCAT_MCP_ADDRESS=127.0.0.1:7420  (default 127.0.0.1:7420)

use std::process::ExitCode;

fn main() -> ExitCode {
    let word = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CONCAT_MCP_MODE").ok())
        .unwrap_or_default();
    // The mode only picks a backend for the Android HTTP transport; on
    // every other target the parse still rejects a bad argument before the
    // message below, but the value goes unused.
    #[cfg_attr(
        not(all(target_os = "android", feature = "http")),
        allow(unused_variables)
    )]
    let Some(mode) = concat_mcp::Mode::parse(&word) else {
        eprintln!("usage: concat-mcp [auto|window|headless]");
        return ExitCode::from(2);
    };

    #[cfg(all(target_os = "android", feature = "http"))]
    {
        let address: std::net::SocketAddr = std::env::var("CONCAT_MCP_ADDRESS")
            .ok()
            .and_then(|text| text.parse().ok())
            .unwrap_or_else(|| "127.0.0.1:7420".parse().expect("default address parses"));
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => match rt.block_on(concat_mcp::run_http(address, mode)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("concat mcp: {error}");
                    ExitCode::FAILURE
                }
            },
            Err(error) => {
                eprintln!("concat mcp: could not start runtime: {error}");
                ExitCode::FAILURE
            }
        }
    }

    #[cfg(not(all(target_os = "android", feature = "http")))]
    {
        eprintln!(
            "concat mcp: HTTP/SSE transport is only available on Android with the `http` feature"
        );
        ExitCode::FAILURE
    }
}
