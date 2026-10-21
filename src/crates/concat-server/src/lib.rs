// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

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
//! to a page or a terminal. A token that is given must pass
//! [`check_token`]: twenty characters at least, and not a repeated
//! pattern. It is compared in constant time, whichever transport carries
//! it, and an address that keeps presenting wrong ones waits longer each
//! time ([`throttle`]). The API writes only under the roots the server is
//! given - the user's home unless told otherwise ([`Config::roots`]) - and
//! a frame or an export is bounded in size. The JSON transport reads a
//! line of at most [`json::MAX_LINE`] bytes, gives a caller
//! [`json::AUTH_TIMEOUT`] to present its token, holds nothing for a caller
//! that has not, seats at most [`json::MAX_CONNECTIONS`] callers at once
//! and hangs up on one that stops reading its replies. There is no
//! encryption: a bind off loopback belongs behind something that provides
//! it, and [`is_exposed`] is how a front end says so.

pub mod client;
pub mod discovery;
mod hub;
pub mod json;
pub mod throttle;
mod token;

#[cfg(feature = "grpc")]
pub mod grpc;

use std::collections::BTreeMap;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use concat_api::{Api, EventSink, Stopper};

pub use hub::{Hub, Subscriber};
pub use token::{MIN_BITS as MIN_TOKEN_BITS, MIN_CHARS as MIN_TOKEN_CHARS};

/// A fresh token: 128 bits from the operating system's randomness, as 32
/// hex digits. What a front end offers when asked for a new one.
pub fn mint_token() -> Result<String, String> {
    token::mint()
}

/// Refuses a chosen token that is short or repetitive enough to guess;
/// the sentence says why. A minted token always passes.
pub fn check_token(token: &str) -> Result<(), String> {
    token::check(token)
}

/// Whether listening on `address` lets other machines in: anything but a
/// loopback address. A front end warns when this is true; the server
/// itself allows it, with a token, as a deliberate choice.
pub fn is_exposed(address: &SocketAddr) -> bool {
    !address.ip().is_loopback()
}

/// How long a stop waits on the connection that wakes a listener.
const WAKE_PATIENCE: std::time::Duration = std::time::Duration::from_secs(2);

/// The address a local caller connects to for a listener bound to
/// `bound`: the same, except that the unspecified address is loopback.
pub fn loopback(bound: SocketAddr) -> SocketAddr {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), bound.port())
        }
        IpAddr::V6(ip) if ip.is_unspecified() => {
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), bound.port())
        }
        _ => bound,
    }
}

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
    /// to be safe to make. One that is set must pass [`check_token`].
    pub token: Option<String>,
    /// The folders the API may write under: a created project, an
    /// instantiated template, an export, a preview file. Empty means the
    /// user's home ([`home_root`]); to write anywhere, name `/`.
    pub roots: Vec<PathBuf>,
    /// Where to list the JSON-RPC address and the token while the server
    /// runs, for a program on this machine to find it - `concat --mcp`,
    /// started by an agent. See [`discovery`]; removed when the server
    /// stops. `None` lists nothing.
    pub discovery: Option<PathBuf>,
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
    /// Refuses a configuration this build or platform cannot serve, or a
    /// token too weak to stand in front of it.
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
        if let Some(token) = self.token.as_deref().filter(|token| !token.is_empty()) {
            check_token(token)?;
        }
        Ok(())
    }
}

/// A running server: its listeners and the hub behind them.
pub struct Server {
    hub: Hub,
    dispatcher: Option<JoinHandle<()>>,
    stopper: Stopper,
    json: Option<SocketAddr>,
    socket: Option<PathBuf>,
    grpc: Option<SocketAddr>,
    token: String,
    roots: Vec<PathBuf>,
    stop: Arc<AtomicBool>,
    listeners: Vec<JoinHandle<()>>,
    connections: Connections,
    seats: Arc<AtomicUsize>,
    pending: Arc<AtomicUsize>,
    /// The listing written for local callers, taken away at the stop.
    listing: Option<PathBuf>,
}

/// Every connection that is in, by number, with its way of being closed,
/// so a stop reaches the threads blocked reading them. A connection takes
/// itself out when it ends. Called with `true`, the closing first writes
/// out what the connection has queued - the last event of a job the stop
/// ended - and waits for that, at most the write timeout.
pub(crate) type Connections = Arc<Mutex<BTreeMap<u64, Box<dyn Fn(bool) + Send>>>>;

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
        let (hub, dispatcher, stopper) = Hub::start(move |events| {
            let mut api = make(events)?;
            for transport in transports {
                api.add_capability(transport);
            }
            api.restrict_writes_to(confined);
            Ok(api)
        })?;
        let stop = Arc::new(AtomicBool::new(false));
        let connections: Connections = Arc::new(Mutex::new(BTreeMap::new()));
        let throttle = Arc::new(throttle::Throttle::default());
        let door = json::Door {
            hub: hub.clone(),
            token: token.clone(),
            stop: Arc::clone(&stop),
            connections: Arc::clone(&connections),
            seats: Arc::new(AtomicUsize::new(0)),
            pending: Arc::new(AtomicUsize::new(0)),
            throttle: Arc::clone(&throttle),
            next_id: Arc::new(AtomicU64::new(1)),
        };
        // Built before the first bind, so a bind that fails drops a server
        // that shuts down what it had already started.
        let mut server = Server {
            hub,
            dispatcher: Some(dispatcher),
            stopper,
            json: None,
            socket: None,
            grpc: None,
            token,
            roots,
            stop: Arc::clone(&stop),
            listeners: Vec::new(),
            connections: Arc::clone(&connections),
            seats: Arc::clone(&door.seats),
            pending: Arc::clone(&door.pending),
            listing: None,
        };
        let could_not = |what: &str, error: &dyn std::fmt::Display| {
            format!("could not listen on {what}: {error}")
        };

        if let Some(address) = config.json {
            let listener = TcpListener::bind(address)
                .map_err(|error| could_not(&address.to_string(), &error))?;
            server.json = Some(
                listener
                    .local_addr()
                    .map_err(|error| could_not(&address.to_string(), &error))?,
            );
            server.listeners.push(
                json::serve_tcp(listener, door.clone())
                    .map_err(|error| could_not(&address.to_string(), &error))?,
            );
        }

        #[cfg(unix)]
        if let Some(path) = config.socket {
            let shown = path.display().to_string();
            let _ = std::fs::remove_file(&path);
            let listener = std::os::unix::net::UnixListener::bind(&path)
                .map_err(|error| could_not(&shown, &error))?;
            server.socket = Some(path);
            server.listeners.push(
                json::serve_unix(listener, door.clone())
                    .map_err(|error| could_not(&shown, &error))?,
            );
        }

        #[cfg(feature = "grpc")]
        if let Some(address) = config.grpc {
            let (bound, thread) = grpc::serve(
                address,
                server.hub.clone(),
                server.token.clone(),
                Arc::clone(&stop),
                Arc::clone(&throttle),
            )?;
            server.grpc = Some(bound);
            server.listeners.push(thread);
        }
        let _ = throttle;

        // Last, so nothing is listed that is not yet listening. A listing
        // that cannot be written costs the agent a way in, not the server.
        if let (Some(path), Some(address)) = (config.discovery, server.json)
            && discovery::write(&path, &discovery::Listing::new(address, &server.token)).is_ok()
        {
            server.listing = Some(path);
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

    /// How many JSON callers are in: connected, with the token presented.
    pub fn connections(&self) -> usize {
        self.seats.load(Ordering::SeqCst)
    }

    /// How many JSON connections are waiting to present their token.
    pub fn pending(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// How many connections a stop would have to close.
    #[cfg(test)]
    pub(crate) fn registered(&self) -> usize {
        self.connections.lock().map(|list| list.len()).unwrap_or(0)
    }

    /// Stops the server and returns once everything it started has ended:
    /// the listeners and connections closed, the API's own jobs cancelled
    /// and their threads finished, the API's thread gone.
    ///
    /// The jobs end before the callers are hung up, so a caller whose
    /// export was cancelled hears `export.failed` for it rather than a
    /// connection that just ends after the last progress (#431).
    pub fn stop(mut self) {
        self.close_listeners();
        self.stopper.cancel_all();
        self.hub.close();
        self.finish();
        self.hang_up(true);
    }

    /// Stops the server without waiting for the API's thread: when this
    /// returns the listeners are closed - the address may be bound again
    /// at once - every connection is hung up and the API's jobs are told
    /// to stop; the call in flight, if one is, and the jobs' last frame
    /// are waited for on a thread of their own. For a window, whose event
    /// loop must not wait on a render.
    pub fn stop_detached(mut self) {
        self.close_doors();
        let dispatcher = self.dispatcher.take();
        let _ = std::thread::Builder::new()
            .name("concat server stop".to_owned())
            .spawn(move || {
                if let Some(dispatcher) = dispatcher {
                    let _ = dispatcher.join();
                }
            });
    }

    /// Everything a stop does that cannot keep the caller waiting long:
    /// the listeners joined, the connections hung up, the jobs cancelled
    /// and the queue closed.
    fn close_doors(&mut self) {
        self.close_listeners();
        self.hang_up(false);
        // Before the queue closes: the API's thread waits for its jobs as
        // it ends, and a job told to stop ends at its next frame instead
        // of its last.
        self.stopper.cancel_all();
        self.hub.close();
    }

    /// Takes the server off the list and stops every listener, so nobody
    /// new comes in; who is in already stays connected.
    fn close_listeners(&mut self) {
        // Unlisted first, so nobody new is sent to a door that is closing.
        if let Some(path) = self.listing.take() {
            discovery::remove(&path);
        }
        self.stop.store(true, Ordering::SeqCst);
        // A blocked accept wakes for a connection; make one - to loopback
        // when the bind was to every interface, which Windows refuses to
        // connect to and which would leave the join below waiting for ever.
        if let Some(address) = self.json {
            let _ = TcpStream::connect_timeout(&loopback(address), WAKE_PATIENCE);
        }
        #[cfg(unix)]
        if let Some(path) = &self.socket {
            let _ = std::os::unix::net::UnixStream::connect(path);
        }
        #[cfg(feature = "grpc")]
        if let Some(address) = self.grpc {
            let _ = TcpStream::connect_timeout(&loopback(address), WAKE_PATIENCE);
        }
        for thread in self.listeners.drain(..) {
            let _ = thread.join();
        }
        #[cfg(unix)]
        if let Some(path) = self.socket.take() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Closes every connection. With `drain`, each writes out what it has
    /// queued first; see [`Connections`].
    fn hang_up(&mut self, drain: bool) {
        // Taken out of the lock before any is closed: a connection that
        // ends takes itself out of the same map.
        let connections = match self.connections.lock() {
            Ok(mut connections) => std::mem::take(&mut *connections),
            Err(_) => return,
        };
        for (_, close) in connections {
            close(drain);
        }
    }

    fn finish(&mut self) {
        if let Some(dispatcher) = self.dispatcher.take() {
            let _ = dispatcher.join();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if self.dispatcher.is_some() {
            self.close_doors();
            self.finish();
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use concat_api::AppDirs;

    /// A token strong enough to be accepted, for tests that choose one.
    pub(crate) const TOKEN: &str = "a test token, long and varied 7f3Q";

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
        let (given, _scratch) = server(Some(TOKEN));
        assert_eq!(given.token(), TOKEN);
        let (minted, _scratch) = server(None);
        assert_eq!(minted.token().len(), 32);
        assert!(minted.token().bytes().all(|byte| byte.is_ascii_hexdigit()));
        let (empty, _scratch) = server(Some(""));
        assert_eq!(empty.token().len(), 32, "an empty token is no token");
        assert_ne!(empty.token(), minted.token());
    }

    #[test]
    fn a_weak_token_is_refused_before_anything_listens() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        let config = Config {
            json: Some("127.0.0.1:0".parse().expect("an address")),
            token: Some("open sesame".to_owned()),
            ..Config::default()
        };
        let refused = Server::start(config, move |events| Ok(Api::with_dirs(dirs, events)));
        let Err(why) = refused else {
            panic!("a weak token was taken");
        };
        assert!(why.contains("at least"), "{why}");
    }

    #[test]
    fn exposure_is_anything_off_loopback() {
        let exposed = |text: &str| is_exposed(&text.parse().expect("an address"));
        assert!(!exposed("127.0.0.1:7420"));
        assert!(!exposed("[::1]:7420"));
        assert!(exposed("0.0.0.0:7420"));
        assert!(exposed("[::]:7420"));
        assert!(exposed("192.168.1.20:7420"));
    }

    #[test]
    fn a_server_stops_cleanly_with_nobody_connected() {
        let (server, _scratch) = server(None);
        assert!(server.json_addr().is_some());
        assert_eq!(server.connections(), 0);
        server.stop();
    }

    #[test]
    fn a_server_on_every_interface_stops_too() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        let config = Config {
            json: Some("0.0.0.0:0".parse().expect("an address")),
            roots: vec![scratch.path().to_path_buf()],
            ..Config::default()
        };
        let server =
            Server::start(config, move |events| Ok(Api::with_dirs(dirs, events))).expect("starts");
        let bound = server.json_addr().expect("listening");
        assert!(is_exposed(&bound));
        assert!(loopback(bound).ip().is_loopback());
        server.stop();
    }

    #[test]
    fn a_server_lists_itself_while_it_runs() {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        let path = discovery::path_in(&dirs.config);
        let config = Config {
            json: Some("127.0.0.1:0".parse().expect("an address")),
            roots: vec![scratch.path().to_path_buf()],
            discovery: Some(path.clone()),
            ..Config::default()
        };
        let server =
            Server::start(config, move |events| Ok(Api::with_dirs(dirs, events))).expect("starts");
        let listing = discovery::read(&path).expect("reads").expect("listed");
        assert_eq!(Some(listing.address), server.json_addr());
        assert_eq!(listing.token, server.token());
        server.stop_detached();
        assert!(!path.exists(), "unlisted at the stop");
    }

    #[test]
    fn a_detached_stop_frees_the_address_at_once() {
        let (server, _scratch) = server(None);
        let address = server.json_addr().expect("listening");
        server.stop_detached();
        let again = TcpListener::bind(address);
        assert!(again.is_ok(), "{again:?}");
    }
}
