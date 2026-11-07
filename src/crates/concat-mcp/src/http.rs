// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! MCP over HTTP with Server-Sent Events (SSE).
//!
//! Two endpoints share one session:
//! - `POST /mcp` : the agent sends one JSON-RPC message per request body.
//! - `GET  /mcp` : the server pushes responses as `text/event-stream`.
//! A session is named by `Mcp-Session-Id`, echoed back on the response that
//! creates it.
//!
//! Each session runs the synchronous [`crate::protocol::serve`] in its own
//! blocking thread, bridged to HTTP through inbound/outbound channels.

use std::collections::HashMap;
use std::io::{self, BufRead, Read, Write};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::Bytes;
use futures_util::stream::StreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::header::CONTENT_TYPE;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::{Mutex as TokioMutex, broadcast, mpsc};
use tokio_stream::wrappers::BroadcastStream;

use crate::backend::{Backend, Mode};
use crate::protocol;

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

/// Every handler replies with this body: a complete buffer, or a stream for
/// the SSE endpoint, boxed so one response type serves both.
type Body = BoxBody<Bytes, io::Error>;

/// A reply body holding `bytes` and nothing more.
fn complete(bytes: impl Into<Bytes>) -> Body {
    Full::new(bytes.into())
        .map_err(|never: std::convert::Infallible| -> io::Error { match never {} })
        .boxed()
}

/// The sessions, reachable from both HTTP handlers.
type Sessions = Arc<TokioMutex<HashMap<String, Arc<Session>>>>;

/// One MCP session's shared state, reachable from both HTTP handlers.
struct Session {
    id: String,
    inbound_tx: mpsc::UnboundedSender<String>,
    outbound_tx: broadcast::Sender<String>,
}

impl Session {
    /// Creates a session, returning it plus the receiving ends the protocol
    /// thread consumes.
    fn new() -> (
        Arc<Self>,
        mpsc::UnboundedReceiver<String>,
        broadcast::Receiver<String>,
    ) {
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<String>();
        let (outbound_tx, outbound_rx) = broadcast::channel::<String>(16);
        let id = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed).to_string();
        (
            Arc::new(Session {
                id,
                inbound_tx,
                outbound_tx,
            }),
            inbound_rx,
            outbound_rx,
        )
    }
}

/// Reads lines from an mpsc channel, as [`BufRead`] for `protocol::serve`.
struct ChannelReader {
    rx: mpsc::UnboundedReceiver<String>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChannelReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let taken = {
            let available = self.fill_buf()?;
            let taken = available.len().min(out.len());
            out[..taken].copy_from_slice(&available[..taken]);
            taken
        };
        self.consume(taken);
        Ok(taken)
    }
}

impl BufRead for ChannelReader {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.pos >= self.buf.len() {
            self.buf.clear();
            self.pos = 0;
            // A closed channel is end of input, not an error.
            if let Some(line) = self.rx.blocking_recv() {
                self.buf.extend_from_slice(line.as_bytes());
                self.buf.push(b'\n');
            }
        }
        Ok(&self.buf[self.pos..])
    }

    fn consume(&mut self, amount: usize) {
        self.pos += amount;
    }
}

/// Writes lines to a broadcast channel, as [`Write`] for `protocol::serve`.
struct ChannelWriter {
    tx: broadcast::Sender<String>,
}

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let line = String::from_utf8_lossy(buf).to_string();
        self.tx
            .send(line)
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "broadcast channel closed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Runs the protocol over this session's channels in a blocking thread until
/// the inbound channel closes.
fn run_protocol(
    session: Arc<Session>,
    inbound_rx: mpsc::UnboundedReceiver<String>,
    outbound_rx: broadcast::Receiver<String>,
    mode: Mode,
) {
    // Drop the receiver we don't use in this thread; the SSE handler owns it.
    drop(outbound_rx);

    let backend: Box<dyn Backend> = match mode {
        Mode::Headless => match crate::backend::Headless::start() {
            Ok(backend) => backend,
            Err(why) => {
                eprintln!("concat mcp http: headless backend failed: {why}");
                return;
            }
        },
        Mode::Window => match crate::backend::Window::find() {
            Ok(backend) => backend,
            Err(why) => {
                eprintln!("concat mcp http: window backend failed: {why}");
                return;
            }
        },
        Mode::Auto => match crate::backend::Window::find() {
            Ok(backend) => backend,
            Err(_) => match crate::backend::Headless::start() {
                Ok(backend) => backend,
                Err(why) => {
                    eprintln!("concat mcp http: no backend available: {why}");
                    return;
                }
            },
        },
        #[cfg(target_os = "android")]
        Mode::WindowLocal => match crate::backend::WindowLocal::find() {
            Ok(backend) => backend,
            Err(why) => {
                eprintln!("concat mcp http: window-local backend failed: {why}");
                return;
            }
        },
    };

    let reader = ChannelReader {
        rx: inbound_rx,
        buf: Vec::new(),
        pos: 0,
    };
    let writer = ChannelWriter {
        tx: session.outbound_tx.clone(),
    };
    protocol::serve(reader, writer, backend);
}

/// The HTTP service: dispatches `POST /mcp` and `GET /mcp`.
async fn handle_request(
    req: Request<Incoming>,
    sessions: Sessions,
    mode: Mode,
) -> Result<Response<Body>, io::Error> {
    if req.uri().path() != "/mcp" {
        return Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(complete("not found"))
            .expect("a fixed response builds"));
    }

    match *req.method() {
        Method::POST => handle_post(req, sessions, mode).await,
        Method::GET => handle_get(req, sessions).await,
        _ => Ok(Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .body(complete("method not allowed"))
            .expect("a fixed response builds")),
    }
}

/// POST /mcp: queue one JSON-RPC message for the session.
async fn handle_post(
    req: Request<Incoming>,
    sessions: Sessions,
    mode: Mode,
) -> Result<Response<Body>, io::Error> {
    let session_id = req
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let session = match session_id.as_deref() {
        Some(id) => sessions.lock().await.get(id).cloned(),
        None => None,
    };

    let session = match session {
        Some(session) => session,
        None => {
            let (session, inbound_rx, outbound_rx) = Session::new();

            std::thread::spawn({
                let session = Arc::clone(&session);
                move || run_protocol(session, inbound_rx, outbound_rx, mode)
            });

            sessions
                .lock()
                .await
                .insert(session.id.clone(), Arc::clone(&session));
            session
        }
    };

    let body = req
        .into_body()
        .collect()
        .await
        .map_err(|error| io::Error::other(format!("could not read the request body: {error}")))?
        .to_bytes();
    let text = String::from_utf8_lossy(&body).into_owned();

    session
        .inbound_tx
        .send(text)
        .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the session has ended"))?;

    Ok(Response::builder()
        .status(StatusCode::ACCEPTED)
        .header("mcp-session-id", session.id.as_str())
        .body(complete(Bytes::new()))
        .expect("a fixed response builds"))
}

/// GET /mcp: SSE stream of responses for the session.
async fn handle_get(
    req: Request<Incoming>,
    sessions: Sessions,
) -> Result<Response<Body>, io::Error> {
    let session_id = req
        .headers()
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "missing mcp-session-id header")
        })?;

    let session = sessions
        .lock()
        .await
        .get(&session_id)
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "session not found"))?;

    let rx = session.outbound_tx.subscribe();

    // A lagging reader loses the messages it did not keep up with rather
    // than ending the stream.
    let stream = BroadcastStream::new(rx)
        .filter_map(|result| std::future::ready(result.ok()))
        .map(|line| {
            Ok::<Frame<Bytes>, io::Error>(Frame::data(Bytes::from(format!("data: {line}\n\n"))))
        });

    Ok(Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/event-stream")
        .header("cache-control", "no-cache")
        .header("connection", "keep-alive")
        .body(BodyExt::boxed(StreamBody::new(stream)))
        .expect("a fixed response builds"))
}

/// Serves MCP over HTTP/SSE on `addr`, through the backend `mode` picks.
pub async fn run_http(addr: SocketAddr, mode: Mode) -> Result<(), String> {
    let sessions: Sessions = Sessions::default();

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|error| format!("could not bind {addr}: {error}"))?;

    eprintln!("concat mcp: serving SSE on http://{addr}");

    loop {
        let (stream, _peer) = listener
            .accept()
            .await
            .map_err(|error| format!("accept failed: {error}"))?;
        let io = TokioIo::new(stream);

        let service = {
            let sessions = Arc::clone(&sessions);
            service_fn(move |req| {
                let sessions = Arc::clone(&sessions);
                async move { handle_request(req, sessions, mode).await }
            })
        };

        tokio::spawn(async move {
            if let Err(error) = hyper::server::conn::http1::Builder::new()
                .serve_connection(io, service)
                .await
            {
                eprintln!("concat mcp http: connection error: {error}");
            }
        });
    }
}
