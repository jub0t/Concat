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
//! The API reads and writes whatever paths it is given, so a server is a
//! door into the machine. It binds loopback unless told otherwise, and it
//! refuses to bind anything else without a token every connection must
//! present before its first call. There is no encryption: a bind off
//! loopback belongs behind something that provides it.

mod hub;
pub mod json;

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
    /// What a connection presents before its first call. Required for
    /// any address that is not loopback.
    pub token: Option<String>,
}

impl Config {
    /// Refuses a configuration that would open the machine: a TCP bind
    /// off loopback with no token.
    pub fn check(&self) -> Result<(), String> {
        for (what, address) in [("JSON-RPC", self.json), ("gRPC", self.grpc)] {
            if let Some(address) = address
                && !address.ip().is_loopback()
                && self.token.as_deref().is_none_or(str::is_empty)
            {
                return Err(format!(
                    "{what} on {address} is reachable from other machines: set a token, or bind 127.0.0.1"
                ));
            }
        }
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
    stop: Arc<AtomicBool>,
    listeners: Vec<JoinHandle<()>>,
    connections: Connections,
}

/// Every open connection's way of being closed, so a stop reaches the
/// threads blocked reading them.
pub(crate) type Connections = Arc<Mutex<Vec<Box<dyn Fn() + Send>>>>;

impl Server {
    /// Binds every address in `config` and starts serving. `make` builds
    /// the API on the hub's thread, given the sink its jobs report through.
    pub fn start(
        config: Config,
        make: impl FnOnce(EventSink) -> Result<Api, String> + Send + 'static,
    ) -> Result<Server, String> {
        config.check()?;
        let (hub, dispatcher) = Hub::start(make)?;
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Connections = Arc::new(Mutex::new(Vec::new()));
        let mut server = Server {
            hub,
            dispatcher: Some(dispatcher),
            json: None,
            socket: None,
            grpc: None,
            stop: Arc::clone(&stop),
            listeners: Vec::new(),
            connections: Arc::clone(&connections),
        };
        let token = config.token.filter(|token| !token.is_empty());

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
                token.clone(),
                Arc::clone(&stop),
                Arc::clone(&connections),
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
                token.clone(),
                Arc::clone(&stop),
                Arc::clone(&connections),
            ));
            server.socket = Some(path);
        }

        #[cfg(feature = "grpc")]
        if let Some(address) = config.grpc {
            let (bound, thread) =
                grpc::serve(address, server.hub.clone(), token, Arc::clone(&stop))?;
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
            ..Config::default()
        };
        let server =
            Server::start(config, move |events| Ok(Api::with_dirs(dirs, events))).expect("starts");
        (server, scratch)
    }

    #[test]
    fn a_bind_off_loopback_needs_a_token() {
        let open = Config {
            json: Some("0.0.0.0:0".parse().expect("an address")),
            ..Config::default()
        };
        assert!(open.check().is_err());
        let closed = Config {
            token: Some("s".to_owned()),
            ..open.clone()
        };
        assert!(closed.check().is_ok());
        let local = Config {
            json: Some("127.0.0.1:0".parse().expect("an address")),
            ..Config::default()
        };
        assert!(local.check().is_ok());
    }

    #[test]
    fn a_server_stops_cleanly_with_nobody_connected() {
        let (server, _scratch) = server(None);
        assert!(server.json_addr().is_some());
        assert_eq!(server.connections(), 0);
        server.stop();
    }
}
