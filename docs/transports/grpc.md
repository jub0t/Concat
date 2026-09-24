# gRPC

**In one line:** the same API over HTTP/2 with generated clients. Two
RPCs, `Call` and `Events`, carrying the API's JSON as text.

**On this page:** [Build and run](#build-and-run) · [The service](#the-service)
· [The token](#the-token) · [Errors](#errors) · [Try it with grpcurl](#try-it-with-grpcurl)
· [Clients](#clients) · [Why an envelope](#why-an-envelope-and-not-a-second-schema)

---

## Build and run

gRPC is behind the `grpc` feature. It is off by default because it brings
tokio and hyper, which nothing else in the workspace needs. CI builds and
tests it on every change.

```sh
cd src
cargo run -p concat-cli --features grpc -- serve --grpc 127.0.0.1:7421
```

```
Concat API 0.2: gRPC on 127.0.0.1:7421
Concat API 0.2: token 3f9c1e…
```

`--grpc` combines with `--json` and `--socket`; all callers share one
dispatcher. A build without the feature refuses `--grpc` with a message
saying so.

> [!NOTE]
> The window's Remote page serves JSON-RPC only. For gRPC, run
> `concat-cli serve`.

---

## The service

`src/crates/concat-server/proto/concat.proto`, package `concat.v1`:

```proto
service Concat {
  rpc Call   (Request)       returns (Response);      // one call, one response
  rpc Events (EventsRequest) returns (stream Event);  // every job's events, while open
}

message Request  { string method = 1; string params = 2; }   // params: JSON text, or empty
message Response { oneof outcome { string result = 1; Error error = 2; } }
message Error    { string code = 1; int32 number = 2; string message = 3; }
message EventsRequest {}
message Event    { string event = 1; string params = 2; }    // params: JSON text
```

| Field | Holds |
|---|---|
| `Request.method` | The method name, e.g. `"edit.apply"` |
| `Request.params` | The params object as JSON text. Empty for a method with none |
| `Response.result` | The reply as JSON text: exactly what a JSON-RPC `result` holds |
| `Response.error` | `code` is the API's name (`notOpen`, …), `number` its JSON-RPC number, `message` the sentence |
| `Event.event` | `"export.progress"`, … |
| `Event.params` | Its fields as JSON text |

Everything in [Methods](../api/methods.md), [Edit commands](../api/edits.md)
and [Types](../api/types.md) applies unchanged. Parse `result` and
`params` with your JSON library.

---

## The token

Every call carries the server's token as bearer metadata:

```
authorization: Bearer 3f9c1e…
```

Missing or wrong: the RPC fails with status `UNAUTHENTICATED`. This
applies to `Events` too. The comparison is constant-time. Where the token
comes from is in [JSON-RPC → The token](json-rpc.md#the-token).

---

## Errors

Two layers, deliberately kept apart:

| Layer | Means | Looks like |
|---|---|---|
| gRPC status | the transport refused | `UNAUTHENTICATED` (token), `INTERNAL` (the server could not run the call at all) |
| `Response.error` | the API refused | `{code: "notOpen", number: -32001, message: "…"}` with the call itself returning `OK` |

So a `Call` that returns `OK` still needs its `outcome` checked.

---

## Try it with grpcurl

There is no reflection service; hand grpcurl the proto.

```sh
PROTO=src/crates/concat-server/proto/concat.proto
TOKEN=3f9c1e…

grpcurl -plaintext -proto $PROTO -H "authorization: Bearer $TOKEN" \
  -d '{"method":"version","params":""}' \
  127.0.0.1:7421 concat.v1.Concat/Call
```

```json
{"result": "{\"apiVersion\":\"0.2\",\"concat\":\"0.2.4\",\"dirs\":{…},\"capabilities\":[\"events\",\"grpc\"]}"}
```

An edit:

```sh
grpcurl -plaintext -proto $PROTO -H "authorization: Bearer $TOKEN" \
  -d '{"method":"edit.apply","params":"{\"path\":\"/edits/Reel\",\"command\":{\"op\":\"addTrack\"}}"}' \
  127.0.0.1:7421 concat.v1.Concat/Call
```

Events, streamed until you stop it:

```sh
grpcurl -plaintext -proto $PROTO -H "authorization: Bearer $TOKEN" \
  -d '{}' 127.0.0.1:7421 concat.v1.Concat/Events
```

---

## Clients

### Rust

The generated client ships in the crate under the feature:

```rust
use concat_server::grpc::proto::{concat_client::ConcatClient, Request as Call, EventsRequest};
use tonic::Request;

let mut client = ConcatClient::connect("http://127.0.0.1:7421").await?;
let mut request = Request::new(Call { method: "version".into(), params: String::new() });
request.metadata_mut().insert("authorization", format!("Bearer {token}").parse()?);
let response = client.call(request).await?.into_inner();
```

### Python

```sh
pip install grpcio grpcio-tools
python -m grpc_tools.protoc -I src/crates/concat-server/proto \
  --python_out=. --grpc_python_out=. concat.proto
```

```python
import json, grpc, concat_pb2, concat_pb2_grpc

channel = grpc.insecure_channel("127.0.0.1:7421")
stub = concat_pb2_grpc.ConcatStub(channel)
meta = [("authorization", f"Bearer {TOKEN}")]

def call(method, **params):
    req = concat_pb2.Request(method=method, params=json.dumps(params) if params else "")
    res = stub.Call(req, metadata=meta)
    if res.HasField("error"):
        raise RuntimeError(f"{res.error.code}: {res.error.message}")
    return json.loads(res.result)

print(call("version")["capabilities"])
for event in stub.Events(concat_pb2.EventsRequest(), metadata=meta):
    print(event.event, json.loads(event.params))
```

### Anything else

Any language with a protobuf and gRPC toolchain: generate from
`concat.proto`, send the bearer, parse the JSON text.

---

## Why an envelope and not a second schema

The protobuf carries JSON text rather than typed messages on purpose:

- A method added to the API reaches gRPC with **no change** to the proto.
- There is **one set of docs** for every transport.
- Both transports parse requests through the same code, so they refuse
  the same things the same way.

What gRPC adds over the line transport is HTTP/2, generated clients, and
a streamed reply for events.
