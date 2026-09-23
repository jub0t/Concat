# MCP

**In one line:** an MCP server for Concat is planned and prototyped, not
shipped. This page says what exists, what it should look like, and how to
connect an agent today.

**Status:** 🚧 not in this tree.

**On this page:** [What exists](#what-exists) · [The shape it should take](#the-shape-it-should-take)
· [Bridging today](#bridging-today) · [Guidance for agents](#guidance-for-agents)

---

## What exists

| Thing | Where | State |
|---|---|---|
| The API an MCP server would expose | `concat-api` | ✅ Shipped. Every method in [Methods](../api/methods.md) |
| A socket server with a token and event fan-out | `concat-server` | ✅ Shipped. [JSON-RPC](json-rpc.md), [gRPC](grpc.md) |
| A prototype: the window serving MCP over Streamable HTTP with 17 tools, plus a stdio Python bridge | [PR #69](https://github.com/jub0t/Concat/pull/69) | ❌ Closed, not merged. It re-implemented the transport instead of reusing `concat-server` |
| The request | [issue #95 "MCP support"](https://github.com/jub0t/Concat/issues/95) | Closed on 2026-09-19 with a pointer to the documentation issue [#123](https://github.com/jub0t/Concat/issues/123) |

The design direction, from the September 2026 audit: build the MCP server
on the official Rust SDK (`rmcp`) over `concat-server`'s `Hub`, so it is a
third transport beside JSON-RPC and gRPC and not a second API.

---

## The shape it should take

The API is already close to a tool set. The intended mapping:

| MCP concept | Concat |
|---|---|
| A **tool** | One API method. `project.open`, `edit.apply`, `export.run`, … Names, params and results as in [Methods](../api/methods.md) |
| A tool's **input schema** | The method's params. For `edit.apply`, the `command` union in [Edit commands](../api/edits.md) |
| A tool **result** | The reply payload as JSON; a `preview.frame` reply as an image content block |
| A **notification** / progress | The job events: `export.progress`, `export.done`, `export.failed`, `cutout.progress` |
| A **resource** | `concat://project/<path>` for `project.get`; the catalogue as `concat://catalogue` |
| **Auth** | The same token `concat-server` mints, as a bearer |

Principles that carry over from the other transports:

- **No new meaning in the transport.** A tool calls a method; it does not
  interpret the edit.
- **Refusals are the window's sentences.** An agent that hits `refused`
  can show or reason about the message as it is.
- **Discover, don't assume.** `version` → `capabilities`, and
  `catalogue.list` → what effects exist, before building a chain.

---

## Bridging today

Until the server ships, an agent reaches Concat through a small MCP
server that wraps the JSON-RPC socket. Three moving parts:

1. `concat-cli serve` (or the window's Remote page), listening on
   `127.0.0.1:7420` with a token.
2. A stdio MCP server, spawned by the agent's host, that opens one socket
   connection and turns tool calls into `call(method, params)`.
3. Tool definitions that mirror the methods.

A sketch in Python, using the `Concat` class from
[JSON-RPC → Python client](json-rpc.md#a-minimal-python-client) and the
`mcp` package:

```python
# concat_mcp.py  — run with: python concat_mcp.py  (the host spawns it over stdio)
import os
from mcp.server.fastmcp import FastMCP
from concat_client import Concat   # the class from the JSON-RPC page

api = Concat(port=int(os.environ.get("CONCAT_PORT", 7420)), token=os.environ["CONCAT_API_TOKEN"])
mcp = FastMCP("concat")

@mcp.tool()
def version() -> dict:
    """What this Concat build serves."""
    return api.call("version")

@mcp.tool()
def project_open(path: str) -> dict:
    """Open a project folder. Returns the editor view."""
    return api.call("project.open", path=path)

@mcp.tool()
def project_create(location: str, name: str) -> dict:
    """Create a project folder under `location` and open it."""
    return api.call("project.create", location=location, name=name)

@mcp.tool()
def media_import(path: str, file: str) -> dict:
    """Probe a file and add it to the project's bin."""
    return api.call("media.import", path=path, file=file)

@mcp.tool()
def edit_apply(path: str, command: dict) -> dict:
    """Apply one edit command ({"op": ..., ...}). See docs/api/edits.md."""
    return api.call("edit.apply", path=path, command=command)

@mcp.tool()
def catalogue_list(kind: str | None = None) -> list:
    """Effect packages and their parameters."""
    return api.call("catalogue.list", **({"kind": kind} if kind else {}))

@mcp.tool()
def export_run(path: str, output: str) -> dict:
    """Render the timeline to a file and wait for it to finish."""
    started = api.call("export.run", path=path, output=output)
    return api.wait_for_job(started["job"])["params"]

if __name__ == "__main__":
    mcp.run()
```

Register it with the host as a stdio server with `CONCAT_API_TOKEN` in
its environment. The exact registration is the host's; the server above
speaks standard MCP over stdio.

> [!NOTE]
> This is a bridge, not the product. It has one connection, no streaming
> progress, and a hand-picked tool list. The shipped server will expose
> every method and stream events.

---

## Guidance for agents

If you are an agent reading this because you have been pointed at Concat:

1. Call `version`. Check `apiVersion` starts with `0.2` and read
   `capabilities`.
2. Open or create a project. Keep its **folder path**; every call needs
   it.
3. Import media with `media.import`. The reply's `createdId` is the media
   id.
4. Place clips with `addClipAtFirstFree` or `addClip`. Read the returned
   `EditorView` for clip ids; do not guess them.
5. Prefer one `batch` for a sequence of edits that belong together.
6. Export with `export.run`, then wait for `export.done` or
   `export.failed` for **that job**. Do not start a second export
   meanwhile; it is refused with `busy`.
7. Treat a `refused` message as the reason, in plain words. Nothing
   changed.
8. Save with `project.save` before `project.close`, or pass `save: true`.

[Recipes](../recipes.md) has complete scripts for each of these.
