// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! Where a tool's request goes: the window's server, or an API of this
//! process's own.
//!
//! Both take a [`Request`] and give back its result as JSON, so a tool
//! cannot tell them apart and the meaning of every request stays the
//! API's. A third backend - the window's own open project, once the
//! window can be asked to edit it ("live mode", #337) - is one more
//! implementation of [`Backend`], with nothing above it changing.
//!
//! - [`Window`]: a JSON-RPC connection to the server the Automation page
//!   runs. Found through the listing the window writes
//!   (`concat_server::discovery`) or named by `CONCAT_API_ADDRESS` and
//!   `CONCAT_API_TOKEN`. Goes through the window's register of open
//!   projects, so the project on screen cannot be opened here and saved
//!   over, and through its one export slot.
//! - [`Headless`]: an [`Api`] in this process, writing under the user's
//!   home (or `CONCAT_MCP_ROOTS`). Nothing to start first, and nothing
//!   shared with a window: a project open in the window can be opened here
//!   too, which is why the window's server is preferred when it is there.

#[cfg(target_os = "android")]
use std::io::{BufRead, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use concat_api::{Api, ApiError, AppDirs, Request, Response, Stopper};
use concat_server::client::Client;
use concat_server::discovery;
use serde_json::Value;

/// How long a connection to the window's server may take to open.
const PATIENCE: Duration = Duration::from_secs(3);

/// The longest reply line read from a LocalSocket: a project's whole
/// document fits.
#[cfg(target_os = "android")]
const MAX_REPLY: u64 = 256 * 1024 * 1024;

/// What a tool's request is handed to.
pub trait Backend: Send {
    /// Runs one request; its result as the JSON the API replies with.
    fn call(&mut self, request: Request) -> Result<Value, ApiError>;

    /// What this backend is, for the agent and the log: "window" or
    /// "headless", and where.
    fn describe(&self) -> Value;

    /// The session is over: stop what is running, and wait for it.
    fn finish(&mut self) {}
}

/// Which backend `concat --mcp` uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Mode {
    /// The window's server when it can be reached, this process otherwise.
    #[default]
    Auto,
    /// The window's server (TCP), or an error saying how to turn it on.
    Window,
    /// The window's server via Android LocalSocket.
    #[cfg(target_os = "android")]
    WindowLocal,
    /// This process, whatever else is running.
    Headless,
}

impl Mode {
    /// The mode a word names: "auto", "window", "window-local" or "headless".
    pub fn parse(word: &str) -> Option<Mode> {
        match word.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(Mode::Auto),
            "window" => Some(Mode::Window),
            #[cfg(target_os = "android")]
            "window-local" => Some(Mode::WindowLocal),
            "headless" => Some(Mode::Headless),
            _ => None,
        }
    }
}

/// The backend `mode` asks for, with the reason for whatever was not
/// chosen written to standard error, where an MCP client shows a server's
/// log.
pub fn open(mode: Mode) -> Result<Box<dyn Backend>, String> {
    match mode {
        Mode::Headless => Ok(Box::new(Headless::start()?)),
        Mode::Window => Ok(Box::new(Window::find()?)),
        #[cfg(target_os = "android")]
        Mode::WindowLocal => Ok(Box::new(WindowLocal::find()?)),
        Mode::Auto => match Window::find() {
            Ok(window) => Ok(Box::new(window)),
            Err(why) => {
                eprintln!("concat mcp: working headless: {why}");
                Ok(Box::new(Headless::start()?))
            }
        },
    }
}

/// Where the window's server is, and its token: the environment's if it
/// names them, the window's listing otherwise.
fn locate() -> Result<(SocketAddr, String), String> {
    let address = std::env::var("CONCAT_API_ADDRESS")
        .ok()
        .filter(|text| !text.is_empty());
    let token = std::env::var("CONCAT_API_TOKEN")
        .ok()
        .filter(|text| !text.is_empty());
    if let (Some(address), Some(token)) = (&address, &token) {
        let address = address.parse().map_err(|_| {
            format!("CONCAT_API_ADDRESS {address:?} is not an address like 127.0.0.1:7420")
        })?;
        return Ok((address, token.clone()));
    }
    if address.is_some() != token.is_some() {
        return Err("CONCAT_API_ADDRESS and CONCAT_API_TOKEN go together".to_owned());
    }
    let dirs = AppDirs::locate()?;
    let path = discovery::path_in(&dirs.config);
    match discovery::read(&path)? {
        Some(listing) => Ok((listing.address, listing.token)),
        None => Err(
            "the Concat window is not serving the API: open Concat, then Settings > Automation, and turn on Scripts and apps"
                .to_owned(),
        ),
    }
}

/// The window's server, over a connection that is reopened once if it
/// drops - the window restarted its server, say.
pub struct Window {
    client: Client,
}

impl Window {
    /// Connects to the window's server.
    pub fn find() -> Result<Window, String> {
        let (address, token) = locate()?;
        let client = Client::connect(address, &token, PATIENCE)
            .map_err(|error| format!("{error} (if the window has closed, its listing is stale)"))?;
        Ok(Window { client })
    }

    /// Connects to the server at `address` with `token`.
    pub fn connect(address: SocketAddr, token: &str) -> Result<Window, String> {
        Ok(Window {
            client: Client::connect(address, token, PATIENCE)?,
        })
    }
}

impl Backend for Window {
    fn call(&mut self, request: Request) -> Result<Value, ApiError> {
        match self.client.call(&request) {
            Err(error)
                if error
                    .message
                    .starts_with("the connection to the Concat server") =>
            {
                let again = Window::find().map_err(|why| {
                    ApiError::failed(format!(
                        "{} - and reconnecting failed: {why}",
                        error.message
                    ))
                })?;
                *self = again;
                self.client.call(&request)
            }
            outcome => outcome,
        }
    }

    fn describe(&self) -> Value {
        serde_json::json!({ "mode": "window", "address": self.client.address().to_string() })
    }
}
/// The window's server via Android LocalSocket (abstract namespace or filesystem).
#[cfg(target_os = "android")]
pub struct WindowLocal {
    client: ClientLocal,
}

#[cfg(target_os = "android")]
impl WindowLocal {
    /// Connects to the window's server via LocalSocket.
    pub fn find() -> Result<WindowLocal, String> {
        let (name, token) = locate_local()?;
        let client = ClientLocal::connect(&name, &token, PATIENCE)?;
        Ok(WindowLocal { client })
    }
}

#[cfg(target_os = "android")]
impl Backend for WindowLocal {
    fn call(&mut self, request: Request) -> Result<Value, ApiError> {
        match self.client.call(&request) {
            Err(error)
                if error
                    .message
                    .starts_with("the connection to the Concat server") =>
            {
                let again = WindowLocal::find().map_err(|why| {
                    ApiError::failed(format!(
                        "{} - and reconnecting failed: {why}",
                        error.message
                    ))
                })?;
                *self = again;
                self.client.call(&request)
            }
            outcome => outcome,
        }
    }

    fn describe(&self) -> Value {
        serde_json::json!({ "mode": "window-local", "socket": self.client.socket_name() })
    }
}

/// A JSON-RPC client over Android LocalSocket.
#[cfg(target_os = "android")]
struct ClientLocal {
    reader: std::io::BufReader<socket2::Socket>,
    writer: socket2::Socket,
    next_id: u64,
    socket_name: String,
}

#[cfg(target_os = "android")]
impl ClientLocal {
    /// Connects to the LocalSocket `name` and presents `token`.
    pub fn connect(name: &str, token: &str, patience: Duration) -> Result<ClientLocal, String> {
        use socket2::{Domain, Protocol, Socket, Type};

        let socket = Socket::new(Domain::UNIX, Type::STREAM, Some(Protocol::from(0)))
            .map_err(|e| format!("could not open the LocalSocket {name}: {e}"))?;
        // Bounds the handshake below, and cleared once the connection is
        // in use, where a call may take as long as its method does.
        socket
            .set_read_timeout(Some(patience))
            .map_err(|e| format!("could not set the LocalSocket timeout: {e}"))?;

        // Connect to the abstract namespace or to a filesystem path: a name
        // that begins with `@` is the abstract namespace, whose `@` becomes
        // the leading NUL byte that marks the address.
        let addr = if let Some(abstract_name) = name.strip_prefix('@') {
            // SAFETY: `try_init` offers a zeroed storage to fill; the bytes
            // written are the `sockaddr_un` family, the leading NUL and the
            // abstract name, and the length set is exactly what was written,
            // as the call asks of its closure.
            let (_, addr) = unsafe {
                socket2::SockAddr::try_init(|storage, len| {
                    let addr = &mut *storage.cast::<libc::sockaddr_un>();
                    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
                    let bytes = abstract_name.as_bytes();
                    let held = bytes.len().min(addr.sun_path.len() - 1);
                    std::ptr::copy_nonoverlapping(
                        bytes.as_ptr(),
                        addr.sun_path[1..].as_mut_ptr().cast::<u8>(),
                        held,
                    );
                    *len = (std::mem::size_of::<libc::sa_family_t>() + 1 + held) as libc::socklen_t;
                    Ok(())
                })
            }
            .map_err(|e| format!("failed to build the LocalSocket address for {name}: {e}"))?;
            addr
        } else {
            socket2::SockAddr::unix(name)
                .map_err(|e| format!("failed to build the LocalSocket address for {name}: {e}"))?
        };
        socket
            .connect(&addr)
            .map_err(|e| format!("failed to connect to LocalSocket {name}: {e}"))?;

        let reader = socket
            .try_clone()
            .map_err(|e| format!("could not clone the LocalSocket {name}: {e}"))?;
        let mut client = ClientLocal {
            reader: std::io::BufReader::new(reader),
            writer: socket,
            next_id: 1,
            socket_name: name.to_owned(),
        };

        // Present the token, the way the connection to a TCP server does.
        client
            .exchange("auth", serde_json::json!({ "token": token }))
            .map_err(|error| format!("could not reach the Concat server at {name}: {error}"))?;
        client
            .reader
            .get_ref()
            .set_read_timeout(None)
            .map_err(|e| format!("could not clear the LocalSocket timeout: {e}"))?;
        Ok(client)
    }

    pub fn socket_name(&self) -> &str {
        &self.socket_name
    }

    /// Runs one request and returns its result, as the JSON the server
    /// sent.
    pub fn call(&mut self, request: &Request) -> Result<Value, ApiError> {
        let Value::Object(mut fields) = serde_json::to_value(request)
            .map_err(|error| ApiError::invalid(format!("not a request: {error}")))?
        else {
            return Err(ApiError::invalid("a request is an object"));
        };
        let method = match fields.remove("method") {
            Some(Value::String(method)) => method,
            _ => return Err(ApiError::invalid("a request names its method")),
        };
        self.exchange(&method, Value::Object(fields))
    }

    /// Sends one call and reads lines until its reply.
    fn exchange(&mut self, method: &str, params: Value) -> Result<Value, ApiError> {
        use concat_api::ErrorCode;
        use serde_json::json;

        let id = self.next_id;
        self.next_id += 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let gone = |error: &dyn std::fmt::Display| {
            ApiError::failed(format!(
                "the connection to the Concat server failed: {error}"
            ))
        };
        writeln!(self.writer, "{line}").map_err(|error| gone(&error))?;
        self.writer.flush().map_err(|error| gone(&error))?;
        loop {
            let mut reply = String::new();
            let read = std::io::Read::take(&mut self.reader, MAX_REPLY)
                .read_line(&mut reply)
                .map_err(|error| gone(&error))?;
            if read == 0 {
                return Err(gone(&"the server hung up"));
            }
            let Ok(Value::Object(mut message)) = serde_json::from_str::<Value>(&reply) else {
                return Err(gone(&"the server sent something that is not JSON-RPC"));
            };
            // An event, or the refusal of a connection the server never let
            // in (null id): the latter is the answer.
            let answered = match message.get("id") {
                Some(Value::Number(number)) => number.as_u64() == Some(id),
                Some(Value::Null) => message.contains_key("error"),
                _ => false,
            };
            if !answered {
                continue;
            }
            if let Some(result) = message.remove("result") {
                return Ok(result);
            }
            let error = message.remove("error").unwrap_or(Value::Null);
            let code = error
                .pointer("/data/code")
                .cloned()
                .and_then(|code| serde_json::from_value::<ErrorCode>(code).ok())
                .unwrap_or(ErrorCode::Failed);
            let text = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the server refused the call");
            return Err(ApiError::new(code, text));
        }
    }
}

/// Locate the window's server via LocalSocket: environment or discovery file.
#[cfg(target_os = "android")]
fn locate_local() -> Result<(String, String), String> {
    let name = std::env::var("CONCAT_API_LOCAL_SOCKET")
        .ok()
        .filter(|text| !text.is_empty());
    let token = std::env::var("CONCAT_API_TOKEN")
        .ok()
        .filter(|text| !text.is_empty());

    if let (Some(name), Some(token)) = (&name, &token) {
        return Ok((name.clone(), token.clone()));
    }

    if name.is_some() != token.is_some() {
        return Err("CONCAT_API_LOCAL_SOCKET and CONCAT_API_TOKEN go together".to_owned());
    }

    // Fall back to discovery file
    let dirs = AppDirs::locate()?;
    let path = discovery::path_in(&dirs.config);
    match discovery::read(&path)? {
        Some(listing) => match listing.local_socket {
            Some(socket_name) => Ok((socket_name, listing.token)),
            None => Err("the window's server is not serving the API via LocalSocket".to_owned()),
        },
        None => Err(
            "the Concat window is not serving the API via LocalSocket: enable it in Settings > Automation"
                .to_owned(),
        ),
    }
}

/// An API in this process.
pub struct Headless {
    api: Api,
    stopper: Stopper,
}

impl Headless {
    /// An API over this machine's app directories, writing under the
    /// roots `CONCAT_MCP_ROOTS` names (the platform's path-list syntax),
    /// or the user's home.
    pub fn start() -> Result<Headless, String> {
        Ok(Headless::with_api(Api::new(Arc::new(|_| {}))?))
    }

    /// An API made by the caller - a test's, over a scratch folder.
    pub fn with_api(mut api: Api) -> Headless {
        let roots: Vec<PathBuf> = std::env::var_os("CONCAT_MCP_ROOTS")
            .map(|roots| std::env::split_paths(&roots).collect())
            .filter(|roots: &Vec<PathBuf>| !roots.is_empty())
            .unwrap_or_else(concat_server::home_root);
        api.add_capability("mcp");
        if api.roots().is_empty() {
            api.restrict_writes_to(roots);
        }
        let stopper = api.stopper();
        Headless { api, stopper }
    }
}

impl Backend for Headless {
    fn call(&mut self, request: Request) -> Result<Value, ApiError> {
        match self.api.dispatch(request) {
            Response::Result(reply) => serde_json::to_value(&reply)
                .map_err(|error| ApiError::failed(format!("the reply did not encode: {error}"))),
            Response::Error(error) => Err(error),
        }
    }

    fn describe(&self) -> Value {
        let roots: Vec<String> = self
            .api
            .roots()
            .iter()
            .map(|root| root.display().to_string())
            .collect();
        serde_json::json!({ "mode": "headless", "writesUnder": roots })
    }

    fn finish(&mut self) {
        self.stopper.cancel_all();
        self.api.finish();
    }
}
