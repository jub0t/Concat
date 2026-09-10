# Concat MCP Server

Drive the Concat editor from any AI agent over the **Model Context Protocol**.
There are two ways in, and both speak the same tools:

1. **Built in (recommended): the released Concat binary IS the MCP server.**
   Launch the editor once with `--remote-control` (or `CONCAT_REMOTE=1`) and,
   beside the line-JSON socket, it serves MCP's Streamable-HTTP transport on
   `http://127.0.0.1:<port>/mcp` (loopback only; the port lands in
   `remote-mcp` in the config directory and the log line prints it). Point
   any HTTP-capable MCP client at that URL - no Python, no helper process:

   ```
   AI agent ⇄ (MCP over HTTP) ⇄ concat --remote-control ⇄ concat_api ⇄ engine
   ```

2. **The Python bridge (this file's directory).** A thin stdio bridge that
   translates MCP tool calls into [Concat API](../engine/crates/concat-api/)
   requests and pipes them to `concat-cli api`, the engine's line-JSON
   transport - or, when it finds one, to the running window's remote-control
   socket. It owns no editing meaning of its own - every operation is
   exactly what the Concat window would do, with the same clamps and the
   same refusals. Same story without a window:

   ```
   AI agent ⇄ (MCP over stdio) ⇄ concat_mcp.py ⇄ (line-JSON) ⇄ concat-cli api ⇄ concat_api ⇄ engine
   ```

Both surfaces expose the identical tool table (below), so an agent's habits
transfer unchanged between them.

## Direct connect (built-in HTTP, no Python)

```sh
concat --remote-control        # the released binary, any OS
```

The log prints e.g. `concat: MCP over HTTP on http://127.0.0.1:44945/mcp` and
the URL is written to `~/.config/app.concat.editor/remote-mcp` (platform
equivalents on macOS/Windows). Register that URL with an MCP client that
speaks Streamable HTTP:

```json
{
  "mcpServers": {
    "concat": {
      "type": "http",
      "url": "http://127.0.0.1:44945/mcp"
    }
  }
}
```

Transport notes: one `POST /mcp` per message, JSON responses, notifications
answered with 202; stateless (no session id), so restarts of either side need
no re-registration beyond a changed port. The endpoint is loopback-only and
unauthenticated - the ephemeral port is the secret - and any `Origin` header
that is not loopback is refused (the MCP spec's DNS-rebinding guard).

## Setup

Build the engine once:

```sh
cd engine && cargo build -p concat-cli
```

The bridge finds the binary automatically (`engine/target/debug/concat-cli`, then `release`, then `$PATH`); override with `CONCAT_CLI="/path/to/concat-cli"`.

Register it with your MCP client, e.g. Claude Desktop / any MCP-capable agent:

```json
{
  "mcpServers": {
    "concat": {
      "command": "python3",
      "args": ["/absolute/path/to/Concat/mcp/concat_mcp.py"]
    }
  }
}
```

Requires Python 3.8+ (standard library only, nothing to install).

## Tools

| Tool | Maps to Concat API | Notes |
|---|---|---|
| `concat_version` | `version` | API version + build info |
| `project_create` | `project.create` | Creates and opens; default 1080p30 |
| `project_open` | `project.open` | |
| `project_get` | `project.get` | Full state incl. every clip's id |
| `project_save` | `project.save` | |
| `project_list` | `project.list` | Recent projects |
| `media_probe` | `media.probe` | Duration/streams of a file, no project needed |
| `media_import` | `media.import` | Probe + add to bin; returns minted media id |
| `catalogue` | `catalogue.list` | Effect packages with parameter ranges |
| `edit` | `edit.apply` | **The full vocabulary** — any `Command`, unchanged |
| `edit_undo` / `edit_redo` | `edit.undo` / `edit.redo` | |
| `export_video` | `export.run` | H.264 MP4; progress in the tool result |
| `preview_frame` | `preview.frame` | True composited frame as PNG |
| `template_list` | `template.list` | |

## The edit vocabulary (`edit` tool)

Edits are `concat-project` [`Command`]s carried verbatim — one JSON object tagged by `"op"`, camelCase fields, positions in seconds. The AI never needs internals: `project_get` returns the ids, `catalogue` returns the effect ids and parameter ranges, and refusals arrive as the sentence the window would show.

```json
{"op": "addTextClip", "trackId": null, "start": 0.0, "duration": 3.0,
 "style": {"content": "Hello", "fontSize": 0.08, "color": "#ffffff"}}

{"op": "addClip", "mediaId": "m1", "trackId": null, "start": 0.0}

{"op": "splitClips", "clipIds": ["c1"], "time": 2.5}

{"op": "trimClip", "clipId": "c1", "edge": "start", "delta": 0.5}

{"op": "moveClips", "moves": [{"clipId": "c1", "trackId": "t2", "start": 4.0}]}

{"op": "batch", "commands": ["..."]}   // atomic multi-op, one undo step
```

A typical AI session: `project_create` → `media_import` (get `m` id) → `edit` (`addClip`) → `edit` (`addTextClip`) → `preview_frame` to look → `export_video`.

The authoritative operation list is the [`Command` enum](../engine/crates/concat-project/src/commands.rs); new operations added there are usable through this bridge with no changes here.

## State awareness for AI callers

The guard model converts wrong-order mistakes into recoverable, self-explanatory errors - never corruption. The bridge adds three layers of context awareness on top:

- **`workflow_state`** - one glance at everything (open project, bin contents, clips with ids, undo availability). Call it whenever unsure; it needs no arguments.
- **Error HINTs** - refusals arrive with the next move attached (e.g. "No project is open" → run `project_create`/`project_open`).
- **`add_media_to_timeline`** - the common import-then-place sequence as one call.

A session also remembers the last project it created or opened, so `workflow_state` works without arguments. Unknown clip/media ids are **tolerated silent no-ops** by design (the tolerance doctrine) - after edits, confirm with `workflow_state`.

## Driving the running window (real-time GUI)

The bridge can drive a *running* window instead of spawning the headless CLI: launch the editor once with `--remote-control` (or `CONCAT_REMOTE=1`), and it opens `127.0.0.1:<port>` gated by a token, both written to the config directory as `remote-port` / `remote-token`. The bridge discovers them automatically - if the window is up, `project.create`, `edit`, `export_video` and friends act on it and the GUI updates in real time, exactly as if the user had made the changes. `CONCAT_SOCKET` / `CONCAT_TOKEN` override the discovery.

Notes: one project at a time (the window's own session - a caller naming another open path is told what *is* open); edits land on the window's undo stack; `export_video` honours the window's export slot and blocks until the file is written. `mcp/test_window.py` walks this flow against a live window. Templates are not available over remote control yet.

## Windows

`concat-cli` has no window, no Slint/Skia and no speech stack, so the Windows build only needs:

1. **Rust** via [rustup](https://rustup.rs) — `engine/rust-toolchain.toml` pins the toolchain; any `cargo` command inside `engine/` installs it on first use.
2. **Visual Studio Build Tools** with the "Desktop development with C++" workload (MSVC linker + Windows SDK).
3. **LLVM/libclang** for bindgen: `choco install llvm`, then set `LIBCLANG_PATH=C:\Program Files\LLVM\bin`.
4. **FFmpeg shared dev build** — the [BtbN](https://github.com/BtbN/FFmpeg-Builds/releases) `win64-gpl-shared` zip. Unpack it and set either `FFMPEG_DIR` to its root or `FFMPEG_INCLUDE_DIR` + `FFMPEG_LIBS_DIR` (see `engine/.cargo/config.toml`). Keep its `bin/` on `PATH` so the FFmpeg DLLs resolve at run time.

```powershell
cd engine
cargo build -p concat-cli --release
```

The bridge finds `engine\target\release\concat-cli.exe` automatically. Point your MCP client at Python (adjust the Python path as installed):

```json
{
  "mcpServers": {
    "concat": {
      "command": "python",
      "args": ["C:\\path\\to\\Concat\\mcp\\concat_mcp.py"]
    }
  }
}
```

Run the same test suite: `python mcp\test_mcp.py`. The Linux-only ONNX Runtime/glibc workarounds do not apply on Windows — `ort-sys` links its MSVC prebuilt archive out of the box. Building the full editor window additionally needs `cargo run --release -p concat` (Skia binaries download automatically; keep `cmake` + `Ninja` handy for the speech stack, which the CLI does not pull in).

## Licensing

The bridge touches Concat only via the Concat API, so it falls under the [Concat Plugin Exception](../LICENSE-EXCEPTIONS.md): it may carry its own licence. The code here is AGPL-3.0-or-later, matching the rest of the repository.
