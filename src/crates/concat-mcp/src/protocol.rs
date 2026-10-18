// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Model Context Protocol over a pair of streams: newline-delimited
//! JSON-RPC 2.0, one message a line, as the stdio transport has it.
//!
//! Both eras of the protocol are served, since clients of both are in
//! use. A *legacy* client (protocol 2025-11-25 and earlier) opens with
//! `initialize`, is answered with the version it asked for when this
//! server speaks it, and sends `notifications/initialized`. A *modern*
//! one (2026-07-28) sends no handshake: every request carries its
//! protocol version in `params._meta`, `server/discover` says what this
//! server is, and a version this server does not speak is refused with
//! `UnsupportedProtocolVersion` naming the ones it does. Nothing here is
//! kept per connection either way - the protocol version of a legacy
//! client is only remembered to answer `initialize` - so the two can mix.
//!
//! Requests run one at a time, in order, on the caller's thread; a reader
//! thread takes lines in meanwhile, so `ping` is answered and
//! `notifications/cancelled` is heard while a long call - `export_wait` -
//! is in progress. A cancelled call sends nothing more, as the protocol
//! requires.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use serde_json::{Map, Value, json};

use crate::backend::Backend;
use crate::tools::{Context, Toolbox};

/// The newest protocol, served without a handshake.
pub const MODERN: &str = "2026-07-28";
/// The handshake-era protocols served, newest first.
pub const LEGACY: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];
/// The `_meta` keys a modern request carries.
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";
/// The longest line read: a tool call's arguments are small.
const MAX_LINE: u64 = 16 * 1024 * 1024;
/// How many cancellations of requests not yet begun are remembered.
const EARLY_CANCELS: usize = 1024;

/// JSON-RPC's and MCP's error numbers.
mod code {
    pub const PARSE: i64 = -32700;
    pub const INVALID_REQUEST: i64 = -32600;
    pub const METHOD_NOT_FOUND: i64 = -32601;
    pub const INVALID_PARAMS: i64 = -32602;
    pub const INTERNAL: i64 = -32603;
    pub const UNSUPPORTED_VERSION: i64 = -32022;
}

/// What the agent is told about this server, once, when it connects.
const INSTRUCTIONS: &str = "Concat is a video editor; these tools edit its projects. A project is a folder: open one (project_open) or make one (project_create), then pass its folder as `path` to every other tool. Read timeline_read for the ids of lanes (tracks), clips and media. Times and durations are seconds on the timeline; positions within a clip (keyframes) are fractions from 0 to 1; sizes and offsets of titles and pictures are fractions of the frame. Every edit is one undoable step (edit_undo); edit_apply with a batch command makes several into one. Edits live in memory until project_save. To render, export_start then export_wait. preview_frame shows the frame at a time. A refused edit says why in the window's words and changes nothing.";

/// The server's name and version, as `serverInfo`.
fn server_info() -> Value {
    json!({
        "name": "concat",
        "title": "Concat",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// The capabilities this server has: tools, and nothing else.
fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

/// Every version this server speaks, newest first.
fn supported() -> Vec<&'static str> {
    std::iter::once(MODERN).chain(LEGACY).collect()
}

/// The one writer of the output stream: a line and a flush, under a lock,
/// so a progress notification from a tool never lands inside a reply.
#[derive(Clone)]
struct Output(Arc<Mutex<Box<dyn Write + Send>>>);

impl Output {
    fn send(&self, message: &Value) {
        if let Ok(mut out) = self.0.lock() {
            // A line, never more: serde_json escapes the newlines a string
            // holds, and the compact form has none of its own.
            let _ = writeln!(out, "{message}");
            let _ = out.flush();
        }
    }
}

/// Which requests were cancelled: the one running, by its flag, and ones
/// not yet begun, by id.
#[derive(Default)]
struct Cancels {
    running: Option<(String, Arc<AtomicBool>)>,
    early: HashSet<String>,
}

/// A request handed from the reader to the worker.
struct Incoming {
    id: Value,
    method: String,
    params: Value,
}

/// Serves MCP on `input` and `output` until `input` ends, with `backend`
/// behind every tool. Returns once the last request has been answered and
/// the backend has finished.
pub fn serve(
    input: impl BufRead + Send + 'static,
    output: impl Write + Send + 'static,
    mut backend: Box<dyn Backend>,
) {
    let output = Output(Arc::new(Mutex::new(Box::new(output))));
    let cancels: Arc<Mutex<Cancels>> = Arc::default();
    let (requests, queue) = mpsc::channel::<Incoming>();
    let reader = {
        let output = output.clone();
        let cancels = Arc::clone(&cancels);
        std::thread::Builder::new()
            .name("concat mcp read".to_owned())
            .spawn(move || read(input, &output, &cancels, &requests))
    };
    if reader.is_err() {
        eprintln!("concat mcp: could not start the reader");
        return;
    }
    let toolbox = Toolbox::new();
    let mut server = Server {
        toolbox,
        output: output.clone(),
        cancels,
    };
    for incoming in queue {
        server.handle(incoming, backend.as_mut());
    }
    backend.finish();
}

/// Reads lines until the input ends: answers what needs no backend at
/// once, and queues the rest.
fn read(
    input: impl BufRead,
    output: &Output,
    cancels: &Mutex<Cancels>,
    requests: &mpsc::Sender<Incoming>,
) {
    let mut input = input;
    loop {
        let mut line = String::new();
        match std::io::Read::take(&mut input, MAX_LINE).read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        if line.trim().is_empty() {
            continue;
        }
        let message = match serde_json::from_str::<Value>(&line) {
            Ok(message) => message,
            Err(error) => {
                output.send(&failure(Value::Null, code::PARSE, &format!("not JSON: {error}"), None));
                continue;
            }
        };
        let Value::Object(mut message) = message else {
            // A batch, or a bare value: neither is a message since
            // 2025-06-18.
            output.send(&failure(
                Value::Null,
                code::INVALID_REQUEST,
                "a message is one JSON-RPC object; batches are not supported",
                None,
            ));
            continue;
        };
        let method = message.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = message.remove("id");
        let params = message.remove("params").unwrap_or(Value::Null);
        match (method, id) {
            // A response from the client: this server sends no requests.
            (None, _) => {}
            (Some(method), None) => notification(&method, &params, cancels),
            (Some(_), Some(id)) if !(id.is_string() || id.is_i64() || id.is_u64()) => {
                output.send(&failure(
                    Value::Null,
                    code::INVALID_REQUEST,
                    "a request's id is a string or an integer",
                    None,
                ));
            }
            (Some(method), Some(id)) if method == "ping" => {
                output.send(&success(&id, json!({}), &params));
            }
            (Some(method), Some(id)) => {
                if requests.send(Incoming { id, method, params }).is_err() {
                    return;
                }
            }
        }
    }
}

/// A notification: a cancellation is acted on; the rest need nothing.
fn notification(method: &str, params: &Value, cancels: &Mutex<Cancels>) {
    if method != "notifications/cancelled" {
        return;
    }
    let Some(id) = params.get("requestId") else {
        return;
    };
    let key = id.to_string();
    let Ok(mut cancels) = cancels.lock() else {
        return;
    };
    match &cancels.running {
        Some((running, flag)) if *running == key => flag.store(true, Ordering::Relaxed),
        _ => {
            if cancels.early.len() < EARLY_CANCELS {
                cancels.early.insert(key);
            }
        }
    }
}

/// A result, with the modern era's additions when the request was modern.
fn success(id: &Value, mut result: Value, params: &Value) -> Value {
    if modern_version(params).is_some()
        && let Some(object) = result.as_object_mut()
    {
        object.insert("resultType".to_owned(), json!("complete"));
        let meta = object
            .entry("_meta")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(META_SERVER.to_owned(), server_info());
        }
    }
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

/// The protocol version a request names in its `_meta`, if it names one.
fn modern_version(params: &Value) -> Option<&str> {
    params
        .get("_meta")
        .and_then(|meta| meta.get(META_VERSION))
        .and_then(Value::as_str)
}

struct Server {
    toolbox: Toolbox,
    output: Output,
    cancels: Arc<Mutex<Cancels>>,
}

impl Server {
    fn handle(&mut self, incoming: Incoming, backend: &mut dyn Backend) {
        let Incoming { id, method, params } = incoming;
        let key = id.to_string();
        let flag = Arc::new(AtomicBool::new(false));
        if let Ok(mut cancels) = self.cancels.lock() {
            if cancels.early.remove(&key) {
                return;
            }
            cancels.running = Some((key, Arc::clone(&flag)));
        }
        let reply = self.answer(&id, &method, &params, backend, &flag);
        if let Ok(mut cancels) = self.cancels.lock() {
            cancels.running = None;
        }
        if !flag.load(Ordering::Relaxed) {
            self.output.send(&reply);
        }
    }

    fn answer(
        &mut self,
        id: &Value,
        method: &str,
        params: &Value,
        backend: &mut dyn Backend,
        cancelled: &AtomicBool,
    ) -> Value {
        // A modern request is checked before anything else: its version,
        // then the metadata every modern request must carry.
        if let Some(version) = modern_version(params) {
            if !supported().contains(&version) {
                return failure(
                    id.clone(),
                    code::UNSUPPORTED_VERSION,
                    "Unsupported protocol version",
                    Some(json!({ "supported": supported(), "requested": version })),
                );
            }
            if version == MODERN
                && params
                    .pointer(&format!("/_meta/{}", META_CAPABILITIES.replace('/', "~1")))
                    .is_none()
            {
                return failure(
                    id.clone(),
                    code::INVALID_PARAMS,
                    &format!("a {MODERN} request carries {META_CAPABILITIES} in _meta"),
                    None,
                );
            }
        }
        match method {
            "initialize" => {
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                // The version asked for when it is one of the handshake
                // era's this server speaks; the newest of them otherwise,
                // which the client may then decline.
                let version = LEGACY
                    .iter()
                    .find(|known| **known == asked)
                    .copied()
                    .unwrap_or(LEGACY[0]);
                success(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": capabilities(),
                        "serverInfo": server_info(),
                        "instructions": INSTRUCTIONS,
                    }),
                    params,
                )
            }
            "server/discover" => {
                let mut result = json!({
                    "supportedVersions": supported(),
                    "capabilities": capabilities(),
                    "instructions": INSTRUCTIONS,
                });
                let mut meta = Map::new();
                meta.insert(META_SERVER.to_owned(), server_info());
                result["_meta"] = Value::Object(meta);
                let mut reply = success(id, result, params);
                reply["result"]["resultType"] = json!("complete");
                reply
            }
            "tools/list" => success(id, json!({ "tools": self.toolbox.list() }), params),
            "tools/call" => self.call(id, params, backend, cancelled),
            _ => failure(
                id.clone(),
                code::METHOD_NOT_FOUND,
                &format!("Method not found: {method}"),
                None,
            ),
        }
    }

    fn call(
        &mut self,
        id: &Value,
        params: &Value,
        backend: &mut dyn Backend,
        cancelled: &AtomicBool,
    ) -> Value {
        let Some(name) = params.get("name").and_then(Value::as_str) else {
            return failure(id.clone(), code::INVALID_PARAMS, "tools/call names a tool", None);
        };
        let arguments = params.get("arguments").cloned().unwrap_or(Value::Null);
        if !(arguments.is_object() || arguments.is_null()) {
            return failure(
                id.clone(),
                code::INVALID_PARAMS,
                "a tool's arguments are an object",
                None,
            );
        }
        let token = params
            .pointer("/_meta/progressToken")
            .filter(|token| token.is_string() || token.is_i64() || token.is_u64())
            .cloned();
        let output = self.output.clone();
        let report = move |progress: f64, total: f64, message: String| {
            if let Some(token) = &token {
                output.send(&json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/progress",
                    "params": {
                        "progressToken": token,
                        "progress": progress,
                        "total": total,
                        "message": message,
                    },
                }));
            }
        };
        let wants_progress = params.pointer("/_meta/progressToken").is_some();
        let context = Context {
            backend,
            cancelled,
            progress: wants_progress.then_some(&report as &dyn Fn(f64, f64, String)),
        };
        // A panic in a tool is that call's failure, not the server's end.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.toolbox.call(name, arguments, context)
        }));
        match outcome {
            Ok(Some(outcome)) => {
                let mut result = json!({ "content": outcome.content, "isError": outcome.is_error });
                if let Some(structured) = outcome.structured {
                    result["structuredContent"] = structured;
                }
                success(id, result, params)
            }
            Ok(None) => failure(
                id.clone(),
                code::INVALID_PARAMS,
                &format!("Unknown tool: {name}"),
                None,
            ),
            Err(_) => failure(
                id.clone(),
                code::INTERNAL,
                &format!("the {name} tool failed inside the server"),
                None,
            ),
        }
    }
}
