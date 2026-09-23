# JSON-RPC

**In one line:** one JSON object per line, in and out, over stdin, TCP or a
Unix socket. The same lines everywhere.

**On this page:** [The envelope](#the-envelope) · [Over stdin](#over-stdin-concat-cli-api)
· [Over a socket](#over-a-socket-concat-cli-serve) · [From the window](#from-the-window)
· [The token](#the-token) · [Events](#events) · [Security](#security) ·
[Python client](#a-minimal-python-client)

---

## The envelope

A call, its response, and an event, as JSON-RPC 2.0:

```json
{"jsonrpc":"2.0","id":1,"method":"project.open","params":{"path":"/edits/Reel"}}
```
```json
{"jsonrpc":"2.0","id":1,"result":{"project":{"…":"…"},"canUndo":false,"canRedo":false,"settings":{"…":"…"}}}
```
```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32001,"message":"/edits/Reel is not open - open it first","data":{"code":"notOpen"}}}
```
```json
{"jsonrpc":"2.0","method":"export.progress","params":{"job":"j1","path":"/edits/Reel","frame":30,"total":150,"stage":"video"}}
```

Rules:

| Rule | Detail |
|---|---|
| One object per line | Newline-delimited. No pretty printing across lines |
| `id` is yours | A string, a number or null. It comes back as given. Anything else is `invalid` |
| Every call is answered | Even one without an `id` (the reply's `id` is `null`) |
| `params` is an object | The method's fields. Omit it for a method with none |
| Errors carry two codes | `error.code` is the JSON-RPC number, `error.data.code` is the API's name. See [Errors](../api/overview.md#errors) |
| Events are notifications | No `id`. `method` is the event's name, `params` its fields |

> [!TIP]
> **The bare shape is accepted too.** `{"method": "version"}` with no
> envelope is the request as it is, and it is answered with `"id": null`.
> Handy when typing by hand.

---

## Over stdin: `concat-cli api`

From a source checkout:

```sh
cd src
cargo run -p concat-cli -- api '{"method":"catalogue.list","kind":"filter"}'   # one request
cargo run -p concat-cli -- api < edits.jsonl                                   # one call per line
```

How it behaves:

- Reads a line, answers it, flushes. Blank lines are skipped.
- Writes events between responses as they happen, never half a line.
- A line that is not a call gets an error response; the loop goes on.
- At end of input it **waits for running jobs**, so an export begun on
  the last line still finishes and its events are written.

No token, no socket. The process is yours.

---

## Over a socket: `concat-cli serve`

```sh
cd src
cargo run -p concat-cli -- serve                            # JSON-RPC on 127.0.0.1:7420
cargo run -p concat-cli -- serve --json 0.0.0.0:7420        # any interface (read Security first)
cargo run -p concat-cli -- serve --socket /tmp/concat.sock  # a Unix socket
cargo run -p concat-cli -- serve --token "my secret"        # your own token
```

It prints where it listens and the token to present, then serves until
stopped:

```
Concat API 0.2: JSON-RPC on 127.0.0.1:7420
Concat API 0.2: token 3f9c1e…
```

| Flag | Meaning |
|---|---|
| `--json ADDR` | TCP address for JSON-RPC lines. Default `127.0.0.1:7420` when nothing else is asked for |
| `--socket PATH` | A Unix socket path (Unix only). A file already there is replaced |
| `--grpc ADDR` | gRPC, in a build with the feature. See [gRPC](grpc.md) |
| `--token TOKEN` | The token every connection presents. Also `CONCAT_API_TOKEN`. Minted when absent |
| `--root DIR` | A folder the API may write under. Repeatable. Your home when absent; `--root /` for anywhere |

Several flags at once serve on all of them. All callers share one
dispatcher and its open projects.

---

## From the window

**Settings › Remote** serves the same JSON-RPC lines while the editor is
open.

| Control | Meaning |
|---|---|
| **Serve the API** | The switch. Remembered across launches |
| **Address** | Host and port. Default `127.0.0.1:7420` |
| **Token** | Yours, or one the window mints and shows you when the field is empty |
| Status line | "Listening on … · N connected", or why the bind failed |

A caller on the socket edits projects of its own, never the one on
screen: a folder the window has open is `refused` over the socket, and
the window refuses to open one a caller holds. The export slot is shared,
so one export at a time holds across the two. The API writes under your
home folder.

---

## The token

Every connection, **loopback included**, presents the token as its
**first line**, as a call to `auth`:

```json
{"jsonrpc":"2.0","id":0,"method":"auth","params":{"token":"3f9c1e…"}}
```
```json
{"jsonrpc":"2.0","id":0,"result":{}}
```

- Anything else first, or the wrong token: an `unauthorized` error and
  the connection is closed.
- `auth` is the transport's word, not the API's. It never reaches the
  dispatcher and is not a method you call again.
- The comparison is constant-time.
- With no token configured, the server **mints one** (128 bits, 32 hex
  digits) that only the process that started it knows: printed by
  `concat-cli serve`, shown on the window's Remote page.

Why loopback too: another user's process on the same machine reaches
`127.0.0.1` as easily as yours does.

---

## Events

Once authenticated, a connection receives **every** event from every job,
whoever started it. Each names its `job` and `path`; keep the ones you
asked for.

Responses and events share one writer per connection, so a line is never
interleaved with another. But an event can arrive **between** your call
and its response. Read by `method` versus `id`, not by position:

```
you →  {"jsonrpc":"2.0","id":7,"method":"project.get",...}
   ←  {"jsonrpc":"2.0","method":"export.progress","params":{...}}   ← someone's job
   ←  {"jsonrpc":"2.0","id":7,"result":{...}}                       ← your answer
```

---

## Security

> [!WARNING]
> A server is a door into the machine, as the user running it. The door
> is narrow, and the token is the only key.

- **No encryption.** Bind off loopback only behind something that
  provides it (an SSH tunnel, a reverse proxy with TLS).
- The token gates the door; the network is the wall. Do not put the port
  on the open internet.
- **Writes stay under the roots.** A created project, an instantiated
  template, an export and a preview file must land under a `--root` (the
  window: your home folder); anything else is `refused`, and a path with
  `..` in it always is. Reads are not confined: anyone with the token can
  probe any file the server's user can read and open any project folder.
- **Sizes are bounded.** A frame or an export is at most 8192 a side, an
  export's frame rate at most 240 a second and its `crf` at most 63:
  anything larger is `invalid`.
- **The transport has limits.** A line is at most 4 MiB (the `auth` line
  4 KiB); a caller has ten seconds to present its token; at most 64
  callers are connected at once, the next is answered `busy` and closed;
  a caller that stops reading is hung up on once 256 lines are waiting
  for it.

---

## A minimal Python client

Standard library only. Handles the handshake, matches responses by `id`,
and hands events to a callback.

```python
import json, socket, itertools

class Concat:
    def __init__(self, host="127.0.0.1", port=7420, token="", on_event=None):
        self.sock = socket.create_connection((host, port))
        self.reader = self.sock.makefile("r", encoding="utf-8")
        self.ids = itertools.count(1)
        self.on_event = on_event or (lambda e: None)
        self._send({"jsonrpc": "2.0", "id": 0, "method": "auth", "params": {"token": token}})
        self._wait(0)

    def call(self, method, **params):
        rid = next(self.ids)
        self._send({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
        return self._wait(rid)

    def wait_for_job(self, job):
        """Blocks until export.done / export.failed for `job`; returns the event."""
        while True:
            msg = self._read()
            if "id" in msg:
                continue  # a response to something else; not expected here
            self.on_event(msg)
            if msg["params"].get("job") == job and msg["method"] in ("export.done", "export.failed"):
                return msg

    def _send(self, obj):
        self.sock.sendall((json.dumps(obj) + "\n").encode("utf-8"))

    def _read(self):
        line = self.reader.readline()
        if not line:
            raise ConnectionError("server closed the connection")
        return json.loads(line)

    def _wait(self, rid):
        while True:
            msg = self._read()
            if "id" not in msg:
                self.on_event(msg)      # an event; keep waiting
                continue
            if msg["id"] != rid:
                continue                # not ours; keep waiting
            if "error" in msg:
                raise RuntimeError(f'{msg["error"]["data"]["code"]}: {msg["error"]["message"]}')
            return msg["result"]
```

Use it:

```python
c = Concat(token="3f9c1e…", on_event=lambda e: print(e["method"], e["params"]))
print(c.call("version")["capabilities"])
view = c.call("project.create", location="/edits", name="Reel")
view = c.call("media.import", path="/edits/Reel", file="/footage/take1.mp4")
c.call("edit.apply", path="/edits/Reel", command={"op": "addClipAtFirstFree", "mediaId": view["createdId"], "start": 0})
started = c.call("export.run", path="/edits/Reel", output="/edits/reel.mp4")
print(c.wait_for_job(started["job"]))
```

More in [Recipes](../recipes.md).
