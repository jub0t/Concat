// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! gRPC over HTTP/2, through tonic.
//!
//! The service is two methods: `Call`, one request in and one response
//! out, and `Events`, a stream of every job's events for as long as the
//! caller keeps it open. Payloads are the API's JSON as text, so this is
//! the line transport's contract in a protobuf envelope; `proto/concat.proto`
//! says so at length.
//!
//! An Events stream is held, not given away: at most [`MAX_STREAMS`] are
//! open at once - one more is answered with `resource_exhausted`, the way
//! the JSON door answers a caller past its seats - and each is taken out
//! of the hub's fan-out the moment it ends, not when the next event finds
//! it gone. A caller that opens streams and abandons them has to run out
//! of streams before the server runs out of anything else (audit
//! 2026-09-23: an abandoned stream stayed subscribed until a later event
//! noticed, which on an idle server is never, and there was no cap at
//! all).
//!
//! tonic needs tokio, and the rest of the workspace does not, so the
//! runtime lives here: one, on its own thread, doing nothing but I/O. A
//! call is handed to the [`Hub`] from a blocking task, the same way a
//! JSON-RPC connection thread hands one over, and waits there.
//!
//! Every call carries the server's token as `authorization: Bearer ...`
//! metadata; one without it, or with another, is `UNAUTHENTICATED`.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::thread::JoinHandle;
use std::time::Duration;

use concat_api::rpc::{Call, Message};
use concat_api::{ApiError, Event, Response};
use serde_json::{Value, json};
use tokio_stream::{Stream, wrappers::ReceiverStream};
use tonic::{Request, Status};

use crate::throttle::{Peer, Throttle};
use crate::{Hub, token};

/// The generated messages and service.
pub mod proto {
    #![allow(missing_docs)]
    tonic::include_proto!("concat.v1");
}

use proto::concat_server::{Concat, ConcatServer};

/// How many Events streams may be open at once; one more is answered
/// with `resource_exhausted`. The same number as the JSON door's
/// [`crate::json::MAX_CONNECTIONS`] seats, and for the same reason: a
/// caller has to run out of streams before the server runs out of
/// anything else. A stream here is a channel and an entry in the fan-out,
/// where a JSON seat is a thread and a socket, so the numbers may differ;
/// they have not needed to. Public the way the JSON door's own caps are:
/// the crate's docs say what this one is.
pub const MAX_STREAMS: usize = 64;

/// The service: the hub, and a count of the Events streams open.
struct Service {
    hub: Hub,
    seats: Arc<AtomicUsize>,
}

#[tonic::async_trait]
impl Concat for Service {
    async fn call(
        &self,
        request: Request<proto::Request>,
    ) -> Result<tonic::Response<proto::Response>, Status> {
        let proto::Request { method, params } = request.into_inner();
        let hub = self.hub.clone();
        let response = tokio::task::spawn_blocking(move || match parse(&method, &params) {
            Ok(call) => hub.call(call.request),
            Err(error) => Response::Error(error),
        })
        .await
        .map_err(|error| Status::internal(error.to_string()))?;
        Ok(tonic::Response::new(encode(response)))
    }

    type EventsStream = Guarded;

    async fn events(
        &self,
        _request: Request<proto::EventsRequest>,
    ) -> Result<tonic::Response<Self::EventsStream>, Status> {
        // A seat first, before anything is held for this caller: one past
        // the cap is refused the way the JSON door refuses a caller past
        // its seats (audit 2026-09-23: there was no cap here at all).
        let Some(seat) = Seat::take(&self.seats) else {
            return Err(Status::resource_exhausted(format!(
                "{MAX_STREAMS} event streams are open already"
            )));
        };
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        // Called on the job's thread, which must not wait on a caller: one
        // that has not taken the last sixty-four events is treated as gone,
        // like one that hung up.
        let id = self.hub.subscribe(Arc::new(move |event: &Event| {
            sender.try_send(Ok(encode_event(event))).is_ok()
        }));
        // The guards ride in the stream tonic is handed: it drops the
        // response body when the caller's stream ends - the caller went
        // away, reset the HTTP/2 stream, or the server stopped - and the
        // fan-out entry and the seat come back with it, not at the next
        // event, which on an idle server is never (audit 2026-09-23:
        // abandoned streams accumulated until one did).
        Ok(tonic::Response::new(Guarded {
            events: ReceiverStream::new(receiver),
            seat,
            subscribed: Subscribed {
                hub: self.hub.clone(),
                id,
            },
        }))
    }
}

/// One of a counted number of open Events streams, given back when
/// dropped. json.rs's `Place`, written again here rather than shared
/// there: this door counts streams, that one seats connections, and each
/// keeps its own count.
struct Seat(Arc<AtomicUsize>);

impl Seat {
    /// Takes a place if one is free; `None` when [`MAX_STREAMS`] streams
    /// are open already.
    fn take(count: &Arc<AtomicUsize>) -> Option<Seat> {
        let mut taken = count.load(Ordering::SeqCst);
        loop {
            if taken >= MAX_STREAMS {
                return None;
            }
            match count.compare_exchange(taken, taken + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => return Some(Seat(Arc::clone(count))),
                Err(now) => taken = now,
            }
        }
    }
}

impl Drop for Seat {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// An Events stream's entry in the hub's fan-out, taken out when the
/// stream drops: json.rs's `Registered` pattern, for a subscriber rather
/// than a connection. The stream guard owns one, so a caller that has
/// gone is forgotten at once instead of at the next event.
struct Subscribed {
    hub: Hub,
    id: u64,
}

impl Drop for Subscribed {
    fn drop(&mut self) {
        self.hub.unsubscribe_by_id(self.id);
    }
}

/// What `Events` hands back: the channel's receiving end wearing the
/// guards - the seat, the fan-out entry - that come back when the stream
/// ends. Every poll is handed straight to the receiver; only the guards
/// are this type's own business.
struct Guarded {
    events: ReceiverStream<Result<proto::Event, Status>>,
    // Held for their Drop alone: the seat frees when the stream ends and
    // the fan-out entry goes with it. Nothing ever reads them, which is
    // the point of a guard, so the fields say so to the lint that cannot
    // see a drop coming.
    #[allow(dead_code)]
    seat: Seat,
    #[allow(dead_code)]
    subscribed: Subscribed,
}

impl Stream for Guarded {
    type Item = Result<proto::Event, Status>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        // No projection needed: the receiver and the guards are `Unpin`,
        // so the guard type is too.
        let this = self.get_mut();
        Pin::new(&mut this.events).poll_next(cx)
    }
}

/// The request the method and params text name, by way of the JSON-RPC
/// parser so the two transports refuse the same things the same way.
fn parse(method: &str, params: &str) -> Result<Call, ApiError> {
    let params: Value = if params.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(params)
            .map_err(|error| ApiError::invalid(format!("params is not JSON: {error}")))?
    };
    let line = json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string();
    Call::parse(&line).map_err(|(_, error)| error)
}

fn encode(response: Response) -> proto::Response {
    use proto::response::Outcome;
    let outcome = match response {
        Response::Result(reply) => {
            Outcome::Result(serde_json::to_string(&reply).unwrap_or_else(|_| "{}".to_owned()))
        }
        Response::Error(error) => Outcome::Error(proto::Error {
            code: serde_json::to_value(error.code)
                .ok()
                .and_then(|code| code.as_str().map(str::to_owned))
                .unwrap_or_default(),
            number: error.code.number() as i32,
            message: error.message,
        }),
    };
    proto::Response {
        outcome: Some(outcome),
    }
}

fn encode_event(event: &Event) -> proto::Event {
    // The notification's method and params, exactly as the line transport
    // would write them.
    let notification = Message::Event(event.clone()).to_value();
    proto::Event {
        event: notification["method"]
            .as_str()
            .unwrap_or_default()
            .to_owned(),
        params: notification["params"].to_string(),
    }
}

/// Serves gRPC on `address` until `stop` is set. Returns the address
/// bound, port resolved, and the runtime's thread.
pub(crate) fn serve(
    address: SocketAddr,
    hub: Hub,
    token: String,
    stop: Arc<AtomicBool>,
    throttle: Arc<Throttle>,
) -> Result<(SocketAddr, JoinHandle<()>), String> {
    let could_not =
        |error: &dyn std::fmt::Display| format!("could not listen on {address}: {error}");
    let listener = std::net::TcpListener::bind(address).map_err(|error| could_not(&error))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| could_not(&error))?;
    let bound = listener.local_addr().map_err(|error| could_not(&error))?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("concat grpc")
        .enable_all()
        .build()
        .map_err(|error| format!("could not start the gRPC runtime: {error}"))?;

    // Everything the runtime runs, it runs from its own thread: a block_on
    // from a caller that is itself inside a runtime - a test, an embedder
    // with one of its own - would panic.
    let thread = std::thread::Builder::new()
        .name("concat grpc".to_owned())
        .spawn(move || {
            runtime.block_on(async move {
                let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
                    return;
                };
                let incoming = tokio_stream::wrappers::TcpListenerStream::new(listener);
                let service = ConcatServer::with_interceptor(
                    Service {
                        hub,
                        // This door's count of open Events streams, at zero
                        // until a caller opens one; see [`MAX_STREAMS`].
                        seats: Arc::new(AtomicUsize::new(0)),
                    },
                    bearer(token, throttle),
                );
                let stopped = async move {
                    while !stop.load(Ordering::SeqCst) {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                };
                let _ = tonic::transport::Server::builder()
                    .add_service(service)
                    .serve_with_incoming_shutdown(incoming, stopped)
                    .await;
            });
        })
        .map_err(|error| format!("could not start the gRPC thread: {error}"))?;
    Ok((bound, thread))
}

/// The check every call passes through: the token as a bearer, compared
/// in constant time, from an address the throttle is not holding back.
fn bearer(
    token: String,
    throttle: Arc<Throttle>,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone {
    let expected = format!("Bearer {token}");
    move |request: Request<()>| {
        let peer = request
            .remote_addr()
            .map(|address| Peer::Ip(address.ip()))
            .unwrap_or(Peer::Local);
        if let Some(wait) = throttle.wait(peer) {
            return Err(Status::resource_exhausted(format!(
                "too many wrong tokens from this address: try again in {} s",
                wait.as_secs().max(1)
            )));
        }
        let presented = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if token::matches(presented.as_bytes(), expected.as_bytes()) {
            throttle.succeeded(peer);
            Ok(request)
        } else {
            throttle.failed(peer);
            Err(Status::unauthenticated("the token is missing or wrong"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Config, Server};
    use concat_api::{Api, AppDirs};
    use proto::concat_client::ConcatClient;

    fn server(token: Option<&str>) -> (Server, tempfile::TempDir) {
        let scratch = tempfile::tempdir().expect("scratch");
        let dirs = AppDirs {
            config: scratch.path().join("config"),
            data: scratch.path().join("data"),
        };
        let config = Config {
            grpc: Some("127.0.0.1:0".parse().expect("an address")),
            token: token.map(str::to_owned),
            ..Config::default()
        };
        let server =
            Server::start(config, move |events| Ok(Api::with_dirs(dirs, events))).expect("starts");
        (server, scratch)
    }

    /// `request`, with `token` as its bearer.
    fn with_bearer<T>(request: T, token: &str) -> Request<T> {
        let mut request = Request::new(request);
        request.metadata_mut().insert(
            "authorization",
            format!("Bearer {token}").parse().expect("a value"),
        );
        request
    }

    #[tokio::test]
    async fn a_call_answers_in_json_text_and_an_error_carries_its_code() {
        let (server, _scratch) = server(None);
        let address = server.grpc_addr().expect("listening");
        let mut client = ConcatClient::connect(format!("http://{address}"))
            .await
            .expect("connects");
        let version = client
            .call(with_bearer(
                proto::Request {
                    method: "version".to_owned(),
                    params: String::new(),
                },
                server.token(),
            ))
            .await
            .expect("answers")
            .into_inner();
        let Some(proto::response::Outcome::Result(text)) = version.outcome else {
            panic!("not a result: {version:?}");
        };
        let value: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(value["apiVersion"], concat_api::API_VERSION);
        assert_eq!(value["capabilities"], json!(["events", "grpc"]));

        let refused = client
            .call(with_bearer(
                proto::Request {
                    method: "project.get".to_owned(),
                    params: r#"{"path":"/none"}"#.to_owned(),
                },
                server.token(),
            ))
            .await
            .expect("answers")
            .into_inner();
        let Some(proto::response::Outcome::Error(error)) = refused.outcome else {
            panic!("not an error: {refused:?}");
        };
        assert_eq!((error.code.as_str(), error.number), ("notOpen", -32001));
        drop(client);
        tokio::task::spawn_blocking(move || server.stop())
            .await
            .expect("stops");
    }

    #[tokio::test]
    async fn a_token_travels_as_a_bearer() {
        let (server, _scratch) = server(Some(crate::tests::TOKEN));
        let address = server.grpc_addr().expect("listening");
        let mut client = ConcatClient::connect(format!("http://{address}"))
            .await
            .expect("connects");
        let version = || proto::Request {
            method: "version".to_owned(),
            params: String::new(),
        };
        let refused = client.call(version()).await.expect_err("no token");
        assert_eq!(refused.code(), tonic::Code::Unauthenticated);
        let refused = client
            .call(with_bearer(version(), "open"))
            .await
            .expect_err("wrong token");
        assert_eq!(refused.code(), tonic::Code::Unauthenticated);

        assert!(
            client
                .call(with_bearer(version(), crate::tests::TOKEN))
                .await
                .is_ok()
        );
        drop(client);
        tokio::task::spawn_blocking(move || server.stop())
            .await
            .expect("stops");
    }

    /// An Events stream takes one of [`MAX_STREAMS`] seats and gives it
    /// back when it ends, and takes itself out of the fan-out with it: a
    /// caller that opens streams and walks away has to run out of them,
    /// and leaves nothing subscribed behind (audit 2026-09-23 - before,
    /// an abandoned stream stayed in the fan-out until the next event,
    /// which never comes on an idle server).
    #[tokio::test]
    async fn events_streams_are_capped_and_not_left_subscribed() {
        let (server, _scratch) = server(None);
        let address = server.grpc_addr().expect("listening");
        let token = server.token().to_owned();
        let mut client = ConcatClient::connect(format!("http://{address}"))
            .await
            .expect("connects");
        let mut streams = Vec::new();
        for _ in 0..MAX_STREAMS {
            let stream = client
                .events(with_bearer(proto::EventsRequest {}, &token))
                .await
                .expect("a stream");
            streams.push(stream.into_inner());
        }
        // One past the cap: refused, not taken.
        match client
            .events(with_bearer(proto::EventsRequest {}, &token))
            .await
        {
            Ok(_) => panic!("the streams were capped, but one more was taken"),
            Err(status) => assert_eq!(status.code(), tonic::Code::ResourceExhausted),
        }
        // Gone: the guards run as tonic drops the streams' bodies, and the
        // hub is left with nobody subscribed.
        drop(streams);
        drop(client);
        let unsubscribed = tokio::time::timeout(Duration::from_secs(5), async {
            while server.hub().subscribers() > 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(
            unsubscribed.is_ok(),
            "{} subscribers were left behind",
            server.hub().subscribers()
        );
        // A seat given back is taken again.
        let mut client = ConcatClient::connect(format!("http://{address}"))
            .await
            .expect("connects");
        client
            .events(with_bearer(proto::EventsRequest {}, &token))
            .await
            .expect("a seat was given back");
        drop(client);
        tokio::task::spawn_blocking(move || server.stop())
            .await
            .expect("stops");
    }
}
