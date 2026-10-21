// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The MCP server on its own, for development and the end-to-end test.
//! What ships is the same server inside the app: `concat --mcp`.
//!
//! usage: concat-mcp [auto|window|headless]   (or CONCAT_MCP_MODE)

use std::process::ExitCode;

fn main() -> ExitCode {
    let word = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CONCAT_MCP_MODE").ok())
        .unwrap_or_default();
    let Some(mode) = concat_mcp::Mode::parse(&word) else {
        eprintln!("usage: concat-mcp [auto|window|headless]");
        return ExitCode::from(2);
    };
    match concat_mcp::run(mode) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("concat mcp: {error}");
            ExitCode::FAILURE
        }
    }
}
