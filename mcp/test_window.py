#!/usr/bin/env python3
"""Drives a running Concat window over its remote-control socket.

Usage:
  test_window.py [project_dir] [export_mp4] [frame_png] [media_file]

Walks the MCP-exposed flow: version, create, (optional) import a real
video and drop it on the timeline, add a title, preview a frame, save,
export. Watch the window while this runs - everything appears live.
"""

import json
import shutil
import socket
import subprocess
import sys
from pathlib import Path

PROJECT = Path(sys.argv[1] if len(sys.argv) > 1 else "/tmp/concat-window-demo")
EXPORT = sys.argv[2] if len(sys.argv) > 2 else "/tmp/concat-window-demo.mp4"
FRAME = sys.argv[3] if len(sys.argv) > 3 else "/tmp/concat-window-frame.png"
MEDIA = sys.argv[4] if len(sys.argv) > 4 else None


def config_dir():
    home = Path.home()
    for candidate in [
        home / ".config" / "app.concat.editor",
        home / "Library" / "Application Support" / "app.concat.editor",
        home / "AppData" / "Roaming" / "app.concat.editor",
    ]:
        if (candidate / "remote-port").exists():
            return candidate
    raise SystemExit("no remote-port file - is the window running with --remote-control?")


def main():
    directory = config_dir()
    address = ("127.0.0.1", int((directory / "remote-port").read_text().strip()))
    token = (directory / "remote-token").read_text().strip()
    print(f"window socket at {address[0]}:{address[1]}")

    connection = socket.create_connection(address, timeout=300)
    reader = connection.makefile("r", encoding="utf-8")
    writer = connection.makefile("w", encoding="utf-8")
    writer.write(token + "\n")
    writer.flush()

    def call(request):
        writer.write(json.dumps(request, ensure_ascii=False) + "\n")
        writer.flush()
        while True:
            line = reader.readline()
            if line == "":
                raise RuntimeError("window closed the connection")
            message = json.loads(line)
            if "result" in message or "error" in message:
                return message
            print(f"  [event] {message}")

    def must(response, label=""):
        prefix = f"{label}: " if label else ""
        if "error" in response:
            raise RuntimeError(f"{prefix}{response['error']}")
        return response["result"]

    version = call({"method": "version"})
    print("version:", version["result"]["concat"], "/ api", version["result"]["apiVersion"])

    project = str(PROJECT)
    # Deterministic: every run builds the same fresh project instead of
    # appending to whatever a previous run left open.
    if PROJECT.exists():
        shutil.rmtree(PROJECT)
        print("removed previous project folder")
    must(call({"method": "project.create", "location": str(PROJECT.parent),
               "name": PROJECT.name}), "create")
    print("project ready:", project)

    if MEDIA:
        imported = must(call({"method": "media.import", "path": project, "file": MEDIA}),
                        "media import")
        media_id = imported.get("createdId")
        print("media imported:", media_id)
        clip = must(call({"method": "edit.apply", "path": project, "command": {
            "op": "addClipAtFirstFree", "mediaId": media_id, "start": 0.0,
        }}), "add clip")
        print("footage clip:", clip.get("createdId"))

    title = must(call({"method": "edit.apply", "path": project, "command": {
        "op": "addTextClip", "trackId": None, "start": 0.5, "duration": 2.5,
        "style": {"content": "Driven by AI", "fontSize": 0.14, "color": "#ffd54a"},
    }}), "add title")
    print("title clip:", title.get("createdId"))

    state = must(call({"method": "project.get", "path": project}))
    timeline = next(t for t in state["project"]["timelines"]
                    if t["id"] == state["project"]["activeTimelineId"])
    print("timeline holds", len(timeline["clips"]), "clip(s):",
          [clip["name"] for clip in timeline["clips"]])

    frame = must(call({"method": "preview.frame", "path": project,
                       "time": 1.2, "output": FRAME}))
    print("frame:", frame["path"], frame["width"], "x", frame["height"])

    must(call({"method": "project.save", "path": project}))
    print("document saved:", (PROJECT / "concat.json").exists())

    export = must(call({"method": "export.run", "path": project,
                        "output": EXPORT, "width": 1280, "height": 720}))
    print("export:", export["path"], export["width"], "x", export["height"])
    if shutil.which("ffprobe"):
        probe = subprocess.run(
            ["ffprobe", "-v", "error", "-show_entries", "format=duration",
             "-of", "default=nw=1:nk=1", EXPORT],
            capture_output=True, text=True)
        print("export duration:", probe.stdout.strip(), "seconds")

    connection.close()


if __name__ == "__main__":
    main()
