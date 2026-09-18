// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! JSON-RPC lines on a socket: the stdin transport, for several callers at
//! once.
//!
//! A connection is two threads: one reads lines and answers each through
//! the [`Hub`], one writes whatever is put in the connection's outbox -
//! responses and events alike - so a job's event from another thread never
//! interleaves with a response half-written. The wire is exactly what
//! `concat-cli api` reads and writes; see `concat_api::rpc`.
//!
//! A connection's first line is `{"jsonrpc": "2.0", "id": 1, "method":
//! "auth", "params": {"token": "..."}}`, with the server's token. Anything
//! else first, or the wrong token, is answered with an `unauthorized` error
//! and the connection is closed. `auth` is the transport's word, not the
//! API's: it never reaches the hub.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use concat_api::rpc::{Call, Id, Message};
use concat_api::{ApiError, Done, ErrorCode, Reply, Response};
use serde_json::Value;

use crate::{Connections, Hub, token};

/// Accepts JSON-RPC connections on `listener` until `stop` is set.
pub(crate) fn serve_tcp(
    listener: TcpListener,
    hub: Hub,
    token: String,
    stop: Arc<AtomicBool>,
    connections: Connections,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let _ = stream.set_nodelay(true);
            let Ok(reader) = stream.try_clone() else {
                continue;
            };
            let Ok(closer) = stream.try_clone() else {
                continue;
            };
            let Ok(ender) = stream.try_clone() else {
                continue;
            };
            if let Ok(mut connections) = connections.lock() {
                connections.push(Box::new(move || {
                    let _ = closer.shutdown(std::net::Shutdown::Both);
                }));
            }
            let hub = hub.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                connection(BufReader::new(reader), stream, hub, token);
                // The registry above still holds a handle, so the socket is
                // shut here rather than merely dropped: the caller sees EOF.
                let _ = ender.shutdown(std::net::Shutdown::Both);
            });
        }
    })
}

/// Accepts JSON-RPC connections on a Unix socket until `stop` is set.
#[cfg(unix)]
pub(crate) fn serve_unix(
    listener: std::os::unix::net::UnixListener,
    hub: Hub,
    token: String,
    stop: Arc<AtomicBool>,
    connections: Connections,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stop.load(Ordering::SeqCst) {
                break;
            }
            let Ok(stream) = stream else { continue };
            let Ok(reader) = stream.try_clone() else {
                continue;
            };
            let Ok(closer) = stream.try_clone() else {
                continue;
            };
            let Ok(ender) = stream.try_clone() else {
                continue;
            };
            if let Ok(mut connections) = connections.lock() {
                connections.push(Box::new(move || {
                    let _ = closer.shutdown(std::net::Shutdown::Both);
                }));
            }
            let hub = hub.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                connection(BufReader::new(reader), stream, hub, token);
                // The registry above still holds a handle, so the socket is
                // shut here rather than merely dropped: the caller sees EOF.
                let _ = ender.shutdown(std::net::Shutdown::Both);
            });
        }
    })
}

/// One caller, start to end: the handshake, then a call per line.
fn connection(reader: impl BufRead, writer: impl Write + Send + 'static, hub: Hub, token: String) {
    let outbox = Outbox::start(writer);
    let mut lines = reader.lines().map_while(Result::ok);

    let Some(first) = lines.find(|line| !line.trim().is_empty()) else {
        outbox.close();
        return;
    };
    let (id, outcome) = handshake(&first, &token);
    let ok = outcome.is_ok();
    outbox.send(Message::Reply {
        id,
        response: outcome.into(),
    });
    if !ok {
        outbox.close();
        return;
    }

    // Events reach this caller from here on: the outbox is shared with the
    // hub's fan-out, which drops it once a send fails.
    let events = outbox.clone();
    hub.subscribe(Box::new(move |event| {
        events.send(Message::Event(event.clone()))
    }));

    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let (id, response) = match Call::parse(&line) {
            Ok(call) => (call.id, hub.call(call.request)),
            Err((id, error)) => (id, Response::Error(error)),
        };
        if !outbox.send(Message::Reply { id, response }) {
            break;
        }
    }
    outbox.close();
}

/// Reads the first line as the `auth` call and checks its token, in
/// constant time.
fn handshake(line: &str, token: &str) -> (Option<Id>, Result<Reply, ApiError>) {
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
        (id, Ok(Reply::Done(Done {})))
    } else {
        refuse(id, "wrong token")
    }
}

/// The connection's one writer: a queue and the thread that drains it.
#[derive(Clone)]
struct Outbox {
    queue: Arc<Mutex<Option<Sender<Message>>>>,
    writer: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Outbox {
    fn start(mut writer: impl Write + Send + 'static) -> Outbox {
        let (queue, messages) = mpsc::channel::<Message>();
        let thread = std::thread::spawn(move || {
            for message in messages {
                if writeln!(writer, "{}", message.to_json()).is_err() || writer.flush().is_err() {
                    break;
                }
            }
        });
        Outbox {
            queue: Arc::new(Mutex::new(Some(queue))),
            writer: Arc::new(Mutex::new(Some(thread))),
        }
    }

    /// Queues one line. False once the caller is gone.
    fn send(&self, message: Message) -> bool {
        match self.queue.lock() {
            Ok(queue) => queue
                .as_ref()
                .is_some_and(|queue| queue.send(message).is_ok()),
            Err(_) => false,
        }
    }

    /// Ends the queue and waits for the writer to finish what is left, so
    /// the last reply is on the wire when this returns. The fan-out forgets
    /// this caller at its next event.
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
    use crate::tests::server;
    use std::net::TcpStream;

    /// A client on `server`: a line in, a line out.
    struct Client {
        reader: BufReader<TcpStream>,
        writer: TcpStream,
    }

    impl Client {
        /// A connection with the server's token presented, ready to call.
        fn connect(server: &crate::Server) -> Client {
            let mut client = Client::reach(server);
            let welcomed = client.ask(&format!(
                r#"{{"jsonrpc":"2.0","id":0,"method":"auth","params":{{"token":"{}"}}}}"#,
                server.token()
            ));
            assert_eq!(welcomed["result"], serde_json::json!({}), "{welcomed}");
            client
        }

        /// A connection with nothing said yet.
        fn reach(server: &crate::Server) -> Client {
            let stream =
                TcpStream::connect(server.json_addr().expect("listening")).expect("connects");
            Client {
                reader: BufReader::new(stream.try_clone().expect("clone")),
                writer: stream,
            }
        }

        fn ask(&mut self, line: &str) -> Value {
            writeln!(self.writer, "{line}").expect("writes");
            let mut reply = String::new();
            self.reader.read_line(&mut reply).expect("reads");
            serde_json::from_str(&reply).expect("a JSON line")
        }
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
    fn two_callers_share_one_api() {
        let (server, scratch) = server(None);
        let location = scratch.path().to_string_lossy().into_owned();
        let mut first = Client::connect(&server);
        let mut second = Client::connect(&server);
        let created = first.ask(&format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"project.create","params":{{"location":"{location}","name":"Shared"}}}}"#
        ));
        assert!(created["result"]["project"].is_object(), "{created}");
        let seen = second.ask(&format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"project.get","params":{{"path":"{location}/Shared"}}}}"#
        ));
        assert!(seen["result"]["project"].is_object(), "{seen}");
        assert_eq!(server.connections(), 2);
        server.stop();
    }

    #[test]
    fn a_token_is_presented_first_or_the_connection_ends() {
        let (server, _scratch) = server(Some("open sesame"));
        let mut silent = Client::reach(&server);
        let refused = silent.ask(r#"{"jsonrpc":"2.0","id":1,"method":"version"}"#);
        assert_eq!(refused["error"]["data"]["code"], "unauthorized");
        let mut after = String::new();
        assert_eq!(
            silent.reader.read_line(&mut after).expect("eof"),
            0,
            "closed"
        );

        let mut wrong = Client::reach(&server);
        let refused =
            wrong.ask(r#"{"jsonrpc":"2.0","id":1,"method":"auth","params":{"token":"open"}}"#);
        assert_eq!(refused["error"]["data"]["code"], "unauthorized");
        assert_eq!(wrong.reader.read_line(&mut after).expect("eof"), 0);

        let mut right = Client::reach(&server);
        let welcomed = right
            .ask(r#"{"jsonrpc":"2.0","id":1,"method":"auth","params":{"token":"open sesame"}}"#);
        assert_eq!(welcomed["result"], serde_json::json!({}));
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
}
