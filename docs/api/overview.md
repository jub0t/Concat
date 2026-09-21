# API overview

**In one line:** one dispatcher answers every request; transports only carry
JSON to it and back.

**On this page:** [Three shapes](#three-shapes) · [Sessions](#sessions) ·
[Projects on disk](#projects-on-disk) · [Jobs and events](#jobs-and-events) ·
[Errors](#errors) · [Versioning](#versioning-and-capabilities) ·
[Tolerance](#tolerance-and-refusal) · [Concurrency](#concurrency) ·
[Embedding in Rust](#embedding-in-rust) · [App directories](#app-directories)

---

## Three shapes

Everything that crosses the API is one of three JSON objects.

| Shape | Looks like | Direction |
|---|---|---|
| **Request** | `{"method": "project.open", "path": "…"}` | in |
| **Response** | `{"result": …}` or `{"error": {"code": "…", "message": "…"}}` | out, one per request |
| **Event** | `{"event": "export.progress", "job": "j1", …}` | out, any time a job has news |

- Methods are named `area.verb`: `project.open`, `edit.apply`, `export.run`.
- A result is the method's payload **alone**. Nothing wraps it.
- Field names are **camelCase** everywhere: requests, replies, the project
  document, the edit commands.

On stdin or a socket the three travel inside the JSON-RPC 2.0 envelope,
which adds an `id` and nothing else. See [JSON-RPC](../transports/json-rpc.md).
gRPC carries the same JSON as text inside a protobuf message. See
[gRPC](../transports/grpc.md).

---

## Sessions

A **session** is one open project. The API keeps one per project folder.

- Every project method takes `path`, the project **folder**.
- The folder must be open. A method on a closed folder is a `notOpen`
  error, never a silent open.
- `project.open` on a folder already open returns its state as it stands,
  edits and all.
- The path is canonicalised, so `.` and its absolute spelling are one
  session.

A session holds the **edit history**:

| Method | Does |
|---|---|
| `edit.apply` | applies one command, records one undo step |
| `edit.undo` / `edit.redo` | walk the history |
| `project.save` | writes `concat.json` |
| `project.close` | drops the session; saves first only if `save: true` |

> [!WARNING]
> `project.close` without `save: true` drops unsaved edits, the same as the
> window's "Don't save".

On a socket, **all connected callers share the sessions**. A project one
caller opens is open for the others.

---

## Projects on disk

A project is a **folder** holding `concat.json`.

`project.create`:

1. makes `<location>/<name>` (characters a filesystem refuses become `-`),
2. writes the manifest,
3. opens a session,
4. puts it at the front of the recents list.

It refuses a folder that already holds a project.

| To read the project as… | Call |
|---|---|
| the model the engine holds | `project.get` |
| the exact bytes a save writes | `project.document` |

Both shapes are in [Types](types.md).

---

## Jobs and events

A request that would take long runs as a **job**. Today that is
`export.run`.

1. The response names the job at once: `{"job": "j1", …}`.
2. The work runs on its own thread.
3. Progress arrives as **events**, each carrying `job` and `path`.
4. The job ends with exactly one of `export.done` or `export.failed`.

| Event | When |
|---|---|
| `cutout.progress` | masks for an automatic cutout are being found before the render; a model may download first |
| `export.progress` | `frame` of `total`, in stage `video`, `audio` or `mux` |
| `export.done` | the file was written |
| `export.failed` | the file was not written; `error.code` is `cancelled` if you cancelled it |

Rules:

- **One export at a time.** A second `export.run` is refused with `busy`.
- Job names are unique for the life of the dispatcher: `j1`, `j2`, …
- On a socket, events go to **every** connected caller. Filter by `job`.
- On stdin, the process waits for running jobs before it exits, so a job
  begun on the last line still finishes.

---

## Errors

An error has a `code` you branch on and a `message` you can show a person.

| Code | JSON-RPC number | Meaning |
|---|---|---|
| `parse` | -32700 | The line was not JSON |
| `invalid` | -32600 | Not a request: unknown method, missing or ill-typed field, value out of range |
| `notOpen` | -32001 | That project folder is not open |
| `notFound` | -32002 | That job, template or package does not exist |
| `refused` | -32003 | The edit layer said no. Nothing changed. The message is the window's own sentence |
| `busy` | -32004 | A one-at-a-time job is already running |
| `cancelled` | -32005 | The job was stopped by `export.cancel`. Only ever inside `export.failed` |
| `unauthorized` | -32006 | Wrong or missing token. Raised by the socket transports, never by the API itself |
| `failed` | -32000 | Everything else: a file that would not read or write, a decode that failed, a model that did not download |

In the JSON-RPC envelope the number is `error.code` and the name is
`error.data.code`. Over gRPC both are fields of the `Error` message.

---

## Versioning and capabilities

**Call `version` first.** Its reply has:

| Field | Meaning |
|---|---|
| `apiVersion` | The contract this build speaks. Currently `0.2` |
| `concat` | The build's own version |
| `dirs` | The config and data directories (see [below](#app-directories)) |
| `capabilities` | What this build serves, as names to test for |

Capability names:

| Name | Means |
|---|---|
| `events` | Jobs report through events. Every build has it |
| `gpu` | Frames composite on a GPU. Depends on the machine |
| `json-rpc` | Listening for JSON-RPC lines on TCP |
| `unix-socket` | Listening for JSON-RPC lines on a Unix socket |
| `grpc` | Listening for gRPC |

When `apiVersion` changes:

- ✅ Bumped when a method's **shape changes** in a way an old caller would
  misread.
- ❌ Not bumped for a new method, a new optional field, a new event or a
  new capability. Ignore names you do not know.

---

## Tolerance and refusal

The edit layer is forgiving about **state** and strict about **shape**.

| Situation | Outcome |
|---|---|
| An edit names a clip, track or media that no longer exists | Usually a tolerated **no-op**: success, nothing changed, no undo step. Each command in [Edit commands](edits.md) says which |
| A number is outside its range | **Clamped**, not refused. Scale 100 lands at 8; start -2 lands at 0 |
| A number is NaN or infinite | `refused` |
| A field has the wrong type, or the `op` / `method` is unknown | `invalid` |
| A file cannot be probed, read or written | `failed` |

---

## Concurrency

- The dispatcher handles **one request at a time**, in order, like the
  window's event loop.
- Long work is a job, so a request never waits on a render.
- The socket server queues every caller through that one thread.
- Responses on one connection come back in the order the requests were
  sent. The envelope's `id` lets you tell them apart anyway.

---

## Embedding in Rust

The dispatcher and the server are ordinary workspace crates.

```rust
use std::sync::Arc;
use concat_api::{Api, Event, Request, Response};

let mut api = Api::new(Arc::new(|event: Event| {
    println!("{}", serde_json::to_string(&event).unwrap());
}))?;
let response: Response = api.dispatch(Request::ProjectList);
api.finish(); // wait for running jobs before dropping it
```

| Need | Use |
|---|---|
| Your own config and data folders (tests, embedders) | `Api::with_dirs(dirs, sink)` |
| The API on sockets, with a token | `concat_server::Server::start(config, Api::new)` |
| Calling the served API from the same process | `Server::hub()` |

`Api` is not `Send`: it lives on one thread. The event sink is called from
job threads, so it must be `Send + Sync`. The window's Remote page is a
`Server::start` from the settings sheet.

---

## App directories

`version` reports two folders. Recents, settings and templates live in
`config`; downloaded models and painted titles in `data`.

| Platform | `config` | `data` |
|---|---|---|
| macOS | `~/Library/Application Support/app.concat.editor` | same folder |
| Windows | `%APPDATA%\app.concat.editor` | same folder |
| Linux | `~/.config/app.concat.editor` (or `$XDG_CONFIG_HOME/…`) | `~/.local/share/app.concat.editor` (or `$XDG_DATA_HOME/…`) |
| Portable build | `portable/` beside the executable | same folder |
