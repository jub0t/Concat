// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! JSON-RPC lines on a socket: the stdin transport, for several callers at
//! once.
//!
//! A connection's first line is `{"jsonrpc": "2.0", "id": 1, "method":
//! "auth", "params": {"token": "..."}}`, with the server's token. Anything
//! else first, or the wrong token, is answered with an `unauthorized` error
//! and the connection is closed. `auth` is the transport's word, not the
//! API's: it never reaches the hub. The wire after it is exactly what
//! `concat-cli api` reads and writes; see `concat_api::rpc`.
//!
//! A caller that has not presented its token yet holds as little as it
//! can: one thread and the socket it arrived on, for at most
//! [`AUTH_TIMEOUT`] all told - however slowly it dribbles its line in -
//! and at most [`MAX_PENDING`] of them wait at once. Only a caller that is
//! in takes one of the [`MAX_CONNECTIONS`] seats, gets the handles a stop
//! needs to reach it, and is subscribed to events; all three are given
//! back the moment it hangs up, not when the server stops. An address
//! that keeps presenting wrong tokens waits longer each time (see
//! [`crate::throttle`]). The accept loop backs off when accepting fails -
//! out of file descriptors, say - instead of spinning on the error.
//!
//! Once in, a connection is two threads: one reads lines and answers each
//! through the [`Hub`], one writes whatever is put in the connection's
//! outbox - responses and events alike - so a job's event from another
//! thread never interleaves with a response half-written. The outbox is
//! bounded and a write gives up after [`WRITE_TIMEOUT`], so a caller that
//! stops reading is hung up on rather than holding anything up.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use concat_api::rpc::{Call, Id, Message};
use concat_api::{ApiError, Done, ErrorCode, Reply, Response};
use serde_json::Value;

use crate::throttle::{Peer, Throttle};
use crate::{Connections, Hub, Subscriber, token};

/// The longest line a caller may send once it is in: a document-sized
/// edit fits, a buffer grown to whatever arrives does not.
pub const MAX_LINE: usize = 4 * 1024 * 1024;
/// The longest first line: the `auth` call is a few dozen bytes.
pub const MAX_AUTH_LINE: usize = 4 * 1024;
/// How long a caller has, in all, to present its token before the
/// connection is closed.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(5);
/// How many callers may be in at once; the next to present its token is
/// refused with `busy` and closed.
pub const MAX_CONNECTIONS: usize = 64;
/// How many connections may be waiting to present their token at once;
/// one more is refused with `busy` as it arrives.
pub const MAX_PENDING: usize = 32;
/// How long one write to a caller may take before the caller is taken to
/// have stopped reading and is hung up on.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How many lines may wait for a caller that is not reading them before
/// the connection is closed rather than the queue grown.
const OUTBOX_LINES: usize = 256;
/// The accept loop's first and longest pause after a failed accept.
const BACKOFF_FIRST: Duration = Duration::from_millis(10);
const BACKOFF_LONGEST: Duration = Duration::from_secs(1);

/// What the JSON transport needs of a stream: TCP and a Unix socket alike.
pub(crate) trait Socket: Read + Write + Send + Sized + 'static {
    fn duplicate(&self) -> std::io::Result<Self>;
    fn hang_up(&self);
    fn read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    fn write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()>;
    fn peer(&self) -> Peer;
}

impl Socket for TcpStream {
    fn duplicate(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn hang_up(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
    fn read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)
    }
    fn write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_write_timeout(timeout)
    }
    fn peer(&self) -> Peer {
        self.peer_addr()
            .map(|address| Peer::Ip(address.ip()))
            .unwrap_or(Peer::Local)
    }
}

#[cfg(unix)]
impl Socket for std::os::unix::net::UnixStream {
    fn duplicate(&self) -> std::io::Result<Self> {
        self.try_clone()
    }
    fn hang_up(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
    fn read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_read_timeout(timeout)
    }
    fn write_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.set_write_timeout(timeout)
    }
    fn peer(&self) -> Peer {
        Peer::Local
    }
}

/// What every connection of one server shares.
#[derive(Clone)]
pub(crate) struct Door {
    pub(crate) hub: Hub,
    pub(crate) token: String,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) connections: Connections,
    pub(crate) seats: Arc<AtomicUsize>,
    pub(crate) pending: Arc<AtomicUsize>,
    pub(crate) throttle: Arc<Throttle>,
    pub(crate) next_id: Arc<AtomicU64>,
}

/// One of a counted number of places, given back when dropped.
struct Place(Arc<AtomicUsize>);

impl Place {
    fn take(count: &Arc<AtomicUsize>, cap: usize) -> Option<Place> {
        let mut taken = count.load(Ordering::SeqCst);
        loop {
            if taken >= cap {
                return None;
            }
            match count.compare_exchange(taken, taken + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Place(Arc::clone(count))),
                Err(now) => taken = now,
            }
        }
    }
}

impl Drop for Place {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A connection's entry in the server's registry of ways to close it,
/// taken out when the connection ends.
struct Registered {
    connections: Connections,
    id: u64,
}

impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.remove(&self.id);
        }
    }
}

/// Writes one refusal and lets the socket go. Best effort: the line is
/// small enough for the socket's buffer, and the write may not wait.
fn refuse(mut stream: impl Socket, id: Option<Id>, code: ErrorCode, why: String) {
    let _ = stream.write_timeout(Some(Duration::from_millis(500)));
    let line = Message::Reply {
        id,
        response: Response::Error(ApiError::new(code, why)),
    }
    .to_json();
    let _ = writeln!(stream, "{line}");
    let _ = stream.flush();
    stream.hang_up();
}

/// Accepts JSON-RPC connections on `listener` until `stop` is set.
pub(crate) fn serve_tcp(listener: TcpListener, door: Door) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("concat json accept".to_owned())
        .spawn(move || accept_loop(|| listener.accept().map(|(stream, _)| stream), door))
}

/// Accepts JSON-RPC connections on a Unix socket until `stop` is set.
#[cfg(unix)]
pub(crate) fn serve_unix(
    listener: std::os::unix::net::UnixListener,
    door: Door,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("concat socket accept".to_owned())
        .spawn(move || accept_loop(|| listener.accept().map(|(stream, _)| stream), door))
}

/// Takes connections until the server stops. A failed accept - the
/// process out of descriptors, the peer gone before it was taken - is
/// waited out, longer each time it repeats, so the loop never spins.
fn accept_loop<S: Socket>(mut accept: impl FnMut() -> std::io::Result<S>, door: Door) {
    let mut backoff = Duration::ZERO;
    loop {
        let accepted = accept();
        if door.stop.load(Ordering::SeqCst) {
            break;
        }
        match accepted {
            Ok(stream) => {
                backoff = Duration::ZERO;
                admit(stream, &door);
            }
            Err(_) => {
                backoff = (backoff * 2).clamp(BACKOFF_FIRST, BACKOFF_LONGEST);
                std::thread::sleep(backoff);
            }
        }
    }
}

/// Lets one connection wait for its token, if it may.
fn admit<S: Socket>(stream: S, door: &Door) {
    let peer = stream.peer();
    if let Some(wait) = door.throttle.wait(peer) {
        refuse(
            stream,
            None,
            ErrorCode::Unauthorized,
            format!(
                "too many wrong tokens from this address: try again in {} s",
                wait.as_secs().max(1)
            ),
        );
        return;
    }
    let Some(pending) = Place::take(&door.pending, MAX_PENDING) else {
        refuse(
            stream,
            None,
            ErrorCode::Busy,
            format!("{MAX_PENDING} connections are waiting to present a token already"),
        );
        return;
    };
    let id = door.next_id.fetch_add(1, Ordering::SeqCst);
    let door = door.clone();
    // A thread that could not be started drops the stream with the
    // closure, which closes it.
    let _ = std::thread::Builder::new()
        .name(format!("concat json {id}"))
        .spawn(move || connection(stream, door, id, peer, pending));
}

/// One caller, start to end: the handshake, then a call per line.
fn connection<S: Socket>(mut stream: S, door: Door, id: u64, peer: Peer, pending: Place) {
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let Some((first, rest)) = first_line(&mut stream, deadline) else {
        // Silent, too slow, too long, or gone: the socket closes as it
        // drops, and nothing else was ever held for it.
        return;
    };
    let (call, outcome) = handshake(&first, &door.token);
    if let Err(error) = outcome {
        door.throttle.failed(peer);
        refuse(stream, call, error.code, error.message);
        return;
    }
    door.throttle.succeeded(peer);
    drop(pending);
    let Some(_seat) = Place::take(&door.seats, MAX_CONNECTIONS) else {
        refuse(
            stream,
            call,
            ErrorCode::Busy,
            format!("{MAX_CONNECTIONS} callers are connected already"),
        );
        return;
    };
    if door.stop.load(Ordering::SeqCst) {
        return;
    }
    // In: the handles the rest needs. A stop reaches the reader through
    // the registry; the writer has its own handle, and its own timeout.
    let (Ok(writer), Ok(closer)) = (stream.duplicate(), stream.duplicate()) else {
        return;
    };
    let _registered = match door.connections.lock() {
        Ok(mut connections) => {
            connections.insert(id, Box::new(move || closer.hang_up()));
            Registered {
                connections: Arc::clone(&door.connections),
                id,
            }
        }
        Err(_) => return,
    };
    if stream.read_timeout(None).is_err() || writer.write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    let Some(outbox) = Outbox::start(writer, id) else {
        return;
    };
    outbox.send(Message::Reply {
        id: call,
        response: Response::Result(Reply::Done(Done {})),
    });

    // Events reach this caller from here on: the outbox is shared with the
    // hub's fan-out, which drops it once a send fails.
    let events = outbox.clone();
    let subscriber: Subscriber = Arc::new(move |event| events.send(Message::Event(event.clone())));
    door.hub.subscribe(Arc::clone(&subscriber));

    let mut reader = BufReader::new(std::io::Cursor::new(rest).chain(stream));
    while let Some(line) = read_line(&mut reader, MAX_LINE) {
        if line.trim().is_empty() {
            continue;
        }
        let (id, response) = match Call::parse(&line) {
            Ok(call) => (call.id, door.hub.call(call.request)),
            Err((id, error)) => (id, Response::Error(error)),
        };
        if !outbox.send(Message::Reply { id, response }) {
            break;
        }
    }
    door.hub.unsubscribe(&subscriber);
    outbox.close();
    // The caller sees EOF even where its own end is still open.
    reader.into_inner().into_inner().1.hang_up();
}

/// The first line that is not blank, read before `deadline` - a deadline
/// for the whole line, not for each read, so a caller cannot keep a
/// handshake open by sending a byte at a time. The bytes that arrived
/// after the line come back with it, for the reader that follows.
fn first_line<S: Socket>(stream: &mut S, deadline: Instant) -> Option<(String, Vec<u8>)> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        while let Some(end) = buffer.iter().position(|&byte| byte == b'\n') {
            let rest = buffer.split_off(end + 1);
            let line = String::from_utf8(std::mem::take(&mut buffer)).ok()?;
            let line = line.trim_end_matches(['\r', '\n']);
            if !line.trim().is_empty() {
                return Some((line.to_owned(), rest));
            }
            buffer = rest;
        }
        if buffer.len() > MAX_AUTH_LINE {
            return None;
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        if left.is_zero() {
            return None;
        }
        stream.read_timeout(Some(left)).ok()?;
        match stream.read(&mut chunk) {
            Ok(0) => return None,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return None,
        }
    }
}

/// One line of at most `cap` bytes, its newline taken off. `None` at the
/// end of the stream, on a read error, and for a line over the cap, which
/// ends the connection: the buffer grows to the cap and no further,
/// whatever the caller sends.
fn read_line(reader: &mut impl BufRead, cap: usize) -> Option<String> {
    let mut line = String::new();
    match std::io::Read::take(&mut *reader, cap as u64 + 1).read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) if line.len() > cap && !line.ends_with('\n') => None,
        Ok(_) => Some(line.trim_end_matches(['\r', '\n']).to_owned()),
    }
}

/// Reads the first line as the `auth` call and checks its token, in
/// constant time.
fn handshake(line: &str, token: &str) -> (Option<Id>, Result<(), ApiError>) {
    let refuse = |id: Option<Id>, why: &str| {
        (
            id,
            Err(ApiError::new(ErrorCode::Unauthorized, why.to_owned())),
        )
    };
    let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(line) else {
        return refuse(None, "the first line is the auth call");
    };
    let id = object.remove("id");
    if object.get("method").and_then(Value::as_str) != Some("auth") {
        return refuse(id, "the first line is the auth call");
    }
    let presented = object
        .get("params")
        .and_then(|params| params.get("token"))
        .or_else(|| object.get("token"))
        .and_then(Value::as_str);
    if token::matches(presented.unwrap_or_default().as_bytes(), token.as_bytes()) {
        (id, Ok(()))
    } else {
        refuse(id, "wrong token")
    }
}

/// The connection's one writer: a queue and the thread that drains it.
#[derive(Clone)]
struct Outbox {
    queue: Arc<Mutex<Option<SyncSender<Message>>>>,
    writer: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Outbox {
    /// `None` when no thread could be started for it.
    fn start<S: Socket>(mut writer: S, id: u64) -> Option<Outbox> {
        // Bounded: a caller that stops reading is hung up on at the cap,
        // not kept in memory line by line.
        let (queue, messages) = mpsc::sync_channel::<Message>(OUTBOX_LINES);
        let thread = std::thread::Builder::new()
            .name(format!("concat json {id} out"))
            .spawn(move || {
                for message in messages {
                    if writeln!(writer, "{}", message.to_json()).is_err()
                        || writer.flush().is_err()
                    {
                        // Gone, or stalled past the write timeout: hang up
                        // so the reader ends too, and drop the queue so
                        // every later send says the caller is gone.
                        writer.hang_up();
                        break;
                    }
                }
            })
            .ok()?;
        Some(Outbox {
            queue: Arc::new(Mutex::new(Some(queue))),
            writer: Arc::new(Mutex::new(Some(thread))),
        })
    }

    /// Queues one line. False once the caller is gone.
    fn send(&self, message: Message) -> bool {
        match self.queue.lock() {
            Ok(queue) => queue
                .as_ref()
                .is_some_and(|queue| queue.try_send(message).is_ok()),
            Err(_) => false,
        }
    }

    /// Ends the queue and waits for the writer to finish what is left, so
    /// the last reply is on the wire when this returns - or the write
    /// timeout has given up on it.
    fn close(&self) {
        if let Ok(mut queue) = self.queue.lock() {
            queue.take();
        }
        let thread = self.writer.lock().ok().and_then(|mut writer| writer.take());
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{TOKEN, server};
    use crate::throttle::FREE_FAILURES;

    /// A client on `server`: a line in, a line out.
    struct Client {
        reader: BufReader<TcpStream>,
        writer: TcpStream,
    }

    impl Client {
        /// A connection with the server's token presented, ready to call.
        fn connect(server: &crate::Server) -> Client {
            let mut client = Client::reach(server);
            let welcomed = client.auth(server.token());
            assert_eq!(welcomed["result"], serde_json::json!({}), "{welcomed}");
            client
        }

        /// A connection with nothing said yet.
        fn reach(server: &crate::Server) -> Client {
            let stream =
                TcpStream::connect(server.json_addr().expect("listening")).expect("connects");
            stream
                .set_read_timeout(Some(Duration::from_secs(30)))
                .expect("a timeout");
            Client {
                reader: BufReader::new(stream.try_clone().expect("clone")),
                writer: stream,
            }
        }

        fn auth(&mut self, token: &str) -> Value {
            self.ask(
                &serde_json::json!({
                    "jsonrpc": "2.0", "id": 0, "method": "auth", "params": { "token": token },
                })
                .to_string(),
            )
        }

        fn ask(&mut self, line: &str) -> Value {
            writeln!(self.writer, "{line}").expect("writes");
            self.read()
        }

        fn read(&mut self) -> Value {
            let mut reply = String::new();
            self.reader.read_line(&mut reply).expect("reads");
            serde_json::from_str(&reply).unwrap_or_else(|_| panic!("a JSON line: {reply:?}"))
        }

        /// Whether the server has hung up: EOF, or a reset.
        fn closed(&mut self) -> bool {
            let mut after = String::new();
            matches!(self.reader.read_line(&mut after), Ok(0) | Err(_))
        }
    }

    /// Waits up to five seconds for `condition`.
    fn eventually(condition: impl Fn() -> bool) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if condition() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        condition()
    }

    #[test]
    fn a_line_over_the_cap_ends_the_connection() {
        let (server, _scratch) = server(None);
        let mut client = Client::connect(&server);
        let long = "a".repeat(MAX_LINE + 2);
        // The write may fail part way once the server hangs up; that is
        // the point.
        let _ = writeln!(client.writer, "{long}");
        assert!(client.closed(), "closed without an answer");
        server.stop();
    }

    #[test]
    fn a_full_house_refuses_the_next_caller() {
        let (server, _scratch) = server(None);
        let seated: Vec<Client> = (0..MAX_CONNECTIONS)
            .map(|_| Client::connect(&server))
            .collect();
        assert_eq!(server.connections(), MAX_CONNECTIONS);
        let mut late = Client::reach(&server);
        let refused = late.auth(server.token());
        assert_eq!(refused["error"]["data"]["code"], "busy", "{refused}");
        assert!(late.closed());
        drop(seated);
        // A seat given back is taken again.
        assert!(eventually(|| server.connections() == 0), "the seats come back");
        let _back = Client::connect(&server);
        server.stop();
    }

    #[test]
    fn a_silent_connection_is_dropped_at_the_deadline_and_holds_no_seat() {
        let (server, _scratch) = server(None);
        let started = Instant::now();
        let mut silent = Client::reach(&server);
        // Waiting callers are not seated callers.
        let mut talker = Client::connect(&server);
        assert_eq!(server.connections(), 1);
        assert!(silent.closed(), "hung up on");
        let waited = started.elapsed();
        assert!(
            waited >= AUTH_TIMEOUT - Duration::from_millis(100)
                && waited < AUTH_TIMEOUT + Duration::from_secs(3),
            "after {waited:?}"
        );
        let reply = talker.ask(r#"{"jsonrpc":"2.0","id":1,"method":"version"}"#);
        assert_eq!(reply["id"], 1);
        server.stop();
    }

    #[test]
    fn a_dribbled_token_does_not_stretch_the_deadline() {
        let (server, _scratch) = server(None);
        let mut slow = Client::reach(&server);
        let started = Instant::now();
        // A byte every half second resets a per-read timeout forever; the
        // deadline is for the whole line.
        let mut hung_up = false;
        while started.elapsed() < AUTH_TIMEOUT + Duration::from_secs(3) {
            if slow.writer.write_all(b" ").is_err() {
                hung_up = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        assert!(hung_up || slow.closed(), "still open after the deadline");
        assert!(started.elapsed() < AUTH_TIMEOUT + Duration::from_secs(3));
        server.stop();
    }

    #[test]
    fn waiting_connections_are_capped_and_a_closed_one_leaves_nothing_behind() {
        let (server, _scratch) = server(None);
        let waiting: Vec<Client> = (0..MAX_PENDING).map(|_| Client::reach(&server)).collect();
        assert!(eventually(|| server.pending() == MAX_PENDING));
        let mut over = Client::reach(&server);
        let refused = over.read();
        assert_eq!(refused["error"]["data"]["code"], "busy", "{refused}");
        drop(waiting);
        // Each waiting thread sees its caller go and gives its place back.
        assert!(eventually(|| server.pending() == 0), "{}", server.pending());
        // In and out again: nothing stays registered for a stop to find.
        for _ in 0..8 {
            let client = Client::connect(&server);
            drop(client);
        }
        assert!(eventually(|| server.registered() == 0 && server.connections() == 0));
        assert_eq!(server.hub().subscribers(), 0, "nobody left subscribed");
        server.stop();
    }

    #[test]
    fn wrong_tokens_are_throttled_and_the_right_one_clears_it() {
        let (server, _scratch) = server(None);
        for _ in 0..=FREE_FAILURES {
            let mut wrong = Client::reach(&server);
            let refused = wrong.auth("not the token at all, no");
            assert_eq!(refused["error"]["data"]["code"], "unauthorized");
            assert_eq!(refused["error"]["message"], "wrong token");
        }
        // Now even the right token waits: the refusal comes as the
        // connection is taken, before a token is read.
        let mut early = Client::reach(&server);
        let refused = early.read();
        assert_eq!(refused["error"]["data"]["code"], "unauthorized");
        assert!(
            refused["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains("try again")),
            "{refused}"
        );
        std::thread::sleep(crate::throttle::FIRST_WAIT + Duration::from_millis(100));
        let mut right = Client::connect(&server);
        let reply = right.ask(r#"{"jsonrpc":"2.0","id":2,"method":"version"}"#);
        assert_eq!(reply["id"], 2);
        // A clean slate: one more wrong token is forgiven again.
        let mut slip = Client::reach(&server);
        assert_eq!(slip.auth("still not it, sorry")["error"]["message"], "wrong token");
        let _ = Client::connect(&server);
        server.stop();
    }

    #[test]
    fn a_call_over_tcp_is_answered_with_its_id() {
        let (server, _scratch) = server(None);
        let mut client = Client::connect(&server);
        let reply = client.ask(r#"{"jsonrpc":"2.0","id":41,"method":"version"}"#);
        assert_eq!(reply["id"], 41);
        assert_eq!(reply["result"]["apiVersion"], concat_api::API_VERSION);
        assert_eq!(
            reply["result"]["capabilities"],
            serde_json::json!(["events", "json-rpc"]),
            "the transport listening is named"
        );
        let refused = client
            .ask(r#"{"jsonrpc":"2.0","id":42,"method":"project.get","params":{"path":"/none"}}"#);
        assert_eq!(refused["error"]["data"]["code"], "notOpen");
        server.stop();
    }

    #[test]
    fn the_auth_line_and_the_first_call_may_arrive_together() {
        let (server, _scratch) = server(None);
        let mut client = Client::reach(&server);
        write!(
            client.writer,
            "\n{}\n{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"version\"}}\n",
            serde_json::json!({
                "jsonrpc": "2.0", "id": 0, "method": "auth", "params": { "token": server.token() },
            }),
        )
        .expect("writes");
        assert_eq!(client.read()["result"], serde_json::json!({}));
        assert_eq!(client.read()["id"], 1);
        server.stop();
    }

    #[test]
    fn two_callers_share_one_api() {
        let (server, scratch) = server(None);
        // Built as JSON rather than pasted into it: a Windows path's
        // backslashes are escapes in a JSON string.
        let location = scratch.path().to_string_lossy().into_owned();
        let shared = scratch.path().join("Shared").to_string_lossy().into_owned();
        let mut first = Client::connect(&server);
        let mut second = Client::connect(&server);
        let created = first.ask(
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "project.create",
                "params": { "location": location, "name": "Shared" },
            })
            .to_string(),
        );
        assert!(created["result"]["project"].is_object(), "{created}");
        let seen = second.ask(
            &serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "project.get",
                "params": { "path": shared },
            })
            .to_string(),
        );
        assert!(seen["result"]["project"].is_object(), "{seen}");
        assert_eq!(server.connections(), 2);
        server.stop();
    }

    #[test]
    fn a_token_is_presented_first_or_the_connection_ends() {
        let (server, _scratch) = server(Some(TOKEN));
        let mut silent = Client::reach(&server);
        let refused = silent.ask(r#"{"jsonrpc":"2.0","id":1,"method":"version"}"#);
        assert_eq!(refused["error"]["data"]["code"], "unauthorized");
        assert_eq!(refused["id"], 1, "the refusal names the call");
        assert!(silent.closed());

        let mut wrong = Client::reach(&server);
        let refused = wrong.auth(&TOKEN[1..]);
        assert_eq!(refused["error"]["data"]["code"], "unauthorized");
        assert!(wrong.closed());

        let mut right = Client::reach(&server);
        assert_eq!(right.auth(TOKEN)["result"], serde_json::json!({}));
        let reply = right.ask(r#"{"jsonrpc":"2.0","id":2,"method":"version"}"#);
        assert_eq!(reply["id"], 2);
        assert_eq!(server.connections(), 1);
        server.stop();
    }

    #[test]
    fn a_minted_token_is_required_on_loopback_too() {
        let (server, _scratch) = server(None);
        let mut silent = Client::reach(&server);
        let refused = silent.ask(r#"{"jsonrpc":"2.0","id":1,"method":"version"}"#);
        assert_eq!(refused["error"]["data"]["code"], "unauthorized");

        let mut right = Client::connect(&server);
        let reply = right.ask(r#"{"jsonrpc":"2.0","id":2,"method":"version"}"#);
        assert_eq!(reply["result"]["apiVersion"], concat_api::API_VERSION);
        server.stop();
    }

    #[test]
    fn a_stop_reaches_connected_callers() {
        let (server, _scratch) = server(None);
        let mut client = Client::connect(&server);
        server.stop();
        assert!(client.closed());
    }
}
