// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! MCP over HTTP: the Model Context Protocol surface a local AI client
//! drives the window through, no helper process needed.
//!
//! Brought up together with the line-JSON socket by `--remote-control` or
//! `CONCAT_REMOTE=1`. A second listener on 127.0.0.1 (ephemeral port,
//! written to `remote-mcp` beside `remote-port`) speaks MCP's
//! Streamable-HTTP transport: one `POST /mcp` per message, one JSON
//! response back, no session state, no server-initiated streams. An MCP
//! client therefore points at `http://127.0.0.1:<port>/mcp` and reaches
//! the same studio the line-JSON socket serves - both transports share
//! one dispatch ([`remote::call`]), so nothing here decides what an edit
//! means; that is `concat-project`'s, and the wire shapes are
//! `concat-api`'s.
//!
//! The tool table is a faithful port of the standalone Python bridge
//! (`mcp/concat_mcp.py`): same names, same descriptions, same result
//! shapes, so an agent's habits transfer unchanged.
//!
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use concat_api::message::{ExportSpec, Request};
use concat_project::model::VideoSettings;

/// Sent when the service's own state cannot be read; a poisoned lock
/// means a thread died mid-toggle and the process is on its way out.
const LOCK_POISONED: &str = "MCP service state is unavailable";

/// The MCP protocol revision this build speaks. Streamable HTTP arrived
/// with 2025-03-26; a client proposing a newer known revision gets that
/// echoed back, anything else falls back to this one.
const PROTOCOL_VERSION: &str = "2025-03-26";

/// Revisions this server can answer with.
const KNOWN_VERSIONS: [&str; 2] = ["2025-03-26", "2025-06-18"];

/// Ceiling on one request body; a tool call carries at most an edit
/// command, and a bound keeps a broken client from allocating us into
/// oblivion.
const MAX_BODY: usize = 8 * 1024 * 1024;

/// Where the MCP-over-HTTP listener sits by default. Fixed so a client's
/// saved URL keeps working across launches; when something else owns the
/// number, the bind walks up a little and the real address still lands in
/// `remote-mcp`.
const DEFAULT_PORT: u16 = 9360;

/// How far past the default the walk may go before giving up.
const PORT_ATTEMPTS: u16 = 16;

/// The live listener, when there is one. Stopping takes the option away
/// and flips the flag: the accept loop wakes on a throwaway loopback
/// connect, sees the flag, and exits; each open connection finishes its
/// current request and then closes.
struct Service {
    running: Arc<AtomicBool>,
    port: u16,
    endpoint: String,
}

static SERVICE: Mutex<Option<Service>> = Mutex::new(None);

/// Starts the service when asked to, beside the line-JSON socket: forced
/// on by `--remote-control` or `CONCAT_REMOTE=1`, otherwise listening
/// only while the remembered preference says so. Never fatal: a machine
/// where nothing can be started simply runs without MCP over HTTP, the
/// same way it runs without recents.
pub fn start() {
    let forced = std::env::args().any(|arg| arg == "--remote-control")
        || std::env::var("CONCAT_REMOTE").is_ok_and(|value| value == "1");
    let remembered = !forced
        && concat_host::AppDirs::locate()
            .map(|dirs| crate::prefs::Preferences::load(&dirs).mcp_enabled)
            .unwrap_or(false);
    if forced || remembered {
        let _ = ensure_started();
    }
}

/// Brings the listener up if it is not already, and answers with the
/// endpoint URL. An explicitly requested `CONCAT_MCP_PORT` has to be
/// free exactly there; the default walk from [`DEFAULT_PORT`] dodges the
/// rare day something else owns the number.
pub fn ensure_started() -> Result<String, String> {
    let mut slot = SERVICE.lock().map_err(|_| LOCK_POISONED.to_owned())?;
    if let Some(service) = slot.as_ref() {
        return Ok(service.endpoint.clone());
    }
    let dirs = concat_host::AppDirs::locate()?;
    let requested = std::env::var("CONCAT_MCP_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok());
    let (listener, port) = match requested {
        Some(port) => TcpListener::bind(("127.0.0.1", port))
            .map(|listener| (listener, port))
            .map_err(|error| format!("CONCAT_MCP_PORT {port} is taken: {error}"))?,
        None => (0..PORT_ATTEMPTS)
            .find_map(|offset| {
                let port = DEFAULT_PORT.saturating_add(offset);
                TcpListener::bind(("127.0.0.1", port))
                    .map(|listener| (listener, port))
                    .ok()
            })
            .ok_or_else(|| {
                format!(
                    "no free port between {DEFAULT_PORT} and {}",
                    DEFAULT_PORT + PORT_ATTEMPTS - 1
                )
            })?,
    };
    let endpoint = format!("http://127.0.0.1:{port}/mcp");
    std::fs::create_dir_all(&dirs.config).map_err(|error| error.to_string())?;
    let file = dirs.config.join("remote-mcp");
    std::fs::write(&file, format!("{endpoint}\n")).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    if let Ok(metadata) = std::fs::metadata(&file) {
        let mut permissions = metadata.permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(&file, permissions);
    }
    let running = Arc::new(AtomicBool::new(true));
    let accept_running = Arc::clone(&running);
    let spawned = std::thread::Builder::new()
        .name("concat-mcp".into())
        .spawn(move || {
            for stream in listener.incoming() {
                if !accept_running.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let running = Arc::clone(&accept_running);
                let _ = std::thread::Builder::new()
                    .name("concat-mcp-conn".into())
                    .spawn(move || serve_connection(stream, running));
            }
        });
    if let Err(error) = spawned {
        eprintln!("concat: MCP over HTTP off - {error}");
        return Err(error.to_string());
    }
    eprintln!("concat: MCP over HTTP on {endpoint}");
    *slot = Some(Service {
        running,
        port,
        endpoint: endpoint.clone(),
    });
    Ok(endpoint)
}

/// Takes the listener down: new connections are refused, connections
/// already being served finish their current request and close. The
/// endpoint file goes with it.
pub fn stop() {
    let Ok(mut slot) = SERVICE.lock() else { return };
    let Some(service) = slot.take() else { return };
    service.running.store(false, Ordering::Relaxed);
    // Dropping a listener does not necessarily wake a blocked accept on
    // every platform; a throwaway loopback connect does.
    let _ = TcpStream::connect(("127.0.0.1", service.port));
    if let Ok(dirs) = concat_host::AppDirs::locate() {
        let _ = std::fs::remove_file(dirs.config.join("remote-mcp"));
    }
    eprintln!("concat: MCP over HTTP off");
}

/// Whether the listener is up right now.
pub fn is_running() -> bool {
    SERVICE.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

/// The endpoint URL while the listener is up.
pub fn endpoint() -> Option<String> {
    SERVICE
        .lock()
        .ok()
        .and_then(|slot| slot.as_ref().map(|service| service.endpoint.clone()))
}

/// One keep-alive HTTP connection: requests until the client hangs up or
/// asks to close. A malformed request ends the connection, as a server
/// may when it cannot find the request boundary.
fn serve_connection(stream: TcpStream, running: Arc<AtomicBool>) {
    let Ok(read_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_stream);
    let mut writer = stream;
    loop {
        if !running.load(Ordering::Relaxed) {
            return;
        }
        let request = match read_request(&mut reader) {
            Ok(Some(request)) => request,
            Ok(None) | Err(_) => return,
        };
        let (status, body) = route(&request);
        if write_response(&mut writer, status, &body).is_err() || request.close {
            return;
        }
    }
}

/// One parsed HTTP request head plus body: only what MCP over HTTP needs.
struct HttpRequest {
    method: String,
    path: String,
    origin: Option<String>,
    close: bool,
    body: Vec<u8>,
}

/// Writes one HTTP response head and body. Always JSON; the 202 answer
/// for a notification simply has no body.
fn write_response(writer: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len(),
    );
    writer.write_all(head.as_bytes())?;
    writer.write_all(body.as_bytes())?;
    writer.flush()
}

/// Reads one request off the wire. `Ok(None)` is a clean end of stream.
fn read_request(reader: &mut BufReader<impl Read>) -> std::io::Result<Option<HttpRequest>> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or("").to_owned();
    let path = parts.next().unwrap_or("").to_owned();
    let version = parts.next().unwrap_or("HTTP/1.1").to_owned();
    let mut content_length = 0usize;
    let mut origin = None;
    let mut close = version == "HTTP/1.0";
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            return Ok(None);
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            match name.as_str() {
                "content-length" => content_length = value.parse().unwrap_or(0),
                "origin" => origin = Some(value.to_owned()),
                "connection" => close = value.eq_ignore_ascii_case("close"),
                _ => {}
            }
        }
    }
    let mut body = vec![0u8; content_length.min(MAX_BODY)];
    reader.read_exact(&mut body)?;
    Ok(Some(HttpRequest {
        method,
        path,
        origin,
        close,
        body,
    }))
}

/// Turns one request into one response: the whole HTTP surface is a
/// single endpoint that speaks JSON-RPC.
fn route(request: &HttpRequest) -> (u16, String) {
    if request.method != "POST" {
        return (
            405,
            rpc_error(Value::Null, -32600, "MCP over HTTP speaks POST").to_string(),
        );
    }
    if request.path != "/mcp" {
        return (
            404,
            rpc_error(Value::Null, -32600, "no such endpoint").to_string(),
        );
    }
    if request
        .origin
        .as_deref()
        .is_some_and(|origin| !origin_is_loopback(origin))
    {
        return (
            403,
            rpc_error(Value::Null, -32600, "cross-origin MCP is refused").to_string(),
        );
    }
    let message: Value = match serde_json::from_slice(&request.body) {
        Ok(message) => message,
        Err(error) => {
            return (
                400,
                rpc_error(Value::Null, -32700, &format!("parse error: {error}")).to_string(),
            );
        }
    };
    match handle(&message) {
        Some(response) => (200, response.to_string()),
        None => (202, String::new()),
    }
}

/// The host inside an Origin header, without scheme, port or brackets.
fn origin_host(origin: &str) -> &str {
    let rest = origin
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(origin);
    let host_port = rest.split('/').next().unwrap_or(rest);
    if let Some(bracketed) = host_port.strip_prefix('[') {
        bracketed
            .split_once(']')
            .map(|(host, _)| host)
            .unwrap_or(bracketed)
    } else {
        host_port
            .split_once(':')
            .map(|(host, _)| host)
            .unwrap_or(host_port)
    }
}

/// Loopback only: the DNS-rebinding guard. A web page must not reach the
/// studio through the user's browser.
fn origin_is_loopback(origin: &str) -> bool {
    matches!(origin_host(origin), "127.0.0.1" | "localhost" | "::1")
}

/// Dispatches one JSON-RPC message. `None` is a notification: nothing to
/// answer, and HTTP answers it with 202.
fn handle(message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let id = message.get("id").cloned().unwrap_or(Value::Null);
    if id.is_null() || method.starts_with("notifications/") {
        return None;
    }
    match method {
        "initialize" => {
            let requested = message
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION);
            let version = if KNOWN_VERSIONS.contains(&requested) {
                requested
            } else {
                PROTOCOL_VERSION
            };
            Some(reply(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "concat", "version": env!("CARGO_PKG_VERSION")},
                }),
            ))
        }
        "ping" => Some(reply(id, json!({}))),
        "tools/list" => Some(reply(id, json!({"tools": tools()}))),
        "tools/call" => {
            let name = message
                .pointer("/params/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = message
                .pointer("/params/arguments")
                .cloned()
                .unwrap_or(json!({}));
            Some(reply(id, dispatch_tool(name, &arguments)))
        }
        other => Some(rpc_error(id, -32601, &format!("Method not found: {other}"))),
    }
}

fn reply(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_text(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}]})
}

fn tool_error(text: String) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": true})
}

/// Common refusal sentences and the next move that fixes them. Guards
/// make mistakes recoverable; these hints make recovery one turn instead
/// of three. Ported from the Python bridge so both surfaces advise alike.
const ERROR_HINTS: [(&str, &str); 5] = [
    (
        "No project is open",
        "Run project_create (new) or project_open (existing) first.",
    ),
    (
        "is not the open project",
        "Call project_get with the path project_get returned before - it names the project the window actually has open.",
    ),
    (
        "No such file",
        "Check the path exists and is readable from the machine running Concat.",
    ),
    (
        "nothing on the timeline",
        "The timeline is empty: media_import a file, then edit with op addClipAtFirstFree (or use add_media_to_timeline for both at once).",
    ),
    (
        "No filter on this clip yet",
        "Filters are applied with edit op updateClip - see catalogue(kind='filter') for ids.",
    ),
];

fn with_hint(text: &str) -> String {
    for (pattern, hint) in ERROR_HINTS {
        if text.contains(pattern) {
            return format!("{text}\nHINT: {hint}");
        }
    }
    if text.starts_with("Error:") {
        return format!(
            "{text}\nHINT: If an id was rejected, call project_get for the current clip/media ids; \
             if a command seemed to do nothing, it may have been a tolerated no-op (e.g. importing \
             a file already in the bin)."
        );
    }
    text.to_owned()
}

/// The command vocabulary the `edit` tool carries in its description, so
/// an agent can build one without reading `concat-project`.
const EDIT_EXAMPLES: &str = "\
Command vocabulary (serde tag 'op', camelCase fields), common ops:
  {\"op\": \"addTextClip\", \"trackId\": null, \"start\": 0.0, \"duration\": 3.0, \"style\": {\"content\": \"Hello\", \"fontSize\": 0.08, \"color\": \"#ffffff\"}}
  {\"op\": \"addClip\", \"mediaId\": \"m1\", \"trackId\": null, \"start\": 0.0}
  {\"op\": \"splitClips\", \"clipIds\": [\"c1\"], \"time\": 2.5}
  {\"op\": \"trimClip\", \"clipId\": \"c1\", \"edge\": \"start\", \"delta\": 0.5}
  {\"op\": \"moveClips\", \"moves\": [{\"clipId\": \"c1\", \"trackId\": \"t2\", \"start\": 4.0}]}
  {\"op\": \"batch\", \"commands\": [ ... ]}  (atomic multi-op)
Unknown clip/media ids are TOLERATED SILENT NO-OPS - the call succeeds but changes nothing. After edits, verify with workflow_state. Refusals (unsaved changes, impossible moves) arrive as the sentence the window would show. project_get returns every clip's id.";

/// The tool table, names and schemas first: what `tools/list` answers.
fn tools() -> Value {
    let path_schema = json!({
        "type": "object",
        "properties": {"path": {"type": "string"}},
        "required": ["path"],
    });
    json!([
        {
            "name": "concat_version",
            "description": "Concat API version and build info.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "project_create",
            "description": "Create a new project folder and open it. Defaults to 1080p at 30fps.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "location": {"type": "string", "description": "Parent directory for the project folder"},
                    "name": {"type": "string"},
                    "width": {"type": "integer", "description": "Frame width, default 1920"},
                    "height": {"type": "integer", "description": "Frame height, default 1080"},
                    "fps": {"type": "number", "description": "Frame rate, default 30"},
                },
                "required": ["location", "name"],
            },
        },
        {
            "name": "project_open",
            "description": "Open an existing project folder.",
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]},
        },
        {
            "name": "project_get",
            "description": "Full state of an open project: timelines, tracks, every clip with its id - the ids the edit vocabulary needs.",
            "inputSchema": path_schema,
        },
        {
            "name": "project_save",
            "description": "Write the project document to its folder.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "name": {"type": "string", "description": "Rename the project"},
                },
                "required": ["path"],
            },
        },
        {
            "name": "project_list",
            "description": "Projects recently opened on this machine, newest first.",
            "inputSchema": {"type": "object", "properties": {}},
        },
        {
            "name": "media_probe",
            "description": "Inspect a media file: duration, video stream, audio tracks. No project needed.",
            "inputSchema": {"type": "object", "properties": {"file": {"type": "string"}}, "required": ["file"]},
        },
        {
            "name": "media_import",
            "description": "Probe a file and add it to a project's bin (what dropping a file does). Returns the minted media id.",
            "inputSchema": {
                "type": "object",
                "properties": {"project": {"type": "string"}, "file": {"type": "string"}},
                "required": ["project", "file"],
            },
        },
        {
            "name": "catalogue",
            "description": "Every built-in effect package with its parameters and value ranges. kind: effect | filter | audio | transition | generator.",
            "inputSchema": {"type": "object", "properties": {"kind": {"type": "string"}}},
        },
        {
            "name": "edit",
            "description": format!(
                "Apply one edit command to an open project. This is the full vocabulary the window \
                 has - the command rides through unchanged.\n{EDIT_EXAMPLES}"
            ),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {"type": "string"},
                    "command": {"type": "object", "description": "One Command object, tagged by 'op'"},
                },
                "required": ["project", "command"],
            },
        },
        {
            "name": "edit_undo",
            "description": "Step the project's history back one edit.",
            "inputSchema": path_schema,
        },
        {
            "name": "edit_redo",
            "description": "Step the project's history forward one edit.",
            "inputSchema": path_schema,
        },
        {
            "name": "export_video",
            "description": "Render an open project to an MP4 file (H.264), exactly as the window's Export does. Blocks until finished; progress arrives as the tool result's progress lines.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {"type": "string"},
                    "output": {"type": "string", "description": "Output .mp4 path"},
                    "crf": {"type": "integer", "description": "Quality, lower is better/bigger (default 20)"},
                    "preset": {"type": "string", "description": "x264 preset (default medium)"},
                    "width": {"type": "integer"},
                    "height": {"type": "integer"},
                    "fps": {"type": "number"},
                },
                "required": ["project", "output"],
            },
        },
        {
            "name": "preview_frame",
            "description": "Composite the true frame at one instant and write it as a PNG.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {"type": "string"},
                    "time": {"type": "number", "description": "Timeline instant, seconds"},
                    "output": {"type": "string", "description": "Output .png path"},
                    "width": {"type": "integer"},
                    "height": {"type": "integer"},
                },
                "required": ["project", "time", "output"],
            },
        },
        {
            "name": "add_media_to_timeline",
            "description": "One-step: import a media file into the project bin AND drop it on the first free track. Returns both the media id and the clip id. Equivalent to media_import followed by edit(addClipAtFirstFree).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {"type": "string"},
                    "file": {"type": "string"},
                    "start": {"type": "number", "description": "Timeline position, seconds (default 0)"},
                },
                "required": ["project", "file"],
            },
        },
        {
            "name": "workflow_state",
            "description": "Where am I? One glance: the open project, media in the bin, clips on the active timeline, undo availability. No arguments needed - the window's own project answers. Call this whenever unsure what to do next, and after any edit to confirm ids.",
            "inputSchema": {
                "type": "object",
                "properties": {"project": {"type": "string", "description": "Omit for the project the window has open"}},
            },
        },
        {
            "name": "template_list",
            "description": "The template library.",
            "inputSchema": {"type": "object", "properties": {}},
        },
    ])
}

/// Routes one tool call: the composites first, then the plain table of
/// request builders. Tool failures come back as `isError` content, never
/// as JSON-RPC errors - the call itself succeeded.
fn dispatch_tool(name: &str, arguments: &Value) -> Value {
    match name {
        "add_media_to_timeline" => add_media_to_timeline(arguments),
        "workflow_state" => workflow_state(arguments),
        other => match build_request(other, arguments) {
            Ok(request) => tool_from_call(request),
            Err(error) => tool_error(error),
        },
    }
}

/// Sends one request through the window and folds its event stream into
/// the tool text: progress lines first, the result value last - the same
/// shape the Python bridge taught agents to read.
fn tool_from_call(request: Request) -> Value {
    match crate::remote::call(request) {
        Ok((result, events)) => {
            let mut lines: Vec<String> = events
                .iter()
                .map(|event| {
                    let stage = event
                        .get("stage")
                        .and_then(Value::as_str)
                        .or_else(|| event.get("event").and_then(Value::as_str))
                        .unwrap_or("");
                    match (event.get("frame"), event.get("total")) {
                        (Some(frame), Some(total)) => {
                            format!("[progress] {stage} frame {frame}/{total}")
                        }
                        _ => format!("[event] {event}"),
                    }
                })
                .collect();
            lines.push(result.to_string());
            tool_text(lines.join("\n"))
        }
        Err(error) => tool_error(with_hint(&format!("Error: {error}"))),
    }
}

/// Builds the Concat API request a table tool stands for. Pure so the
/// mappings are testable without a running window.
fn build_request(name: &str, args: &Value) -> Result<Request, String> {
    match name {
        "concat_version" => Ok(Request::Version),
        "project_create" => {
            let fps = args.get("fps").and_then(Value::as_f64).unwrap_or(30.0);
            Ok(Request::ProjectCreate {
                location: text_arg(args, "location")?,
                name: text_arg(args, "name")?,
                video: Some(VideoSettings {
                    width: u32_arg(args, "width").unwrap_or(1920),
                    height: u32_arg(args, "height").unwrap_or(1080),
                    rate_num: (fps * 1000.0).round() as i64,
                    rate_den: 1000,
                }),
            })
        }
        "project_open" => Ok(Request::ProjectOpen {
            path: text_arg(args, "path")?,
        }),
        "project_get" => Ok(Request::ProjectGet {
            path: text_arg(args, "path")?,
        }),
        "project_save" => Ok(Request::ProjectSave {
            path: text_arg(args, "path")?,
            name: args.get("name").and_then(Value::as_str).map(str::to_owned),
        }),
        "project_list" => Ok(Request::ProjectList),
        "media_probe" => Ok(Request::MediaProbe {
            path: text_arg(args, "file")?,
        }),
        "media_import" => Ok(Request::MediaImport {
            path: text_arg(args, "project")?,
            file: text_arg(args, "file")?,
        }),
        "catalogue" => Ok(Request::CatalogueList {
            kind: args.get("kind").and_then(Value::as_str).map(str::to_owned),
        }),
        "edit" => {
            let command =
                serde_json::from_value(args.get("command").cloned().unwrap_or(Value::Null))
                    .map_err(|error| format!("not a command: {error}"))?;
            Ok(Request::EditApply {
                path: text_arg(args, "project")?,
                command: Box::new(command),
            })
        }
        "edit_undo" => Ok(Request::EditUndo {
            path: text_arg(args, "path")?,
        }),
        "edit_redo" => Ok(Request::EditRedo {
            path: text_arg(args, "path")?,
        }),
        "export_video" => {
            let fps = args.get("fps").and_then(Value::as_f64);
            Ok(Request::ExportRun {
                path: text_arg(args, "project")?,
                spec: ExportSpec {
                    output: text_arg(args, "output")?,
                    crf: args
                        .get("crf")
                        .and_then(Value::as_u64)
                        .map(|value| value as u8),
                    preset: args
                        .get("preset")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    width: u32_opt(args, "width"),
                    height: u32_opt(args, "height"),
                    rate_num: fps.map(|fps| (fps * 1000.0).round() as i64),
                    rate_den: fps.map(|_| 1000),
                },
            })
        }
        "preview_frame" => Ok(Request::PreviewFrame {
            path: text_arg(args, "project")?,
            time: args
                .get("time")
                .and_then(Value::as_f64)
                .ok_or("missing required argument: time")?,
            output: text_arg(args, "output")?,
            width: u32_opt(args, "width"),
            height: u32_opt(args, "height"),
        }),
        "template_list" => Ok(Request::TemplateList),
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn text_arg(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing required argument: {key}"))
}

fn u32_arg(args: &Value, key: &str) -> Option<u32> {
    args.get(key)
        .and_then(Value::as_u64)
        .map(|value| value as u32)
}

fn u32_opt(args: &Value, key: &str) -> Option<u32> {
    u32_arg(args, key)
}

/// Composite: import the file, then drop it on the first free track. A
/// project named but never created is made (or opened) on the way, the
/// same forgiveness the Python bridge taught.
fn add_media_to_timeline(arguments: &Value) -> Value {
    let project = match text_arg(arguments, "project") {
        Ok(value) => value,
        Err(error) => return tool_error(error),
    };
    let file = match text_arg(arguments, "file") {
        Ok(value) => value,
        Err(error) => return tool_error(error),
    };
    let start = arguments
        .get("start")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let import = || {
        crate::remote::call(Request::MediaImport {
            path: project.clone(),
            file: file.clone(),
        })
    };
    let response = match import() {
        // The project does not exist yet: make it (or open it, when a
        // folder of that name already holds a project), then retry.
        Err(error) if error.contains("is not open") => {
            let folder = Path::new(&project);
            let create = crate::remote::call(Request::ProjectCreate {
                location: folder
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_else(|| ".".to_owned()),
                name: folder
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_default(),
                video: None,
            });
            match create {
                Ok(_) => import(),
                Err(create_error) => {
                    if create_error.contains("already holds") {
                        match crate::remote::call(Request::ProjectOpen {
                            path: project.clone(),
                        }) {
                            Ok(_) => import(),
                            Err(open_error) => {
                                return tool_error(with_hint(&format!("Error: {open_error}")));
                            }
                        }
                    } else {
                        return tool_error(with_hint(&format!("Error: {create_error}")));
                    }
                }
            }
        }
        other => other,
    };
    let (result, _) = match response {
        Ok(pair) => pair,
        Err(error) => return tool_error(with_hint(&format!("Error: {error}"))),
    };
    let Some(media_id) = result.get("createdId").and_then(Value::as_str) else {
        return tool_text(
            "The file was already in the bin (tolerated no-op) - its media id was not re-minted. \
             Call project_get to find it, then edit with op addClipAtFirstFree."
                .to_owned(),
        );
    };
    match crate::remote::call(Request::EditApply {
        path: project,
        command: Box::new(concat_project::Command::AddClipAtFirstFree {
            media_id: media_id.to_owned(),
            start,
        }),
    }) {
        Ok((result, _)) => tool_text(format!(
            "Imported {file} as media {media_id}; clip {} placed at {start}s on the first free track.",
            result
                .get("createdId")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        Err(error) => tool_error(with_hint(&format!(
            "Error: {error} (media id {media_id} is in the bin)"
        ))),
    }
}

/// Composite: the window's own "where am I" plus the open project's
/// state, the one glance an agent needs before it drives.
fn workflow_state(arguments: &Value) -> Value {
    let window = crate::remote::window_state();
    let open = window
        .get("projectPath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let path = arguments
        .get("project")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            if open.is_empty() {
                None
            } else {
                Some(open.clone())
            }
        });
    let Some(path) = path else {
        return tool_error(with_hint("No project is open in the window."));
    };
    let (view, _) = match crate::remote::call(Request::ProjectGet { path: path.clone() }) {
        Ok(pair) => pair,
        Err(error) => return tool_error(with_hint(&format!("Error: {error}"))),
    };
    let media: Vec<String> = view["project"]["media"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("id").and_then(Value::as_str).map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let active_id = view["project"]["activeTimelineId"].as_str().unwrap_or("");
    let active = view["project"]["timelines"]
        .as_array()
        .and_then(|timelines| {
            timelines
                .iter()
                .find(|timeline| timeline.get("id").and_then(Value::as_str) == Some(active_id))
        });
    let clips: Vec<String> = active
        .and_then(|timeline| timeline.get("clips").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .map(|clip| {
            format!(
                "({}, {})",
                clip.get("id").and_then(Value::as_str).unwrap_or("?"),
                clip.get("name").and_then(Value::as_str).unwrap_or("?")
            )
        })
        .collect();
    let lines = [
        format!(
            "[state] [window] mode: {}, project: {}, export dialog: {} ({})",
            window.get("mode").and_then(Value::as_str).unwrap_or("?"),
            open,
            window
                .get("exportDialogOpen")
                .map(|value| value.to_string())
                .unwrap_or_else(|| "?".to_owned()),
            window
                .get("exportPhase")
                .and_then(Value::as_str)
                .unwrap_or("?"),
        ),
        format!("[state] open project: {path}"),
        format!("[state] media in bin: {media:?}"),
        format!(
            "[state] clips on '{}': {clips:?}",
            active
                .and_then(|timeline| timeline.get("name").and_then(Value::as_str))
                .unwrap_or("?")
        ),
        format!(
            "[state] undo available: {}, redo available: {}",
            view["canUndo"].as_bool().unwrap_or(false),
            view["canRedo"].as_bool().unwrap_or(false),
        ),
    ];
    let mut text = lines.join("\n");
    text.push('\n');
    text.push_str(&view.to_string());
    tool_text(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every table tool answers `tools/list` with the three fields the
    /// protocol requires, and no name repeats.
    #[test]
    fn the_tool_table_is_well_formed() {
        let tools = tools().as_array().expect("the table is an array").clone();
        assert!(tools.len() >= 17);
        let mut names = Vec::new();
        for tool in &tools {
            assert!(tool.get("name").and_then(Value::as_str).is_some());
            assert!(tool.get("description").and_then(Value::as_str).is_some());
            assert!(tool.get("inputSchema").is_some());
            names.push(tool["name"].as_str().unwrap().to_owned());
        }
        let unique = names.iter().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), names.len());
    }

    /// `project_create` fills the defaults the description promises:
    /// 1080p at 30, the rate carried as a rational with denominator 1000.
    #[test]
    fn project_create_fills_the_promised_defaults() {
        let request = build_request(
            "project_create",
            &json!({"location": "/tmp", "name": "demo"}),
        )
        .expect("the arguments are complete");
        let Request::ProjectCreate {
            location,
            name,
            video,
        } = request
        else {
            panic!("wrong variant");
        };
        assert_eq!(location, "/tmp");
        assert_eq!(name, "demo");
        let video = video.expect("defaults are filled in");
        assert_eq!((video.width, video.height), (1920, 1080));
        assert_eq!((video.rate_num, video.rate_den), (30000, 1000));
    }

    /// `export_video` passes only the options the caller named: an absent
    /// fps leaves the project's own rate untouched.
    #[test]
    fn export_video_passes_only_named_options() {
        let request = build_request(
            "export_video",
            &json!({"project": "/tmp/demo", "output": "/tmp/demo.mp4", "crf": 22}),
        )
        .expect("the arguments are complete");
        let Request::ExportRun { path, spec } = request else {
            panic!("wrong variant");
        };
        assert_eq!(path, "/tmp/demo");
        assert_eq!(spec.output, "/tmp/demo.mp4");
        assert_eq!(spec.crf, Some(22));
        assert_eq!(spec.preset, None);
        assert_eq!(spec.rate_num, None);
        assert_eq!(spec.rate_den, None);
    }

    /// An `edit` command rides through as the `Command` it names - here a
    /// title, the exact JSON the description teaches.
    #[test]
    fn edit_carries_the_command_through() {
        let request = build_request(
            "edit",
            &json!({
                "project": "/tmp/demo",
                "command": {"op": "addTextClip", "trackId": null, "start": 1.0,
                            "duration": 2.0,
                            "style": {"content": "Hello", "fontSize": 0.08, "color": "#ffffff"}},
            }),
        )
        .expect("the arguments are complete");
        let Request::EditApply { path, command } = request else {
            panic!("wrong variant");
        };
        assert_eq!(path, "/tmp/demo");
        assert!(matches!(
            *command,
            concat_project::Command::AddTextClip { .. }
        ));
    }

    /// An unknown tool is an argument error before any engine contact,
    /// and so is a missing required argument.
    #[test]
    fn unknown_tools_and_missing_arguments_error_without_the_engine() {
        assert!(build_request("no_such_tool", &json!({})).is_err());
        assert!(build_request("project_open", &json!({})).is_err());
    }

    /// `initialize` negotiates: a known revision echoes, an unknown one
    /// falls back to the build's own.
    #[test]
    fn initialize_negotiates_the_protocol_revision() {
        let answer = handle(&json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"},
        }))
        .expect("a request wants an answer");
        assert_eq!(answer["result"]["protocolVersion"], json!("2025-06-18"),);
        let answer = handle(&json!({
            "jsonrpc": "2.0", "id": 2, "method": "initialize",
            "params": {"protocolVersion": "1999-01-01"},
        }))
        .expect("a request wants an answer");
        assert_eq!(answer["result"]["protocolVersion"], json!(PROTOCOL_VERSION));
    }

    /// A notification needs no answer, an unknown method is the JSON-RPC
    /// -32601, and a call of an unknown tool is `isError` content rather
    /// than a protocol error.
    #[test]
    fn notifications_and_errors_take_the_protocol_shapes() {
        assert!(
            handle(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none()
        );
        let answer = handle(&json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}))
            .expect("a request wants an answer");
        assert_eq!(answer["error"]["code"], json!(-32601));
        let answer = handle(&json!({
            "jsonrpc": "2.0", "id": 4, "method": "tools/call",
            "params": {"name": "no_such_tool", "arguments": {}},
        }))
        .expect("a request wants an answer");
        assert_eq!(answer["result"]["isError"], json!(true));
    }

    /// The HTTP surface is one endpoint: POST /mcp. Anything else is
    /// refused before a body is parsed, and a cross-origin request is
    /// refused outright - the rebinding guard.
    #[test]
    fn http_route_guards_method_path_and_origin() {
        let request = |method: &str, origin: Option<&str>| HttpRequest {
            method: method.to_owned(),
            path: "/mcp".to_owned(),
            origin: origin.map(str::to_owned),
            close: true,
            body: br#"{"jsonrpc": "2.0", "id": 1, "method": "ping"}"#.to_vec(),
        };
        assert_eq!(route(&request("GET", None)).0, 405);
        let mut evil = request("POST", Some("http://evil.example:8080"));
        evil.path = "/mcp".to_owned();
        assert_eq!(route(&evil).0, 403);
        let mut elsewhere = request("POST", None);
        elsewhere.path = "/other".to_owned();
        assert_eq!(route(&elsewhere).0, 404);
        let (status, body) = route(&request("POST", Some("http://127.0.0.1:5173")));
        assert_eq!(status, 200);
        assert_eq!(
            body,
            json!({"jsonrpc": "2.0", "id": 1, "result": {}}).to_string()
        );
    }

    /// A request head is read by line, headers are case-insensitive, and
    /// the body is exactly Content-Length bytes.
    #[test]
    fn http_request_parsing_reads_head_and_body() {
        let body = br#"{"method":"ping","id":7}"#.as_slice();
        let wire = format!(
            "POST /mcp HTTP/1.1\r\ncontent-LENGTH: {}\r\nOrigin: http://localhost:3000\r\n\r\n",
            body.len(),
        ) + std::str::from_utf8(body).unwrap();
        let mut reader = BufReader::new(wire.as_bytes());
        let request = read_request(&mut reader)
            .expect("the wire is sound")
            .expect("a request");
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/mcp");
        assert_eq!(request.origin.as_deref(), Some("http://localhost:3000"));
        assert_eq!(request.body, body);
    }

    /// The hints turn the usual refusals into one-turn recoveries.
    #[test]
    fn refusals_carry_their_next_move() {
        let hinted = with_hint("Error: /tmp/demo is not the open project");
        assert!(hinted.contains("HINT:"));
        assert!(
            !with_hint("Error: something unspecific").contains("HINT:")
                || with_hint("Error: something unspecific")
                    .contains("may have been a tolerated no-op")
        );
    }

    /// Origins strip scheme, port and brackets down to their host.
    #[test]
    fn origin_hosts_reduce_to_their_host() {
        assert_eq!(origin_host("http://127.0.0.1:9000"), "127.0.0.1");
        assert_eq!(origin_host("http://localhost"), "localhost");
        assert_eq!(origin_host("http://[::1]:9000"), "::1");
        assert_eq!(origin_host("null"), "null");
    }
}
