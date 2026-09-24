<!-- https://github.com/jub0t/Concat/issues/123 -->

# Concat developer docs

**In one line:** anything the Concat window can do to a project, your program
can do through the Concat API, over JSON-RPC or gRPC.

## Pick a page

| I want to… | Go to |
|---|---|
| See a working example in 30 seconds | [Quick start](#quick-start) below |
| Understand the model: sessions, jobs, errors | [API overview](api/overview.md) |
| Look up a method and its reply | [Methods](api/methods.md) |
| Make an edit (add a clip, trim, title, effect) | [Edit commands](api/edits.md) |
| Know what a clip / project / package looks like as JSON | [Types](api/types.md) |
| Script Concat over stdin or a socket | [JSON-RPC](transports/json-rpc.md) |
| Use generated clients over HTTP/2 | [gRPC](transports/grpc.md) |
| Connect an AI agent (Model Context Protocol) | [MCP](transports/mcp.md) |
| Copy-paste a whole script | [Recipes](recipes.md) |

## Quick start

**1. Run one call.** From a source checkout:

```sh
cd src
cargo run -p concat-cli -- api '{"method":"version"}'
```

```json
{"jsonrpc":"2.0","id":null,"result":{"apiVersion":"0.2","concat":"0.2.4","dirs":{"config":"…","data":"…"},"capabilities":["events"]}}
```

**2. Make a video.** Put these lines in `edit.jsonl` and pipe them in with
`cargo run -p concat-cli -- api < edit.jsonl`:

```jsonl
{"jsonrpc":"2.0","id":1,"method":"project.create","params":{"location":"/edits","name":"Reel"}}
{"jsonrpc":"2.0","id":2,"method":"media.import","params":{"path":"/edits/Reel","file":"/footage/take1.mp4"}}
{"jsonrpc":"2.0","id":3,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"addClipAtFirstFree","mediaId":"m1","start":0}}}
{"jsonrpc":"2.0","id":4,"method":"edit.apply","params":{"path":"/edits/Reel","command":{"op":"addTextClip","start":1,"style":{"content":"Hello"}}}}
{"jsonrpc":"2.0","id":5,"method":"export.run","params":{"path":"/edits/Reel","output":"/edits/reel.mp4"}}
```

**3. Watch it finish.** The export answers at once with a job name, then
reports as events until it is done:

```jsonl
{"jsonrpc":"2.0","id":5,"result":{"job":"j1","path":"/edits/Reel","output":"/edits/reel.mp4"}}
{"jsonrpc":"2.0","method":"export.progress","params":{"job":"j1","path":"/edits/Reel","frame":30,"total":150,"stage":"video"}}
{"jsonrpc":"2.0","method":"export.done","params":{"job":"j1","path":"/edits/Reel","output":"/edits/reel.mp4","width":1920,"height":1080}}
```

> [!TIP]
> The same lines work over a socket. `concat-cli serve` listens on
> `127.0.0.1:7420`, and the window's **Settings › Remote** page serves the
> same API while the editor is open. See [JSON-RPC](transports/json-rpc.md).

## The three ways in

| Transport | How you reach it | Status |
|---|---|---|
| **JSON-RPC 2.0 lines** | `concat-cli api` (stdin), `concat-cli serve` (TCP or Unix socket), the window's Remote page | ✅ Shipped |
| **gRPC** over HTTP/2 | `concat-cli serve --grpc`, in a build with the `grpc` feature | ✅ Shipped, behind a feature |
| **MCP** | An MCP server exposing the API as tools | 🚧 Not in this tree yet. [MCP](transports/mcp.md) says what exists and how to bridge today |

All three carry the **same methods, payloads and errors**. A method added
to the API reaches every transport without a change to any of them.

## Five things to know

1. **Call `version` first.** Its `capabilities` list says what this build
   serves.
2. **Projects are folders.** Every method names a project by its folder
   path, and the folder must be open (`project.open`) first.
3. **Edits are commands.** `edit.apply` carries the window's own edit
   vocabulary, unchanged. Same clamps, same refusals.
4. **Exports are jobs.** `export.run` returns at once; progress comes as
   events; one export runs at a time.
5. **Errors have codes.** Branch on `code` (`notOpen`, `refused`, `busy`…);
   show `message` to a person.

## Where the code is

| Piece | Path |
|---|---|
| The contract: methods, payloads, events, errors | `src/crates/concat-api/src/message.rs` |
| The dispatcher that runs them | `src/crates/concat-api/src/lib.rs` |
| The JSON-RPC envelope | `src/crates/concat-api/src/rpc.rs` |
| The socket server: JSON-RPC lines, gRPC, tokens | `src/crates/concat-server/` |
| The gRPC service definition | `src/crates/concat-server/proto/concat.proto` |
| The CLI: `api` and `serve` | `src/crates/concat-cli/src/main.rs` |
| The edit commands | `src/crates/concat-project/src/commands/mod.rs` |

Related reading: [`ARCHITECTURE.md`](../ARCHITECTURE.md) places the API in
the engine.

## Licensing

Concat is AGPL-3.0-or-later. A program that talks to it over this API is a
client. A plugin built on the API may carry its own licence under
[`LICENSE-EXCEPTIONS.md`](../LICENSE-EXCEPTIONS.md).

## Keeping these pages true

These pages live with the code. A pull request that adds or changes a
method, command, event, error or transport updates the page that describes
it. The `apiVersion` a build reports is `API_VERSION` in `message.rs`; the
rules for bumping it are in the
[overview](api/overview.md#versioning-and-capabilities).
