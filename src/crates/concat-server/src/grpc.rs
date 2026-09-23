// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! gRPC over HTTP/2, through tonic.
//!
//! The service is two methods: `Call`, one request in and one response
//! out, and `Events`, a stream of every job's events for as long as the
//! caller keeps it open. Payloads are the API's JSON as text, so this is
//! the line transport's contract in a protobuf envelope; `proto/concat.proto`
//! says so at length.
//!
//! tonic needs tokio, and the rest of the workspace does not, so the
//! runtime lives here: one, on its own thread, doing nothing but I/O. A
//! call is handed to the [`Hub`] from a blocking task, the same way a
//! JSON-RPC connection thread hands one over, and waits there.
//!
//! Every call carries the server's token as `authorization: Bearer ...`
//! metadata; one without it, or with another, is `UNAUTHENTICATED`.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use concat_api::rpc::{Call, Message};
use concat_api::{ApiError, Event, Response};
use serde_json::{Value, json};
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Status};

use crate::{Hub, token};

/// The generated messages and service.
pub mod proto {
    #![allow(missing_docs)]
    tonic::include_proto!("concat.v1");
}

use proto::concat_server::{Concat, ConcatServer};

/// The service: the hub, and nothing else.
struct Service {
    hub: Hub,
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

    type EventsStream = ReceiverStream<Result<proto::Event, Status>>;

    async fn events(
        &self,
        _request: Request<proto::EventsRequest>,
    ) -> Result<tonic::Response<Self::EventsStream>, Status> {
        let (sender, receiver) = tokio::sync::mpsc::channel(64);
        // Called on the job's thread, which must not wait on a caller: one
        // that has not taken the last sixty-four events is treated as gone,
        // like one that hung up.
        self.hub.subscribe(Arc::new(move |event: &Event| {
            sender.try_send(Ok(encode_event(event))).is_ok()
        }));
        Ok(tonic::Response::new(ReceiverStream::new(receiver)))
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
                let service = ConcatServer::with_interceptor(Service { hub }, bearer(token));
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
/// in constant time.
fn bearer(token: String) -> impl Fn(Request<()>) -> Result<Request<()>, Status> + Clone {
    let expected = format!("Bearer {token}");
    move |request: Request<()>| {
        let presented = request
            .metadata()
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if token::matches(presented.as_bytes(), expected.as_bytes()) {
            Ok(request)
        } else {
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
        let (server, _scratch) = server(Some("open sesame"));
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
                .call(with_bearer(version(), "open sesame"))
                .await
                .is_ok()
        );
        drop(client);
        tokio::task::spawn_blocking(move || server.stop())
            .await
            .expect("stops");
    }
}
