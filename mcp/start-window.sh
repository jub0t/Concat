#!/bin/bash
# Starts (or restarts) the Concat window with remote control enabled,
# headless-safe: creates a virtual display when none exists.
#
#   mcp/start-window.sh                # headless box (SSH): window on virtual display
#   DISPLAY=:0 mcp/start-window.sh     # on the machine itself: real screen
#
# The window prints its remote-control port and writes remote-port /
# remote-token into the config directory for the MCP bridge to find.
set -u

REPO="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$REPO/engine/target/debug/concat"
LOG=/tmp/concat-win.log

[ -x "$BIN" ] || { echo "concat not built: $BIN"; exit 1; }

# Restart cleanly.
pkill -f "$BIN --remote-control" 2>/dev/null
sleep 1

# This machine's Intel HD 4000 has no Vulkan driver (ANV needs Skylake+),
# so wgpu can never find a GPU-backed adapter - on a real display or a
# virtual one. Always allow wgpu's CPU adapter.
export SLINT_WGPU_CPU=1

# A display is required. With no DISPLAY in the environment, serve one via
# Xvfb so the window works over SSH too.
if [ -z "${DISPLAY:-}" ]; then
    if ! pgrep -x Xvfb >/dev/null; then
        Xvfb :99 -screen 0 1600x1000x24 >/tmp/xvfb.log 2>&1 &
        sleep 2
    fi
    export DISPLAY=:99
    export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/xdgrt}"
    mkdir -p "$XDG_RUNTIME_DIR"
fi
export SLINT_BACKEND="${SLINT_BACKEND:-winit-software}"

# setsid fully detaches the window from this terminal and the SSH
# session: closing the terminal or losing the connection leaves it
# running. Respond through the log or the remote-control socket.
setsid "$BIN" --remote-control >"$LOG" 2>&1 </dev/null &
sleep 8

if grep -q "remote control on" "$LOG"; then
    grep "remote control on" "$LOG"
    echo "window running (log: $LOG)"
else
    echo "window failed to start:"; cat "$LOG"; exit 1
fi
