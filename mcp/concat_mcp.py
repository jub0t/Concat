#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jareer and Concat contributors
"""Concat MCP server: drives the Concat editor over the Concat API.

A stdio MCP (Model Context Protocol) server that translates MCP tool calls
into Concat API requests and pipes them to `concat-cli api`, the engine's
line-JSON transport. Every operation the Concat window can perform is
reachable here; the bridge adds no editing meaning of its own, it only
carries what `concat_api` defines.

Protocol: newline-delimited JSON-RPC 2.0 on stdin/stdout (the MCP stdio
transport). Diagnostics go to stderr. Python 3.8+, standard library only.
"""

import json
import os
import shlex
import shutil
import socket
import subprocess
import sys
from pathlib import Path

PROTOCOL_VERSION = "2024-11-05"
SERVER_INFO = {"name": "concat-mcp", "version": "0.1.0"}

# ---------------------------------------------------------------------------
# Engine process management
# ---------------------------------------------------------------------------


def engine_command():
    """Where the concat-cli binary lives: $CONCAT_CLI, then the repo's
    debug/release build, then PATH."""
    env = os.environ.get("CONCAT_CLI")
    if env:
        return shlex.split(env)
    repo = Path(__file__).resolve().parent.parent / "engine" / "target"
    for profile in ("debug", "release"):
        binary = "concat-cli.exe" if os.name == "nt" else "concat-cli"
        candidate = repo / profile / binary
        if candidate.exists():
            return [str(candidate)]
    found = shutil.which("concat-cli")
    if found:
        return [found]
    raise FileNotFoundError(
        "concat-cli not found - set CONCAT_CLI or build it: cd engine && cargo build -p concat-cli"
    )


class Engine:
    """One long-lived `concat-cli api` process.

    The Concat API is strictly one request in flight at a time: a dispatch
    blocks until its response is ready, with any events flushed line by line
    ahead of it. This wrapper mirrors that: `call` sends one request, reads
    lines until a response arrives, and returns (reply, events).
    """

    def __init__(self, command):
        self.command = command
        self.proc = None

    def _spawn(self):
        cmd = self.command + ["api"]
        log("spawning engine: %s" % " ".join(cmd))
        self.proc = subprocess.Popen(
            cmd,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )

    def _alive(self):
        return self.proc is not None and self.proc.poll() is None

    def call(self, request):
        if not self._alive():
            self._spawn()
        line = json.dumps(request, ensure_ascii=False)
        log("-> %s" % line[:300])
        try:
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()
        except (BrokenPipeError, ValueError):
            # Engine died between calls; one clean retry on a fresh process.
            log("engine stdin broken; respawning")
            self._spawn()
            self.proc.stdin.write(line + "\n")
            self.proc.stdin.flush()

        events, response = [], None
        while response is None:
            out = self.proc.stdout.readline()
            if out == "":
                raise RuntimeError(
                    "concat-cli exited (code %s) while handling %s"
                    % (self.proc.returncode, request.get("method", "?"))
                )
            out = out.strip()
            if not out:
                continue
            message = json.loads(out)
            if "result" in message or "error" in message:
                response = message
            else:
                events.append(message)  # an Event: export progress and friends
        return response, events


class SocketEngine:
    """Connects to the running window's remote-control socket. A fresh
    connection per request: stateless, and a restarted window is picked up
    on the next call with no bookkeeping here."""

    def __init__(self, address, token):
        self.address = address
        self.token = token
        self.proven = False

    def window_state(self):
        """One-shot window-state probe: fresh connection, state reply."""
        connection = socket.create_connection(
            (self.address.rpartition(":")[0] or "127.0.0.1",
             int(self.address.rpartition(":")[2])), timeout=10)
        try:
            reader = connection.makefile("r", encoding="utf-8")
            writer = connection.makefile("w", encoding="utf-8")
            writer.write(self.token + "\n")
            writer.write(json.dumps({"window-state": True}) + "\n")
            writer.flush()
            while True:
                line = reader.readline()
                if line == "":
                    return None
                message = json.loads(line)
                ws = message.get("result", {}).get("windowState")
                if ws is not None:
                    return ws
        except (OSError, ValueError, json.JSONDecodeError):
            return None
        finally:
            connection.close()

    def call(self, request):
        host, _, port = self.address.rpartition(":")
        connection = socket.create_connection((host or "127.0.0.1", int(port)), timeout=10)
        try:
            reader = connection.makefile("r", encoding="utf-8")
            writer = connection.makefile("w", encoding="utf-8")
            writer.write(self.token + "\n")
            writer.write(json.dumps(request, ensure_ascii=False) + "\n")
            writer.flush()
            events, response = [], None
            while response is None:
                out = reader.readline()
                if out == "":
                    raise RuntimeError("the window closed the remote-control connection")
                message = json.loads(out)
                if "result" in message or "error" in message:
                    response = message
                else:
                    events.append(message)
            return response, events
        finally:
            connection.close()


def window_socket():
    """The running window's remote-control endpoint, when one is up:
    $CONCAT_SOCKET (with optional $CONCAT_TOKEN), else the port and token
    files the window writes into its config directory."""
    address = os.environ.get("CONCAT_SOCKET")
    if address:
        return address, os.environ.get("CONCAT_TOKEN", "")
    home = Path.home()
    candidates = [
        home / ".config" / "app.concat.editor",
        home / "Library" / "Application Support" / "app.concat.editor",
        home / "AppData" / "Roaming" / "app.concat.editor",
    ]
    for directory in candidates:
        port_file, token_file = directory / "remote-port", directory / "remote-token"
        if port_file.exists() and token_file.exists():
            return "127.0.0.1:" + port_file.read_text().strip(), token_file.read_text().strip()
    return None


class StickyEngine:
    """Chooses the backend ONCE - the running window's remote control if it
    is up, else the headless CLI - and then sticks to it for the life of
    this bridge. A session must not silently move between engines
    mid-flight: the edits live in exactly one of them, and flapping would
    split the work across two stores. If the chosen engine dies, calls fail
    loudly instead of silently switching."""

    def __init__(self, engine):
        self.engine = engine

    def call(self, request):
        try:
            return self.engine.call(request)
        except Exception as error:
            raise RuntimeError(
                f"{type(self.engine).__name__} died: {error} - restart it "
                f"(or restart this MCP session) and retry") from error


def engine():
    """The backend, re-evaluated on every call. While a window with remote
    control is up, every tool drives that window - including across window
    restarts, since a fresh window writes a fresh port. With no window up,
    tools fall back to a headless `concat-cli api`. The two never mix: the
    window answers only for the project it has open and the CLI only for
    the folder it was handed, so a switch surfaces as a loud path error,
    never as work silently split across two stores."""
    global ENGINE, LAST_PROJECT
    socket_config = window_socket()
    if socket_config:
        address, token = socket_config
        current = ENGINE.engine if isinstance(ENGINE, StickyEngine) else None
        if not (isinstance(current, SocketEngine)
                and current.address == address and current.token == token):
            log("driving the window at %s" % address)
            ENGINE = StickyEngine(SocketEngine(address, token))
            state = ENGINE.engine.window_state() or {}
            if state.get("projectPath"):
                LAST_PROJECT = state["projectPath"]
        return ENGINE
    current = ENGINE.engine if isinstance(ENGINE, StickyEngine) else None
    if not isinstance(current, Engine):
        command = engine_command()
        log("no window socket; spawning %s" % " ".join(command))
        ENGINE = StickyEngine(Engine(command))
    return ENGINE


LAST_PROJECT = None


def remember_project(arguments, outcome):
    global LAST_PROJECT
    if outcome.get("isError"):
        return
    if "name" in arguments:
        LAST_PROJECT = arguments["location"].rstrip("/") + "/" + arguments["name"]
    elif "path" in arguments:
        LAST_PROJECT = arguments["path"]


ENGINE = None
# ---------------------------------------------------------------------------
# Tool surface: a thin, stable mapping onto Concat API methods
# ---------------------------------------------------------------------------

PROJECT = {
    "type": "object",
    "properties": {
        "path": {"type": "string", "description": "Project folder path, as opened/created"},
    },
    "required": ["path"],
}

EDIT_EXAMPLES = (
    "Command vocabulary (serde tag 'op', camelCase fields), common ops:\n"
    '  {"op": "addTextClip", "trackId": null, "start": 0.0, "duration": 3.0, '
    '"style": {"content": "Hello", "fontSize": 0.08, "color": "#ffffff"}}\n'
    '  {"op": "addClip", "mediaId": "m1", "trackId": null, "start": 0.0}\n'
    '  {"op": "splitClips", "clipIds": ["c1"], "time": 2.5}\n'
    '  {"op": "trimClip", "clipId": "c1", "edge": "start", "delta": 0.5}\n'
    '  {"op": "moveClips", "moves": [{"clipId": "c1", "trackId": "t2", "start": 4.0}]}\n'
    '  {"op": "batch", "commands": [ ... ]}  (atomic multi-op)\n'
    "Unknown clip/media ids are TOLERATED SILENT NO-OPS - the call succeeds "
    "but changes nothing. After edits, verify with workflow_state. Refusals "
    "(unsaved changes, impossible moves) arrive as the sentence the window "
    "would show. project_get returns every clip's id."
)

TOOLS = [
    {
        "name": "concat_version",
        "description": "Concat API version and build info.",
        "inputSchema": {"type": "object", "properties": {}},
        "request": lambda a: {"method": "version"},
    },
    {
        "name": "project_create",
        "description": (
            "Create a new project folder and open it. Defaults to 1080p at 30fps."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "location": {"type": "string", "description": "Parent directory for the project folder"},
                "name": {"type": "string"},
                "width": {"type": "integer", "description": "Frame width, default 1920"},
                "height": {"type": "integer", "description": "Frame height, default 1080"},
                "fps": {"type": "number", "description": "Frame rate, default 30"},
            },
            "required": ["location", "name"],
        },
        "request": lambda a: {
            "method": "project.create",
            "location": a["location"],
            "name": a["name"],
            "video": {
                "width": a.get("width", 1920),
                "height": a.get("height", 1080),
                "rateNum": round(a.get("fps", 30) * 1000),
                "rateDen": 1000,
            },
        },
    },
    {
        "name": "project_open",
        "description": "Open an existing project folder.",
        "inputSchema": {
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"],
        },
        "request": lambda a: {"method": "project.open", "path": a["path"]},
    },
    {
        "name": "project_get",
        "description": (
            "Full state of an open project: timelines, tracks, every clip with "
            "its id - the ids the edit vocabulary needs."
        ),
        "inputSchema": PROJECT,
        "request": lambda a: {"method": "project.get", "path": a["path"]},
    },
    {
        "name": "project_save",
        "description": "Write the project document to its folder.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {"type": "string"},
                "name": {"type": "string", "description": "Rename the project"},
            },
            "required": ["path"],
        },
        "request": lambda a: {
            "method": "project.save",
            "path": a["path"],
            **({"name": a["name"]} if "name" in a else {}),
        },
    },
    {
        "name": "project_list",
        "description": "Projects recently opened on this machine, newest first.",
        "inputSchema": {"type": "object", "properties": {}},
        "request": lambda a: {"method": "project.list"},
    },
    {
        "name": "media_probe",
        "description": "Inspect a media file: duration, video stream, audio tracks. No project needed.",
        "inputSchema": {
            "type": "object",
            "properties": {"file": {"type": "string"}},
            "required": ["file"],
        },
        "request": lambda a: {"method": "media.probe", "path": a["file"]},
    },
    {
        "name": "media_import",
        "description": "Probe a file and add it to a project's bin (what dropping a file does). Returns the minted media id.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {"type": "string"},
                "file": {"type": "string"},
            },
            "required": ["project", "file"],
        },
        "request": lambda a: {"method": "media.import", "path": a["project"], "file": a["file"]},
    },
    {
        "name": "catalogue",
        "description": (
            "Every built-in effect package with its parameters and value ranges. "
            "kind: effect | filter | audio | transition | generator."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {"kind": {"type": "string"}},
        },
        "request": lambda a: {
            "method": "catalogue.list",
            **({"kind": a["kind"]} if "kind" in a else {}),
        },
    },
    {
        "name": "edit",
        "description": (
            "Apply one edit command to an open project. This is the full "
            "vocabulary the window has - the command rides through unchanged.\n"
            + EDIT_EXAMPLES
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {"type": "string"},
                "command": {"type": "object", "description": "One Command object, tagged by 'op'"},
            },
            "required": ["project", "command"],
        },
        "request": lambda a: {"method": "edit.apply", "path": a["project"], "command": a["command"]},
    },
    {
        "name": "edit_undo",
        "description": "Step the project's history back one edit.",
        "inputSchema": PROJECT,
        "request": lambda a: {"method": "edit.undo", "path": a["path"]},
    },
    {
        "name": "edit_redo",
        "description": "Step the project's history forward one edit.",
        "inputSchema": PROJECT,
        "request": lambda a: {"method": "edit.redo", "path": a["path"]},
    },
    {
        "name": "export_video",
        "description": (
            "Render an open project to an MP4 file (H.264), exactly as the "
            "window's Export does. Blocks until finished; progress arrives as "
            "the tool result's progress lines."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {"type": "string"},
                "output": {"type": "string", "description": "Output .mp4 path"},
                "crf": {"type": "integer", "description": "Quality, lower is better/bigger (default 20)"},
                "preset": {"type": "string", "description": "x264 preset (default medium)"},
                "width": {"type": "integer"},
                "height": {"type": "integer"},
                "fps": {"type": "number"},
            },
            "required": ["project", "output"],
        },
        "request": lambda a: {
            "method": "export.run",
            "path": a["project"],
            "output": a["output"],
            **({"crf": a["crf"]} if "crf" in a else {}),
            **({"preset": a["preset"]} if "preset" in a else {}),
            **({"width": a["width"]} if "width" in a else {}),
            **({"height": a["height"]} if "height" in a else {}),
            **(
                {"rateNum": round(a["fps"] * 1000), "rateDen": 1000}
                if "fps" in a
                else {}
            ),
        },
    },
    {
        "name": "preview_frame",
        "description": "Composite the true frame at one instant and write it as a PNG.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {"type": "string"},
                "time": {"type": "number", "description": "Timeline instant, seconds"},
                "output": {"type": "string", "description": "Output .png path"},
                "width": {"type": "integer"},
                "height": {"type": "integer"},
            },
            "required": ["project", "time", "output"],
        },
        "request": lambda a: {
            "method": "preview.frame",
            "path": a["project"],
            "time": a["time"],
            "output": a["output"],
            **({"width": a["width"]} if "width" in a else {}),
            **({"height": a["height"]} if "height" in a else {}),
        },
    },
    {
        "name": "add_media_to_timeline",
        "description": (
            "One-step: import a media file into the project bin AND drop it "
            "on the first free track. Returns both the media id and the clip "
            "id. Equivalent to media_import followed by edit(addClipAtFirstFree)."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {"type": "string"},
                "file": {"type": "string"},
                "start": {"type": "number", "description": "Timeline position, seconds (default 0)"},
            },
            "required": ["project", "file"],
        },
        "run": lambda a: run_add_media_to_timeline(a),
    },
    {
        "name": "workflow_state",
        "description": (
            "Where am I? One glance: the open project, media in the bin, clips "
            "on the active timeline, undo availability. No arguments needed - "
            "the session remembers the last project. Call this whenever unsure "
            "what to do next, and after any edit to confirm ids."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {"project": {"type": "string", "description": "Omit for the session's current project"}},
        },
        "run": lambda a: run_workflow_state(a),
    },
    {
        "name": "template_list",
        "description": "The template library.",
        "inputSchema": {"type": "object", "properties": {}},
        "request": lambda a: {"method": "template.list"},
    },
]

TOOL_BY_NAME = {t["name"]: t for t in TOOLS}


# ---------------------------------------------------------------------------
# MCP plumbing
# ---------------------------------------------------------------------------


def log(message):
    print("concat-mcp: %s" % message, file=sys.stderr, flush=True)


def tool_result(text, is_error=False):
    return {
        "content": [{"type": "text", "text": text}],
        **({"isError": True} if is_error else {}),
    }


# Common refusal sentences -> the next move that fixes them. Guards make
# mistakes recoverable; these hints make recovery one turn instead of three.
ERROR_HINTS = [
    ("No project is open",
     "Run project_create (new) or project_open (existing) first."),
    ("is not the open project",
     "Call project_get with the path project_get returned before - it names the project the window actually has open."),
    ("No such file",
     "Check the path exists and is readable from the machine running Concat."),
    ("nothing on the timeline",
     "The timeline is empty: media_import a file, then edit with op addClipAtFirstFree (or use add_media_to_timeline for both at once)."),
    ("No filter on this clip yet",
     "Filters are applied with edit op updateClip - see catalogue(kind='filter') for ids."),
]


def with_hint(text):
    for pattern, hint in ERROR_HINTS:
        if pattern in text:
            return f"{text}\nHINT: {hint}"
    if text.startswith("Error:"):
        return (text + "\nHINT: If an id was rejected, call project_get for the "
                "current clip/media ids; if a command seemed to do nothing, it "
                "may have been a tolerated no-op (e.g. importing a file already "
                "in the bin).")
    return text


def run_add_media_to_timeline(arguments):
    """Composite: import the file, then drop it on the first free track."""
    project, file = arguments["project"], arguments["file"]
    start = arguments.get("start", 0.0)

    response, _ = engine().call(
        {"method": "media.import", "path": project, "file": file})
    if "error" in response and "is not open" in response["error"]:
        # The project does not exist yet: make it, then retry the import.
        folder = Path(project)
        create = {"method": "project.create", "location": str(folder.parent),
                  "name": folder.name}
        opened = engine().call(create)
        if "error" in opened and "already holds" in opened["error"]:
            opened = engine().call({"method": "project.open", "path": project})
        if "error" in opened:
            return tool_result(with_hint(f"Error: {opened['error']}"), is_error=True)
        response, _ = engine().call(
            {"method": "media.import", "path": project, "file": file})
    if "error" in response:
        return tool_result(with_hint(f"Error: {response['error']}"), is_error=True)
    media_id = response["result"].get("createdId")
    if not media_id:
        return tool_result(
            "The file was already in the bin (tolerated no-op) - its media id "
            "was not re-minted. Call project_get to find it, then edit with "
            "op addClipAtFirstFree.")

    response, _ = engine().call({
        "method": "edit.apply",
        "path": project,
        "command": {"op": "addClipAtFirstFree", "mediaId": media_id, "start": start},
    })
    if "error" in response:
        return tool_result(with_hint(f"Error: {response['error']} (media id {media_id} is in the bin)"), is_error=True)
    clip_id = response["result"].get("createdId")
    return tool_result(
        f"Imported {file} as media {media_id}; clip {clip_id} placed at "
        f"{start}s on the first free track.")


def run_workflow_state(arguments):
    global LAST_PROJECT
    window = None
    engine_obj = engine()
    inner = getattr(engine_obj, "engine", engine_obj)
    getter = getattr(inner, "window_state", None)
    if getter:
        try:
            window = getter()
        except Exception:
            window = None

    path = arguments.get("project") or (window or {}).get("projectPath") or LAST_PROJECT
    if not path:
        return tool_result(
            "No project is known to this session yet - call project_create or "
            "project_open first (or pass project).", is_error=True)
    response, _ = engine().call({"method": "project.get", "path": path})
    if "error" in response:
        return tool_result(with_hint(f"Error: {response['error']}"), is_error=True)
    view = response["result"]
    lines = []
    if window:
        lines.append(
            f"[window] mode: {window.get('mode')}, project: {window.get('projectPath')}, "
            f"export dialog: {window.get('exportDialogOpen')} ({window.get('exportPhase')})")
    view = response["result"]
    project = view.get("project") or {}
    timelines = project.get("timelines") or []
    active_id = project.get("activeTimelineId")
    active = next((t for t in timelines if t.get("id") == active_id), {})
    summary = [
        *(lines or []),
        f"open project: {path}",
        f"media in bin: {[m.get('id') for m in project.get('media') or []]}",
        f"clips on '{active.get('name', '?')}': "
        f"{[(c.get('id'), c.get('name')) for c in active.get('clips') or []]}",
        f"undo available: {view.get('canUndo')}, redo available: {view.get('canRedo')}",
    ]
    lines = ["[state] " + line for line in summary]
    lines.append(json.dumps(view, ensure_ascii=False))
    return tool_result("\n".join(lines))


def call_engine(request):
    response, events = engine().call(request)
    if "error" in response:
        return tool_result("Error: %s" % response["error"], is_error=True), response
    lines = []
    for event in events:
        stage = event.get("stage") or event.get("event", "")
        lines.append(
            "[progress] %s frame %s/%s" % (stage, event.get("frame"), event.get("total"))
            if "frame" in event
            else "[event] %s" % json.dumps(event, ensure_ascii=False)
        )
    lines.append(json.dumps(response["result"], ensure_ascii=False))
    return tool_result("\n".join(lines)), response


def handle(request):
    method = request.get("method")
    rid = request.get("id")

    def reply(result=None, error=None):
        if rid is None:  # a notification: nothing to answer
            return None
        message = {"jsonrpc": "2.0", "id": rid}
        if error is not None:
            message["error"] = error
        else:
            message["result"] = result
        return message

    if method == "initialize":
        return reply(
            {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {"tools": {}},
                "serverInfo": SERVER_INFO,
            }
        )
    if method == "notifications/initialized" or method.startswith("notifications/"):
        return None
    if method == "ping":
        return reply({})
    if method == "tools/list":
        return reply({"tools": [{k: v for k, v in t.items() if k not in ("request", "run")} for t in TOOLS]})
    if method == "tools/call":
        params = request.get("params") or {}
        name, arguments = params.get("name"), params.get("arguments") or {}
        tool = TOOL_BY_NAME.get(name)
        if tool is None:
            return reply(error={"code": -32602, "message": "Unknown tool: %s" % name})
        try:
            if "run" in tool:
                return reply(tool["run"](arguments))
            outcome, response = call_engine(tool["request"](arguments))
            if tool["name"] in ("project_create", "project_open"):
                remember_project(arguments, outcome)
            return reply(outcome)
        except Exception as exc:  # engine died, bad args: report, keep serving
            log("tool %s failed: %s" % (name, exc))
            return reply(tool_result("Error: %s" % exc, is_error=True))
    return reply(error={"code": -32601, "message": "Method not found: %s" % method})


def main():
    log("starting")
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            request = json.loads(line)
        except json.JSONDecodeError as exc:
            log("bad JSON: %s" % exc)
            continue
        try:
            response = handle(request)
        except Exception as exc:  # never die on one bad request
            log("handler error: %s" % exc)
            response = (
                None
                if request.get("id") is None
                else {
                    "jsonrpc": "2.0",
                    "id": request["id"],
                    "error": {"code": -32603, "message": str(exc)},
                }
            )
        if response is not None:
            sys.stdout.write(json.dumps(response, ensure_ascii=False) + "\n")
            sys.stdout.flush()


if __name__ == "__main__":
    main()
