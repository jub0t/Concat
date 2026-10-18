// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
    /// The window's server, or an error saying how to turn it on.
    Window,
    /// This process, whatever else is running.
    Headless,
}

impl Mode {
    /// The mode a word names: "auto", "window" or "headless".
    pub fn parse(word: &str) -> Option<Mode> {
        match word.trim().to_ascii_lowercase().as_str() {
            "" | "auto" => Some(Mode::Auto),
            "window" => Some(Mode::Window),
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
    let address = std::env::var("CONCAT_API_ADDRESS").ok().filter(|text| !text.is_empty());
    let token = std::env::var("CONCAT_API_TOKEN").ok().filter(|text| !text.is_empty());
    if let (Some(address), Some(token)) = (&address, &token) {
        let address = address
            .parse()
            .map_err(|_| format!("CONCAT_API_ADDRESS {address:?} is not an address like 127.0.0.1:7420"))?;
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
        let client = Client::connect(address, &token, PATIENCE).map_err(|error| {
            format!("{error} (if the window has closed, its listing is stale)")
        })?;
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
            Err(error) if error.message.starts_with("the connection to the Concat server") => {
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
