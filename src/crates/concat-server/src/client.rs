// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! The other end of [`crate::json`]: a blocking JSON-RPC line client, for a
//! process that drives a running server - the MCP adapter going through
//! the window, a test.
//!
//! One call at a time. The events the server sends between replies are
//! not the client's business here - a caller that wants to follow a job
//! asks `export.status` - so they are read past and dropped.

use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use concat_api::{ApiError, ErrorCode, Request};
use serde_json::{Value, json};

/// The longest reply line read: a project's whole document fits, and an
/// inline 8K frame.
const MAX_REPLY: u64 = 256 * 1024 * 1024;

/// A connection to a server, its token presented.
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
    address: SocketAddr,
}

impl Client {
    /// Connects to `address` and presents `token`. `patience` bounds the
    /// connect and the handshake, not the calls after it, which take as
    /// long as their method does.
    pub fn connect(address: SocketAddr, token: &str, patience: Duration) -> Result<Client, String> {
        let could_not = |error: &dyn std::fmt::Display| {
            format!("could not reach the Concat server at {address}: {error}")
        };
        let stream =
            TcpStream::connect_timeout(&address, patience).map_err(|error| could_not(&error))?;
        let _ = stream.set_nodelay(true);
        stream
            .set_read_timeout(Some(patience))
            .map_err(|error| could_not(&error))?;
        let writer = stream.try_clone().map_err(|error| could_not(&error))?;
        let mut client = Client {
            reader: BufReader::new(stream),
            writer,
            next_id: 1,
            address,
        };
        client
            .exchange("auth", json!({ "token": token }))
            .map_err(|error| could_not(&error))?;
        client
            .reader
            .get_ref()
            .set_read_timeout(None)
            .map_err(|error| could_not(&error))?;
        Ok(client)
    }

    /// Where this client is connected.
    pub fn address(&self) -> SocketAddr {
        self.address
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
            // An event, or the refusal of a connection the server never
            // let in (null id): the latter is the answer.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::server;

    #[test]
    fn a_client_presents_its_token_and_calls_through_events() {
        let (server, scratch) = server(None);
        let address = server.json_addr().expect("listening");
        let wrong = Client::connect(address, "not the token, not at all", Duration::from_secs(5));
        assert!(wrong.is_err());
        let mut client =
            Client::connect(address, server.token(), Duration::from_secs(5)).expect("in");
        let version = client.call(&Request::Version).expect("answers");
        assert_eq!(version["apiVersion"], concat_api::API_VERSION);
        let location = scratch.path().to_string_lossy().into_owned();
        let created = client
            .call(&Request::ProjectCreate {
                location,
                name: "Remote".to_owned(),
                video: None,
            })
            .expect("creates");
        assert!(created["project"].is_object(), "{created}");
        let refused = client
            .call(&Request::ProjectGet {
                path: "/nowhere".to_owned(),
            })
            .expect_err("not open");
        assert_eq!(refused.code, ErrorCode::NotOpen);
        drop(client);
        server.stop();
    }
}
