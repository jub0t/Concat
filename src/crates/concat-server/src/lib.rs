// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Concat API on a socket, for a caller that is another process: a
//! script, an editor plugin, a service on the same machine.
//!
//! One [`Hub`] owns the one [`Api`] on a thread of its own and serialises
//! every caller through it, the way the window's event loop does; the
//! transports are threads that read a call, hand it to the hub, and write
//! the response back. Two transports speak the same API:
//!
//! - **JSON-RPC lines** over TCP or a Unix socket: the stdin transport's
//!   protocol, one JSON object a line, so what works in a pipe works on a
//!   socket unchanged. [`json`] is the whole of it.
//! - **gRPC** over HTTP/2, behind the `grpc` feature: the same methods and
//!   the same JSON payloads inside a thin protobuf envelope, for a caller
//!   that wants generated clients and a streamed reply. [`grpc`] says how.
//!
//! Events - an export's progress, how it ended - go to every connected
//! caller; each names its job and its project, so a caller keeps the ones
//! it asked for.
//!
//! A server is a door into the machine, so the door is narrow. Every
//! connection presents a token before its first call, loopback included:
//! another user's process on the same machine reaches 127.0.0.1 as
//! easily as this one does. A server given no token mints one, 128 bits
//! from the operating system's randomness, that only the process that
//! started it knows; [`Server::token`] is how that process passes it on,
//! to a page or a terminal. The token is compared in constant time,
//! whichever transport carries it. The API writes only under the roots
//! the server is given - the user's home unless told otherwise
//! ([`Config::roots`]) - and a frame or an export is bounded in size.
//! The JSON transport reads a line of at most [`json::MAX_LINE`] bytes,
//! gives a caller [`json::AUTH_TIMEOUT`] to present its token, seats at
//! most [`json::MAX_CONNECTIONS`] callers at once and hangs up on one
//! that stops reading its replies. There is no encryption: a bind off
//! loopback belongs behind something that provides it.

mod hub;
pub mod json;
mod token;

#[cfg(feature = "grpc")]
pub mod grpc;

use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use concat_api::{Api, EventSink};

pub use hub::{Hub, Subscriber};

/// Where to listen, and who may connect.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// A TCP address for JSON-RPC lines.
    pub json: Option<SocketAddr>,
    /// A Unix socket path for JSON-RPC lines. Unix only; a file already
    /// there is replaced.
    pub socket: Option<PathBuf>,
    /// A TCP address for gRPC. Needs the `grpc` feature.
    pub grpc: Option<SocketAddr>,
    /// What every connection presents before its first call. `None` or
    /// empty means the server mints one at start, which only the process
    /// that started it can learn, through [`Server::token`]; that is the
    /// point, and it is why a bind off loopback needs no token set here
    /// to be safe to make.
    pub token: Option<String>,
    /// The folders the API may write under: a created project, an
    /// instantiated template, an export, a preview file. Empty means the
    /// user's home ([`home_root`]); to write anywhere, name `/`.
    pub roots: Vec<PathBuf>,
}

/// Where the API writes when told nothing else: the user's home folder,
/// or nowhere when the platform has none to name.
pub fn home_root() -> Vec<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(|home| vec![PathBuf::from(home)])
        .unwrap_or_default()
}

impl Config {
    /// Refuses a configuration this build or platform cannot serve.
    pub fn check(&self) -> Result<(), String> {
        #[cfg(not(unix))]
        if self.socket.is_some() {
            return Err("a Unix socket needs a Unix".to_owned());
        }
        #[cfg(not(feature = "grpc"))]
        if self.grpc.is_some() {
            return Err(
                "this build has no gRPC: build concat-server with --features grpc".to_owned(),
            );
        }
        Ok(())
    }
}

/// A running server: its listeners and the hub behind them.
pub struct Server {
    hub: Hub,
    dispatcher: Option<JoinHandle<()>>,
    json: Option<SocketAddr>,
    socket: Option<PathBuf>,
    grpc: Option<SocketAddr>,
    token: String,
    roots: Vec<PathBuf>,
    stop: Arc<AtomicBool>,
    listeners: Vec<JoinHandle<()>>,
    connections: Connections,
}

/// Every open connection's way of being closed, so a stop reaches the
/// threads blocked reading them.
pub(crate) type Connections = Arc<Mutex<Vec<Box<dyn Fn() + Send>>>>;

impl Server {
    /// Binds every address in `config` and starts serving. `make` builds
    /// the API on the hub's thread, given the sink its jobs report through;
    /// the transports listening are added to what its `version` reports.
    /// With no token in `config`, one is minted; [`Server::token`] is it.
    pub fn start(
        config: Config,
        make: impl FnOnce(EventSink) -> Result<Api, String> + Send + 'static,
    ) -> Result<Server, String> {
        config.check()?;
        let token = match config.token.filter(|token| !token.is_empty()) {
            Some(token) => token,
            None => token::mint()?,
        };
        let transports: Vec<&str> = [
            config.json.map(|_| "json-rpc"),
            config.socket.as_ref().map(|_| "unix-socket"),
            config.grpc.map(|_| "grpc"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let roots = if config.roots.is_empty() {
            home_root()
        } else {
            config.roots.clone()
        };
        let confined = roots.clone();
        let (hub, dispatcher) = Hub::start(move |events| {
            let mut api = make(events)?;
            for transport in transports {
                api.add_capability(transport);
            }
            api.restrict_writes_to(confined);
            Ok(api)
        })?;
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Connections = Arc::new(Mutex::new(Vec::new()));
        let seats = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut server = Server {
            hub,
            dispatcher: Some(dispatcher),
            json: None,
            socket: None,
            grpc: None,
            token,
            roots,
            stop: Arc::clone(&stop),
            listeners: Vec::new(),
            connections: Arc::clone(&connections),
        };

        if let Some(address) = config.json {
            let listener = TcpListener::bind(address)
                .map_err(|error| format!("could not listen on {address}: {error}"))?;
            server.json = Some(
                listener
                    .local_addr()
                    .map_err(|error| format!("could not listen on {address}: {error}"))?,
            );
            server.listeners.push(json::serve_tcp(
                listener,
                server.hub.clone(),
                server.token.clone(),
                Arc::clone(&stop),
                Arc::clone(&connections),
                Arc::clone(&seats),
            ));
        }

        #[cfg(unix)]
        if let Some(path) = config.socket {
            let _ = std::fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path)
                .map_err(|error| format!("could not listen on {}: {error}", path.display()))?;
            server.listeners.push(json::serve_unix(
                listener,
                server.hub.clone(),
                server.token.clone(),
                Arc::clone(&stop),
                Arc::clone(&connections),
                Arc::clone(&seats),
            ));
            server.socket = Some(path);
        }

        #[cfg(feature = "grpc")]
        if let Some(address) = config.grpc {
            let (bound, thread) = grpc::serve(
                address,
                server.hub.clone(),
                server.token.clone(),
                Arc::clone(&stop),
            )?;
            server.grpc = Some(bound);
            server.listeners.push(thread);
        }

        Ok(server)
    }

    /// The hub, for an embedder that calls the API from the same process.
    pub fn hub(&self) -> &Hub {
        &self.hub
    }

    /// The JSON-RPC address bound, port resolved.
    pub fn json_addr(&self) -> Option<SocketAddr> {
        self.json
    }

    /// The Unix socket path listened on.
    pub fn socket_path(&self) -> Option<&PathBuf> {
        self.socket.as_ref()
    }

    /// The gRPC address bound, port resolved.
    pub fn grpc_addr(&self) -> Option<SocketAddr> {
        self.grpc
    }

    /// The token every connection presents: the one configured, or the
    /// one minted because none was. Whoever started the server shows or
    /// prints this so a caller can present it.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// The folders the API writes under; empty when it may write anywhere.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// How many callers are connected.
    pub fn connections(&self) -> usize {
        self.hub.subscribers()
    }

    /// Closes every listener and connection, waits for the jobs still
    /// running, and returns when the API's thread has ended.
    pub fn stop(mut self) {
        self.shut();
    }

    fn shut(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // A blocked accept wakes for a connection; make one.
        if let Some(address) = self.json {
            let _ = TcpStream::connect(address);
        }
        #[cfg(unix)]
        if let Some(path) = &self.socket {
            let _ = std::os::unix::net::UnixStream::connect(path);
        }
        #[cfg(feature = "grpc")]
        if let Some(address) = self.grpc {
            let _ = TcpStream::connect(address);
        }
        for thread in self.listeners.drain(..) {
            let _ = thread.join();
        }
        if let Ok(mut connections) = self.connections.lock() {
            for close in connections.drain(..) {
                close();
            }
        }
        #[cfg(unix)]
        if let Some(path) = self.socket.take() {
            let _ = std::fs::remove_file(path);
        }
        self.hub.close();
        if let Some(dispatcher) = self.dispatcher.take() {
            let _ = dispatcher.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.dispatcher.is_some() {
            self.shut();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use concat_api::AppDirs;

    /// A server over a scratch home, on loopback, on any free port.
    pub(crate) fn server(token: Option<&str>) -> (Server, tempfile::TempDir) {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        let config = Config {
            json: Some("127.0.0.1:0".parse().expect("an address")),
            token: token.map(str::to_owned),
            roots: vec![scratch.path().to_path_buf()],
            ..Config::default()
        };
        let server =
            Server::start(config, move |events| Ok(Api::with_dirs(dirs, events))).expect("starts");
        (server, scratch)
    }

    #[test]
    fn a_server_keeps_the_token_it_is_given_and_mints_one_otherwise() {
        let (given, _scratch) = server(Some("open sesame"));
        assert_eq!(given.token(), "open sesame");
        let (minted, _scratch) = server(None);
        assert_eq!(minted.token().len(), 32);
        assert!(minted.token().bytes().all(|byte| byte.is_ascii_hexdigit()));
        let (empty, _scratch) = server(Some(""));
        assert_eq!(empty.token().len(), 32, "an empty token is no token");
        assert_ne!(empty.token(), minted.token());
    }

    #[test]
    fn a_server_stops_cleanly_with_nobody_connected() {
        let (server, _scratch) = server(None);
        assert!(server.json_addr().is_some());
        assert_eq!(server.connections(), 0);
        server.stop();
    }
}
