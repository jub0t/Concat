// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The JSON-RPC 2.0 envelope: what a line transport - stdin, a socket -
//! puts around a [`Request`], a [`Response`] and an [`Event`].
//!
//! A call is `{"jsonrpc": "2.0", "id": 1, "method": "edit.apply", "params":
//! {...}}`; its response is `{"jsonrpc": "2.0", "id": 1, "result": ...}` or
//! `{"jsonrpc": "2.0", "id": 1, "error": {"code": -32001, "message": "...",
//! "data": {"code": "notOpen"}}}`; an event is a notification, `{"jsonrpc":
//! "2.0", "method": "export.progress", "params": {...}}`. The id is the
//! caller's and comes back as given, so a caller with several requests in
//! flight tells the responses apart; every call gets a response, id or
//! not, because a transport that never answers is not one a script can
//! drive.
//!
//! For a person typing at the stdin transport, the bare shape
//! `{"method": "...", ...fields}` is accepted too: it is the request with no
//! envelope, and it is answered with a null id.

use serde_json::{Map, Value, json};

use crate::message::{ApiError, ErrorCode, Event, Request, Response};

/// A caller's id: a string, a number or null, echoed back as given.
pub type Id = Value;

/// One incoming line, parsed.
#[derive(Clone, PartialEq, Debug)]
pub struct Call {
    /// The caller's id, when it gave one.
    pub id: Option<Id>,
    /// What it asked for.
    pub request: Request,
}

impl Call {
    /// Parses one line. A line that is not a call comes back as the error
    /// to answer it with, alongside whatever id could be read from it, so
    /// the caller that sent it can still match the refusal.
    pub fn parse(line: &str) -> Result<Call, (Option<Id>, ApiError)> {
        let value: Value = serde_json::from_str(line)
            .map_err(|error| (None, ApiError::new(ErrorCode::Parse, error.to_string())))?;
        let Value::Object(mut object) = value else {
            return Err((None, ApiError::invalid("a request is a JSON object")));
        };
        let id = match object.remove("id") {
            None => None,
            Some(id @ (Value::String(_) | Value::Number(_) | Value::Null)) => Some(id),
            Some(_) => {
                return Err((
                    None,
                    ApiError::invalid("an id is a string, a number or null"),
                ));
            }
        };
        let refuse = |message: String| (id.clone(), ApiError::invalid(message));

        let flat = match object.remove("jsonrpc") {
            None => object,
            Some(Value::String(version)) if version == "2.0" => {
                let Some(Value::String(method)) = object.remove("method") else {
                    return Err(refuse("a call names its method".to_owned()));
                };
                let mut flat = match object.remove("params") {
                    None | Some(Value::Null) => Map::new(),
                    Some(Value::Object(params)) => params,
                    Some(_) => return Err(refuse("params is an object".to_owned())),
                };
                flat.insert("method".to_owned(), Value::String(method));
                flat
            }
            Some(_) => return Err(refuse("jsonrpc is \"2.0\"".to_owned())),
        };
        let request = serde_json::from_value::<Request>(Value::Object(flat))
            .map_err(|error| refuse(error.to_string()))?;
        Ok(Call { id, request })
    }
}

/// One outgoing line.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Message {
    /// The answer to a call.
    Reply {
        /// The call's id; null when it gave none.
        id: Option<Id>,
        /// The answer.
        response: Response,
    },
    /// Something that happened to a job.
    Event(Event),
}

impl Message {
    /// The line, without its newline.
    pub fn to_json(&self) -> String {
        self.to_value().to_string()
    }

    /// The envelope as a JSON value.
    pub fn to_value(&self) -> Value {
        match self {
            Message::Reply { id, response } => {
                let id = id.clone().unwrap_or(Value::Null);
                match response {
                    Response::Result(reply) => {
                        json!({ "jsonrpc": "2.0", "id": id, "result": reply })
                    }
                    Response::Error(error) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": error.code.number(),
                            "message": error.message,
                            "data": { "code": error.code },
                        },
                    }),
                }
            }
            Message::Event(event) => {
                // The event's tag becomes the notification's method and the
                // rest of its fields its params, so an event reads like a
                // call the server made.
                let mut params = match serde_json::to_value(event) {
                    Ok(Value::Object(fields)) => fields,
                    _ => Map::new(),
                };
                let method = params.remove("event").unwrap_or(Value::Null);
                json!({ "jsonrpc": "2.0", "method": method, "params": params })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::{Done, Reply};

    #[test]
    fn an_envelope_parses_to_the_request_it_wraps() {
        let call = Call::parse(
            r#"{"jsonrpc":"2.0","id":7,"method":"project.open","params":{"path":"/p"}}"#,
        )
        .expect("a call");
        assert_eq!(call.id, Some(json!(7)));
        assert_eq!(
            call.request,
            Request::ProjectOpen {
                path: "/p".to_owned()
            }
        );
    }

    #[test]
    fn a_bare_request_is_a_call_without_an_id() {
        let call = Call::parse(r#"{"method":"project.list"}"#).expect("a call");
        assert_eq!(call.id, None);
        assert_eq!(call.request, Request::ProjectList);
        let named = Call::parse(r#"{"id":"a","method":"project.list"}"#).expect("a call");
        assert_eq!(named.id, Some(json!("a")));
    }

    #[test]
    fn a_bad_line_says_why_and_keeps_the_id() {
        let (id, error) = Call::parse("{").expect_err("not JSON");
        assert_eq!((id, error.code), (None, ErrorCode::Parse));
        let (id, error) =
            Call::parse(r#"{"jsonrpc":"2.0","id":3,"method":"nope"}"#).expect_err("no such method");
        assert_eq!((id, error.code), (Some(json!(3)), ErrorCode::Invalid));
        assert!(error.message.contains("nope"), "{}", error.message);
        let (_, error) =
            Call::parse(r#"{"jsonrpc":"1.0","method":"version"}"#).expect_err("wrong version");
        assert_eq!(error.code, ErrorCode::Invalid);
        let (_, error) = Call::parse(r#"{"id":{},"method":"version"}"#).expect_err("bad id");
        assert_eq!(error.code, ErrorCode::Invalid);
    }

    #[test]
    fn a_reply_carries_the_id_and_an_error_its_code_twice() {
        let ok = Message::Reply {
            id: Some(json!(1)),
            response: Response::Result(Reply::Done(Done {})),
        };
        assert_eq!(
            ok.to_value(),
            json!({ "jsonrpc": "2.0", "id": 1, "result": {} })
        );
        let refused = Message::Reply {
            id: None,
            response: Response::Error(ApiError::new(ErrorCode::NotOpen, "/p is not open")),
        };
        assert_eq!(
            refused.to_value(),
            json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": {
                    "code": -32001,
                    "message": "/p is not open",
                    "data": { "code": "notOpen" }
                }
            })
        );
    }

    #[test]
    fn an_event_is_a_notification_named_after_its_tag() {
        let event = Message::Event(Event::ExportDone {
            job: "j1".to_owned(),
            path: "/p".to_owned(),
            output: "/o.mp4".to_owned(),
            width: 16,
            height: 9,
        });
        assert_eq!(
            event.to_value(),
            json!({
                "jsonrpc": "2.0",
                "method": "export.done",
                "params": {
                    "job": "j1", "path": "/p", "output": "/o.mp4", "width": 16, "height": 9
                }
            })
        );
    }
}
