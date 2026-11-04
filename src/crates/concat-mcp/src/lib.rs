// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Concat for AI agents: an MCP server over HTTP with Server-Sent Events
//! (SSE) whose tools are the Concat API's methods, with schemas generated
//! from those request types. The stdio transport is removed; MCP speaks
//! HTTP/SSE only (Android).
//!
//! It is a thin adapter, and deliberately so. Every tool is a
//! [`concat_api::Request`] - a method of the API, or one of its edit
//! commands - and its argument schema is cut out of the schema `schemars`
//! generates from those types ([`schema`]), so an edit means here exactly
//! what it means in the window, over JSON-RPC and over gRPC, and a field
//! added to a command reaches agents on the next build with nothing
//! written for it here. What this crate owns is the protocol
//! ([`protocol`]), the choice of tools and what an agent is told about
//! them ([`tools`]), a compact reading of a project for an agent's context
//! ([`summary`]), and where requests go ([`backend`]): the window's own
//! server when the Automation page has it on, or an API in this process
//! when it does not.
//!
//! Transport: `POST /mcp` for agent-to-server messages, `GET /mcp` for
//! server-to-agent Server-Sent Events. Sessions are named by
//! `Mcp-Session-Id`.

#[cfg(all(target_os = "android", feature = "http"))]
pub mod http;

pub mod backend;
pub mod protocol;
pub mod schema;
pub mod summary;
pub mod tools;

pub use backend::{Backend, Headless, Mode, Window};

/// Serves MCP over HTTP/SSE on `addr` until the process ends, through the
/// backend `mode` picks. Logs to standard error; standard output carries
/// nothing but protocol bytes on the SSE stream. Android only.
#[cfg(all(target_os = "android", feature = "http"))]
pub async fn run_http(addr: std::net::SocketAddr, mode: Mode) -> Result<(), String> {
    http::run_http(addr, mode).await
}
