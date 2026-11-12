# Concat's log

Every run of Concat writes down what it did, in a file on your own computer.
Nothing in it is sent anywhere. It is the one thing that lets someone else see
what happened on your machine, so it is written to be read.

- [Finding and sending a log](#finding-and-sending-a-log), for anyone reporting a problem.
- [Reading a log](#reading-a-log), for anyone looking at one.
- [Writing a line](#writing-a-line), for anyone changing Concat's code.

## Finding and sending a log

**Settings › About › Show logs** opens the folder with this run's file selected.
Without the app:

| System | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\app.concat.editor\logs` |
| macOS | `~/Library/Application Support/app.concat.editor/logs` |
| Linux | `~/.local/share/app.concat.editor/logs` |

On Android, **Settings › About › Share the log** hands this run's file and the
one before it to the phone's share sheet (and leaves a copy in
Download/Concat). When the last run closed on its own, the next launch says so
and offers the same.

There is one file per run, named for when the run started
(`concat-20261111-052233.log`, in UTC). The newest ten are kept. When you report
a problem, attach the file from the run it happened in. If Concat closed on you,
that is usually the second newest: the newest is the run you opened to find the
log.

A log is safe to attach to a public issue. Your home folder is written as `~`,
so your user name is not in it, and Concat never writes down the words of your
project: not captions, not titles, not what a voice was asked to say. It does
name your files and projects, because "which file" is most of what a log is
for. Read it before you send it if those names are private.

To make a run say more, set **Settings › General › Log level** to Debug and do
the thing again. From a terminal, the `CONCAT_LOG` variable does the same and
more:

```text
CONCAT_LOG=debug                       Concat at debug
CONCAT_LOG=debug,concat_media=trace    and one part of it at trace
CONCAT_LOG=info,wgpu_core=debug        a library underneath, by name
CONCAT_LOG=all=debug                   everything, the libraries included
```

## Reading a log

```text
Concat log · 2026-11-11 · times are UTC
05:22:31.004 INFO  build      Concat 0.2.10 · macos-aarch64 · commit 784021e7d9d7 · optimised build  [concat_host::logs, main]
05:22:31.004 INFO  system     macOS 26.1  [concat_host::logs, main]
05:22:31.005 INFO  cpu        Apple M5 · 10 threads  [concat_host::logs, main]
05:22:31.005 INFO  memory     32.00 GB  [concat_host::logs, main]
05:22:31.369 INFO  gpu        Apple M5 · integrated · Metal  [concat::gpu, main]
05:22:41.090 INFO  project    opened Trip to Hunza in 212 ms · 3 tracks, 48 clips, 2 m 14 s  [concat::studio::project_files, main]
05:23:10.551 INFO  captions   done in 43.2 s · 120 captions · small model, 2 m 14 s of sound · load model 1.21 s · find speech 812 ms · transcribe 40.9 s  [concat_speech::transcribe, concat-worker-2]
05:23:11.900 ERROR gpu        the compositor's device was lost: the driver stopped answering; drawing in software from here  [concat_render::gpu, concat-worker-0]
05:31:02.118 INFO  log        run ended cleanly after 8 m 31 s  [concat_log, main]
```

From the left, a line is **when** it happened, **how bad** it is, **what it is
about**, and **what happened**. The brackets at the end say which part of the
code and which thread said it; you can ignore them unless you are chasing a bug.

- **ERROR**: something you asked for did not happen, or Concat lost something it needs.
- **WARN**: Concat carried on, but not the way it wanted to.
- **INFO**: the story of the run: what you did, what the machine is, how each job went.
- **DEBUG**, **TRACE**: the detail behind the story, off unless asked for.

Things worth knowing:

- A run that ended normally says `run ended cleanly` on its last line. A file
  without that line is a run that crashed, was killed, or lost power. The next
  run says so near its top (`last run`), with the last thing that went
  wrong.
- A line that would repeat is written ten times and then counted:
  `(+412 more in the last 5 s)`, `(repeated 197 more times)`.
- A long piece of work is one line that times each of its steps (`captions`
  above). That line is the one to compare between two machines or two versions.
- A run that writes more than 16 MB keeps its start and its end and says how
  much of the middle it dropped.
- A crash in a driver or a library leaves one `FATAL` line naming the library
  and the place in it.

## Writing a line

The log goes through the `log` crate everywhere: `log::info!("export: started")`.
`concat-log` (in `src/crates/concat-log`) is what is on the other end. Its
`Op` times a piece of work step by step.

### What is worth a line

Ask what someone needs to answer "what happened on that machine?" without
being able to ask the person. At INFO a run should read as a story:

- **What the machine is**: build, system, CPU, memory, GPU and driver, screen
  scale, audio device. Once, at the start, and again when one changes.
- **What the person did** that changes something: opened, imported, edited,
  exported, changed a setting. One line each, named as the interface names it.
- **What Concat decided** about the machine: which GPU, hardware or software
  decode, which encoder, which fallback. With the reason when it is not the
  first choice.
- **How each job went**: one `Op` line with its timings and its result.
- **What went wrong**, and what Concat did next.

And what is not: anything per frame, per packet, per pointer move or per
redraw (that is TRACE); a step inside a job (DEBUG, and `Op` does it for you);
the same fact twice; a line that only says the code got somewhere.

A normal hour of editing should come to a few hundred INFO lines. If a line
can fire on every frame it is at the wrong level, whatever it says.

**Never write down the person's words**: caption text, title text, what a voice
is asked to read, a prompt. Say how much there was (`412 characters`), not
what it was. Never write a token, a key or a URL that carries one.

### How a line is worded

Start with the topic and a colon, then say what happened in plain words,
lower case, with no full stop:

```rust
log::info!("import: {name} · {width}x{height} {fps} fps {codec} · {} · {}", span(length), bytes(size));
log::warn!("decode: {name} is on the CPU: the GPU's decoder refused {codec}");
log::error!("export: stopped at frame {frame} of {frames}: {error}; the file was not written");
```

- **One topic per line**, from the list below. The topic is the column a
  reader scans, so `export` is every line about exporting, whichever crate
  says it.
- **Say the thing, then the detail**, joined with ` · `. A reader who stops at
  the first clause should still have the fact.
- **Numbers carry their unit**, written by the helpers so they read the same
  everywhere: `concat_log::span(duration)` gives `843 ms`, `1.24 s`, `43.2 s`,
  `2 m 05 s`; `concat_log::bytes(count)` gives `812 KB`, `41.3 MB`, `2.75 GB`.
  Sizes are `1920x1080`, rates `60 fps`, `48 kHz`.
- **A warning says what was wanted and what happened instead.** An error says
  what was being done, why it failed (the underlying error, as it came), and
  what happens next.
- **Name things as the interface does** when the person did them (`Split`,
  `Draw on the GPU`), and as the system does when the machine did (`Metal`,
  `h264_videotoolbox`, an adapter's own name).

### Topics

| Topic | What it covers |
|---|---|
| `build` `system` `cpu` `memory` `log` | the lines every run begins with |
| `start` | launch, until the window is up; safe start |
| `gpu` | adapters, devices, what draws on what, device loss and recovery |
| `display` | screen, scale, window size |
| `project` | new, open, save, close, relink, autosave trouble |
| `import` | a file brought into a project, and what it is |
| `decode` | how a file is being read: hardware, software, why |
| `proxy` | lighter copies made for editing |
| `preview` | the monitor's pictures |
| `playback` | playing, and frames that could not be shown in time |
| `audio` | output and input devices, the mix |
| `edit` | what the person did on the timeline |
| `export` | settings, encoder, progress, result |
| `captions` | speech to text |
| `voice` | text to speech, voiceover, cloning, splitting a voice from its music |
| `cutout` `enhance` `stabilize` | each of those jobs |
| `effects` `fonts` `models` | what is loaded, compiled, downloaded |
| `network` `update` | the few times Concat goes online |
| `settings` | a setting changed, and to what |
| `api` | the Remote API and agents: server, clients, calls |
| `close` | leaving |

Add a topic when something new does not fit; keep it one or two short words.

### Work that takes time

Anything a person waits for is an `Op`:

```rust
let mut op = concat_log::op!("captions");
op.note(format_args!("{model} model, {} of sound", concat_log::span(length)));
op.step("load model");
let model = load(model)?;          // an early return is logged too: "left unfinished"
op.step("transcribe");
let words = transcribe(&model, &sound);
match words {
    Ok(words) => op.done(format_args!("{} captions", words.len())),
    Err(error) => op.failed(&error),
}
```

It writes one line when the work ends, `done`, `failed`, `cancelled` or
`left unfinished`, with how long the whole took and how long each step took,
and each step at DEBUG as it ends. Name the op with its topic, and steps with
what the machine is doing (`load model`, `decode`, `encode`), so the same
words show up for the same work on every machine.
