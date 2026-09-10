#!/usr/bin/env python3
"""E2E test of the Concat MCP bridge over real JSON-RPC stdio.

Spawns mcp/concat_mcp.py, performs the MCP handshake, then drives a full
edit session: create project, add a title, apply an effect-bearing layer,
look at a frame, export an MP4, and checks the artifacts exist.
"""

import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MCP = REPO / "mcp" / "concat_mcp.py"

proc = subprocess.Popen(
    [sys.executable, str(MCP)],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
    encoding="utf-8",
)

_next_id = [0]


def rpc(method, params=None, notify=False):
    message = {"jsonrpc": "2.0", "method": method}
    if not notify:
        _next_id[0] += 1
        message["id"] = _next_id[0]
    if params is not None:
        message["params"] = params
    proc.stdin.write(json.dumps(message) + "\n")
    proc.stdin.flush()
    if notify:
        return None
    while True:
        line = proc.stdout.readline()
        if line == "":
            raise RuntimeError("mcp server exited")
        response = json.loads(line)
        if response.get("id") == _next_id[0]:
            return response


def tool(name, arguments=None):
    response = rpc("tools/call", {"name": name, "arguments": arguments or {}})
    if "error" in response:
        raise AssertionError("rpc error for %s: %s" % (name, response["error"]))
    result = response["result"]
    text = result["content"][0]["text"]
    if result.get("isError"):
        raise AssertionError("tool %s failed: %s" % (name, text))
    return text


def check(label, condition):
    print("%-46s %s" % (label, "OK" if condition else "FAIL"))
    if not condition:
        sys.exit(1)


tmp = Path(tempfile.mkdtemp(prefix="concat-mcp-test-"))
try:
    # Handshake
    init = rpc("initialize", {"protocolVersion": "2024-11-05", "capabilities": {}})
    server = init["result"]["serverInfo"]["name"]
    rpc("notifications/initialized", notify=True)
    check("MCP handshake (%s)" % server, server == "concat-mcp")

    tools = rpc("tools/list")["result"]["tools"]
    names = {t["name"] for t in tools}
    check("tools/list has %d tools" % len(tools), {"edit", "export_video", "project_get"} <= names)

    # Engine reachable
    version = json.loads(tool("concat_version"))
    check("engine version %s / api %s" % (version["concat"], version["apiVersion"]), version["apiVersion"] == "0.1")

    # Create + edit + save
    view = json.loads(tool("project_create", {"location": str(tmp), "name": "ai-demo", "width": 640, "height": 360, "fps": 30}))
    project = str(tmp / "ai-demo")
    check("project created at %s" % project, (tmp / "ai-demo").is_dir())
    text = json.loads(tool("edit", {"project": project, "command": {
        "op": "addTextClip", "trackId": None, "start": 0.5, "duration": 2.5,
        "style": {"content": "Made by an AI", "fontSize": 0.12, "color": "#ffd54a"},
    }}))
    check("text clip minted id %s" % text.get("createdId"), bool(text.get("createdId")))

    packages = json.loads(tool("catalogue", {"kind": "effect"}))
    check("catalogue lists %d effect packages" % len(packages), len(packages) >= 20)
    effect_id = packages[0]["id"]
    layer = json.loads(tool("edit", {"project": project, "command": {
        "op": "addLayerClip", "trackId": None, "start": 0.0, "duration": 3.0,
        "effectId": effect_id, "name": "AI layer",
    }}))
    check("layer clip applied (%s)" % effect_id, layer.get("createdId") is not None)

    def active_clips(state):
        project = state["project"]
        return next(t["clips"] for t in project["timelines"] if t["id"] == project["activeTimelineId"])

    state = json.loads(tool("project_get", {"path": project}))
    check("project_get shows %d clips" % len(active_clips(state)), len(active_clips(state)) == 2)

    # Undo / redo round trip
    tool("edit_undo", {"path": project})
    state = json.loads(tool("project_get", {"path": project}))
    check("undo removes the layer", len(active_clips(state)) == 1)
    tool("edit_redo", {"path": project})
    state = json.loads(tool("project_get", {"path": project}))
    check("redo restores the layer", len(active_clips(state)) == 2)

    # Frame + export
    png = str(tmp / "frame.png")
    json.loads(tool("preview_frame", {"project": project, "time": 1.0, "output": png}))
    check("preview frame written", png and Path(png).stat().st_size > 1000)

    tool("project_save", {"path": project})
    check("document saved", (tmp / "ai-demo" / "concat.json").exists())

    mp4 = str(tmp / "out.mp4")
    export_text = tool("export_video", {"project": project, "output": mp4, "width": 640, "height": 360})
    size = Path(mp4).stat().st_size if Path(mp4).exists() else 0
    check("export wrote %d bytes %s" % (size, "(with progress)" if "[progress]" in export_text else ""), size > 3000)

    print("\nALL PASS - artifacts in %s" % tmp)
finally:
    proc.terminate()
    if "--keep" not in sys.argv:
        shutil.rmtree(tmp, ignore_errors=True)
