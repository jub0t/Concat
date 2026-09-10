// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Remote control: a local socket an AI agent drives the window through.
//!
//! Opt-in via `--remote-control` or `CONCAT_REMOTE=1`. The service listens
//! on 127.0.0.1 and speaks the Concat API's line-JSON - one request per
//! line in, one [`Response`] per line out, exactly like `concat-cli api` -
//! gated by a token whose file lands in the config dir beside the port
//! number.
//!
//! The route is deliberately thin. A request is posted onto the
//! event-loop thread and lands in the same [`Studio`] the pointers and
//! keyboards drive, through the same `apply`/`undo`/export seams the
//! window's own callbacks use - so an AI edit refreshes the timeline
//! exactly like a user's, and a remote export fills the window's export
//! sheet as it runs. Nothing here decides what an edit means; that is
//! `concat-project`'s, and the wire shapes are `concat-api`'s.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use crate::host::{self, Shell};
use crate::studio::Studio;
use concat_api::message::{Dirs, Reply, Request, Response, VersionInfo, Written};
use concat_api::{API_VERSION, catalogue};
use concat_host::preview::FrameSpec;
use concat_host::{projects, templates};

fn respond(out: &Sender<String>, result: Result<Reply, String>) {
    let response = Response::from(result);
    if let Ok(line) = serde_json::to_string(&response) {
        let _ = out.send(line);
    }
}

/// The one export wait: a remote `export.run` parks its response channel
/// here until the window's export flow finishes or cancels, because the
/// contract blocks until the file exists.
static EXPORT_WAITER: Mutex<Option<Sender<Result<String, String>>>> = Mutex::new(None);

/// Where the window's export flow reports its end, success or failure.
/// The window's own exports have no waiter parked, so this is a no-op for
/// them; a remote export's route is blocked on exactly this message.
pub(crate) fn export_finished(result: Result<String, String>) {
    let sender = EXPORT_WAITER
        .lock()
        .ok()
        .and_then(|mut waiter| waiter.take());
    if let Some(sender) = sender {
        let _ = sender.send(result);
    }
}

/// Starts the service when asked to. Never fatal: a machine whose config
/// directory cannot be written simply runs without remote control, the
/// same way it runs without recents.
pub fn start() {
    let flag = std::env::args().any(|arg| arg == "--remote-control");
    let env = std::env::var("CONCAT_REMOTE").is_ok_and(|value| value == "1");
    if !flag && !env {
        return;
    }
    let Ok(dirs) = concat_host::AppDirs::locate() else {
        eprintln!("concat: remote control off - no app directory");
        return;
    };
    let port = std::env::var("CONCAT_REMOTE_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("concat: remote control off - {error}");
            return;
        }
    };
    let address = listener
        .local_addr()
        .map(|address| address.port().to_string())
        .unwrap_or_default();
    let token = new_token();
    let token_file = dirs.config.join("remote-token");
    let port_file = dirs.config.join("remote-port");
    if let Err(error) = std::fs::write(&token_file, &token) {
        eprintln!("concat: remote control off - {error}");
        return;
    }
    let _ = std::fs::write(&port_file, &address);
    #[cfg(unix)]
    if let Ok(metadata) = std::fs::metadata(&token_file) {
        let mut permissions = metadata.permissions();
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o600);
        let _ = std::fs::set_permissions(&token_file, permissions);
    }
    eprintln!(
        "concat: remote control on 127.0.0.1:{address} (token in {})",
        token_file.display()
    );
    let spawned = std::thread::Builder::new()
        .name("concat-remote".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let token = token.clone();
                let _ = std::thread::Builder::new()
                    .name("concat-remote-conn".into())
                    .spawn(move || serve(stream, token));
            }
        });
    if let Err(error) = spawned {
        eprintln!("concat: remote control off - {error}");
    }
}

/// Random-enough for a file only this user reads: the OS source when it
/// has one, a hashed mix of the things unique to this process when not.
fn new_token() -> String {
    let mut bytes = [0u8; 16];
    if std::fs::File::open("/dev/urandom")
        .and_then(|mut file| std::io::Read::read_exact(&mut file, &mut bytes))
        .is_err()
    {
        let mut state = std::collections::hash_map::DefaultHasher::new();
        use std::hash::{Hash, Hasher};
        (std::process::id(), std::time::SystemTime::now()).hash(&mut state);
        let first = state.finish().to_le_bytes();
        state.write_u64(0x9E37_79B9_7F4A_7C15);
        let second = state.finish().to_le_bytes();
        bytes[..8].copy_from_slice(&first);
        bytes[8..].copy_from_slice(&second);
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One connection: handshake, then requests until the caller hangs up.
/// Responses are strictly ordered - the next request is only read after
/// the previous response has been written - because that is what the
/// line-JSON contract's callers assume.
fn serve(stream: TcpStream, token: String) {
    let Ok(read_stream) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(read_stream);
    let mut writer = stream;

    let mut handshake = String::new();
    if reader.read_line(&mut handshake).unwrap_or(0) == 0 || handshake.trim() != token {
        let _ = writeln!(writer, "wrong token");
        return;
    }

    let (out, inbox) = mpsc::channel::<String>();
    // One writer per connection: responses from the event-loop thread and
    // lines from worker threads all funnel through this channel.
    let writer_thread = std::thread::Builder::new()
        .name("concat-remote-write".into())
        .spawn(move || {
            for line in inbox {
                if writer.write_all(line.as_bytes()).is_err()
                    || writer.write_all(b"\n").is_err()
                    || writer.flush().is_err()
                {
                    break;
                }
            }
        });
    if writer_thread.is_err() {
        return;
    }

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        // Transport-level extension: a driver asks where the window IS
        // before it starts driving. Answered from the window's own state,
        // outside the Concat API contract on purpose.
        if text.contains("\"window-state\"") {
            let state = window_state();
            let _ = out.send(
                serde_json::to_string(&serde_json::json!({
                    "result": {"windowState": state}
                }))
                .unwrap_or_default(),
            );
            continue;
        }

        let request = match serde_json::from_str::<Request>(text) {
            Ok(request) => request,
            Err(error) => {
                respond(&out, Err(format!("not a request: {error}")));
                continue;
            }
        };
        let (done, wait) = mpsc::channel::<()>();
        let conn = Conn {
            out: out.clone(),
            done,
        };
        let posted = slint::invoke_from_event_loop(move || {
            Shell::with(|shell, app| {
                let outcome = {
                    let mut studio = shell.studio.borrow_mut();
                    route(&mut studio, request, &conn)
                };
                if let Handled::Now(result) = outcome {
                    conn.respond(result);
                }
                shell.studio.borrow().publish(&app, &shell.models);
            });
        });
        if posted.is_err() {
            respond(&out, Err("the window is gone".to_owned()));
            continue;
        }
        // The strict-ordering wait: deferred routes (an export, a frame)
        // answer from their worker and release this through `done`.
        if wait.recv().is_err() {
            break;
        }
    }
}

/// Answers "where is the window": mode, open project, export sheet. The
/// shared core of the line-JSON intercept and MCP's workflow_state; hops
/// onto the event-loop thread and back. A window that is shutting down
/// answers null.
pub(crate) fn window_state() -> serde_json::Value {
    let (state_tx, state_rx) = mpsc::channel();
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, _app| {
            let studio = shell.studio.borrow();
            let _ = state_tx.send(serde_json::json!({
                "mode": if studio.session.is_some() { "editor" } else { "start" },
                "projectPath": studio.project_path,
                "exportDialogOpen": studio.export.open,
                "exportPhase": format!("{:?}", studio.export.phase),
                "exportProgress": studio.export.progress,
            }));
        });
    });
    state_rx.recv().unwrap_or(serde_json::Value::Null)
}

/// Serves one Concat API request through the window's event loop - the
/// dispatch the MCP-over-HTTP service uses instead of a socket. Returns
/// the reply's `result` value together with every event that arrived
/// while the request ran (an export's progress lines, say), in arrival
/// order. A refusal sentence comes back as `Err`.
pub(crate) fn call(
    request: Request,
) -> Result<(serde_json::Value, Vec<serde_json::Value>), String> {
    let (out, inbox) = mpsc::channel::<String>();
    let (done, wait) = mpsc::channel::<()>();
    let conn = Conn { out, done };
    let posted = slint::invoke_from_event_loop(move || {
        Shell::with(|shell, app| {
            let outcome = {
                let mut studio = shell.studio.borrow_mut();
                route(&mut studio, request, &conn)
            };
            if let Handled::Now(result) = outcome {
                conn.respond(result);
            }
            shell.studio.borrow().publish(&app, &shell.models);
        });
    });
    posted.map_err(|_| "the window is gone".to_owned())?;
    wait.recv().map_err(|_| "the window is gone".to_owned())?;
    // The response is the one line carrying result/error; everything
    // before it is the request's event stream.
    let mut events = Vec::new();
    let mut reply = None;
    for line in inbox.try_iter() {
        let value: serde_json::Value =
            serde_json::from_str(&line).unwrap_or(serde_json::Value::Null);
        if value.get("result").is_some() || value.get("error").is_some() {
            reply = Some(value);
        } else {
            events.push(value);
        }
    }
    let reply = reply.ok_or_else(|| "the window sent no reply".to_owned())?;
    if let Some(error) = reply.get("error").and_then(|error| error.as_str()) {
        return Err(error.to_owned());
    }
    Ok((
        reply
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        events,
    ))
}

/// What one request turned into: an immediate reply, or work that owns
/// the reply and will send it when it is done. The reply rides unboxed:
/// the value lives for the few lines between a route returning and its
/// response serialising, so a second box behind `Response`'s would be
/// ceremony rather than saving.
#[allow(clippy::large_enum_variant)]
enum Handled {
    Now(Result<Reply, String>),
    Deferred,
}

/// A connection's answering side, cloned into every context that may
/// produce a line: the route on the event-loop thread, and the workers a
/// deferred route started.
#[derive(Clone)]
struct Conn {
    out: Sender<String>,
    done: Sender<()>,
}

impl Conn {
    fn respond(&self, result: Result<Reply, String>) {
        let response = Response::from(result);
        if let Ok(line) = serde_json::to_string(&response) {
            let _ = self.out.send(line);
        }
        let _ = self.done.send(());
    }
}

/// Routes one request against the window's own state. Runs on the
/// event-loop thread, with the same `&mut Studio` the UI callbacks get.
fn route(studio: &mut Studio, request: Request, conn: &Conn) -> Handled {
    match request {
        Request::Version => Handled::Now(Ok(Reply::Version(VersionInfo {
            api_version: API_VERSION.to_owned(),
            concat: env!("CARGO_PKG_VERSION").to_owned(),
            dirs: Dirs::from(&studio.host.dirs),
        }))),
        Request::ProjectCreate {
            location,
            name,
            video,
        } => {
            // Never drop the user's unsaved edits: whatever is open gets
            // saved to its own folder before the switch (a no-op when
            // nothing is open or nothing has changed).
            studio.save(false);
            let video = video.unwrap_or_default();
            let created = projects::create(
                &location,
                &name,
                video.width,
                video.height,
                video.rate_num,
                video.rate_den,
            );
            match created {
                Ok(info) => {
                    studio.open_project(info);
                    Handled::Now(view(studio))
                }
                Err(error) => Handled::Now(Err(error)),
            }
        }
        Request::ProjectOpen { path } => {
            // Never drop the user's unsaved edits: whatever is open gets
            // saved to its own folder before the switch (a no-op when
            // nothing is open or nothing has changed).
            studio.save(false);
            match projects::open(&path) {
                Ok(info) => {
                    studio.open_project(info);
                    Handled::Now(view(studio))
                }
                Err(error) => Handled::Now(Err(error)),
            }
        }
        Request::ProjectGet { path } if path.is_empty() => {
            // An empty path asks about whatever the window has open: the
            // one affordance a driver needs before it knows a name.
            Handled::Now(view(studio))
        }
        Request::ProjectGet { path } => match open_path(studio, &path) {
            Ok(_) => Handled::Now(view(studio)),
            Err(error) => Handled::Now(Err(error)),
        },
        Request::ProjectDocument { path } => match open_path(studio, &path) {
            Ok(()) => match studio.session.as_ref().map(|session| session.document()) {
                Some(document) => Handled::Now(Ok(Reply::Document(document))),
                None => Handled::Now(Err("No project is open in the window".to_owned())),
            },
            Err(error) => Handled::Now(Err(error)),
        },
        Request::ProjectSave { path, name } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            let saved = studio.session.as_mut().map(|session| {
                let (save_path, document) = session.prepare_save(name.as_deref());
                projects::save(&save_path, &document)
            });
            match saved {
                Some(Ok(())) => Handled::Now(Ok(Reply::Done(Default::default()))),
                Some(Err(error)) => Handled::Now(Err(error)),
                None => Handled::Now(Err("No project is open in the window".to_owned())),
            }
        }
        Request::ProjectList => Handled::Now(Ok(Reply::Projects(projects::list(
            &studio.host.dirs.config,
        )))),
        Request::ProjectClose { path, save } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            if !save {
                return Handled::Now(Err(
                    "closing without saving is not available over remote control - save first"
                        .to_owned(),
                ));
            }
            studio.close_project();
            Handled::Now(Ok(Reply::Done(Default::default())))
        }
        Request::ProjectSetVideo { path, video } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            match studio
                .session
                .as_mut()
                .map(|session| session.set_video(video))
            {
                Some(Ok(_)) => Handled::Now(view(studio)),
                Some(Err(error)) => Handled::Now(Err(error)),
                None => Handled::Now(Err("No project is open in the window".to_owned())),
            }
        }
        Request::EditUndo { path } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            studio.undo();
            Handled::Now(view(studio))
        }
        Request::EditApply { path, command } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            match studio.apply_checked(*command) {
                Ok(view) => Handled::Now(Ok(Reply::View(Box::new(view)))),
                Err(error) => Handled::Now(Err(error)),
            }
        }
        Request::EditRedo { path } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            studio.redo();
            Handled::Now(view(studio))
        }
        Request::MediaProbe { path } => {
            let conn = conn.clone();
            host::spawn(
                move || concat_host::media::probe(&path),
                move |_, _, _, result| match result {
                    Ok(summary) => conn.respond(Ok(Reply::Media(summary))),
                    Err(error) => conn.respond(Err(error)),
                },
            );
            Handled::Deferred
        }
        Request::MediaImport { path, file } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            let expected = studio.project_path.clone();
            let conn = conn.clone();
            host::spawn(
                move || concat_host::media::probe(&file),
                move |studio, _, _, result| match result {
                    Ok(summary) => {
                        // The probe outlived this request: whatever project
                        // is open NOW must still be the one it started for,
                        // or the media would land in the wrong bin.
                        if studio.project_path != expected {
                            conn.respond(Err(
                                "the open project changed while the file was being probed - import again"
                                    .to_owned(),
                            ));
                            return;
                        }
                        match studio.apply_checked(concat_project::Command::AddMedia {
                            item: summary.to_new_media(),
                        }) {
                            Ok(view) => conn.respond(Ok(Reply::View(Box::new(view)))),
                            Err(error) => conn.respond(Err(error)),
                        }
                    }
                    Err(error) => conn.respond(Err(error)),
                },
            );
            Handled::Deferred
        }
        Request::CatalogueList { kind } => match catalogue(kind.as_deref()) {
            Ok(packages) => Handled::Now(Ok(Reply::Packages(packages))),
            Err(error) => Handled::Now(Err(error)),
        },
        Request::TemplateList => Handled::Now(Ok(Reply::Templates(templates::list(
            &studio.host.dirs.config,
        )))),
        Request::TemplateInstantiate { .. } | Request::TemplateSave { .. } => Handled::Now(Err(
            "templates are not available over remote control yet - use the window or concat-cli"
                .to_owned(),
        )),
        Request::ExportRun { path, spec } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            // A remote caller omitting the frame and rate means the
            // project's own settings - not the export sheet's picks,
            // which no API request has ever touched.
            let frame = match (spec.width, spec.height) {
                (Some(width), Some(height)) => Some((width, height)),
                (None, None) => Some(studio.output_size()),
                _ => {
                    return Handled::Now(Err("width and height go together".to_owned()));
                }
            };
            let rate = match (spec.rate_num, spec.rate_den) {
                (Some(num), Some(den)) => Some((num, den)),
                (None, None) => {
                    let video = studio.project().active().video;
                    Some((video.rate_num, video.rate_den))
                }
                _ => {
                    return Handled::Now(Err("rateNum and rateDen go together".to_owned()));
                }
            };
            let started = studio.export_to(
                spec.output.clone(),
                spec.crf.unwrap_or(20),
                spec.preset.clone().unwrap_or_else(|| "medium".to_owned()),
                frame,
                rate,
            );
            let (width, height) = match started {
                Ok(frame) => frame,
                Err(error) => return Handled::Now(Err(error)),
            };
            let (sender, receiver) = mpsc::channel();
            if let Ok(mut waiter) = EXPORT_WAITER.lock() {
                *waiter = Some(sender);
            }
            let conn = conn.clone();
            std::thread::Builder::new()
                .name("concat-remote-export".into())
                .spawn(move || match receiver.recv() {
                    Ok(Ok(path)) => conn.respond(Ok(Reply::Written(Written {
                        path,
                        width,
                        height,
                    }))),
                    Ok(Err(error)) => conn.respond(Err(error)),
                    Err(_) => {}
                })
                .ok();
            Handled::Deferred
        }
        Request::ExportCancel => {
            studio.export_cancel();
            Handled::Now(Ok(Reply::Done(Default::default())))
        }
        Request::PreviewFrame {
            path,
            time,
            output,
            width,
            height,
        } => {
            if let Some(error) = wrong_project(studio, &path) {
                return Handled::Now(Err(error));
            }
            let Some(session) = studio.session.as_ref() else {
                return Handled::Now(Err("No project is open in the window".to_owned()));
            };
            let settings = session.settings();
            let (width, height) = width
                .zip(height)
                .unwrap_or((settings.width, settings.height));
            if width == 0 || height == 0 {
                return Handled::Now(Err("A frame needs a width and a height".to_owned()));
            }
            let mut clips = session.flattened_clips();
            clips.extend(
                studio
                    .host
                    .titles
                    .clips(session.project(), width, height)
                    .into_iter()
                    .map(|title| title.clip),
            );
            let spec = FrameSpec {
                time,
                width,
                height,
            };
            let monitor = studio.host.monitor.clone();
            let conn = conn.clone();
            host::spawn(
                move || {
                    let result = monitor.frame(Arc::new(clips), &settings, spec);
                    match result {
                        Ok(pixels) => {
                            let written = write_png(Path::new(&output), width, height, &pixels)
                                .map(|_| Written {
                                    path: output.clone(),
                                    width,
                                    height,
                                });
                            conn.respond(written.map(Reply::Written));
                        }
                        Err(error) => conn.respond(Err(error)),
                    }
                },
                |_, _, _, _| {},
            );
            Handled::Deferred
        }
    }
}

/// The session's view, when a project is open.
fn view(studio: &Studio) -> Result<Reply, String> {
    let view = studio
        .session
        .as_ref()
        .map(|session| session.view())
        .ok_or_else(|| "No project is open in the window".to_owned())?;
    Ok(Reply::View(Box::new(view)))
}

/// The one-session rule, from the other side: the window holds exactly one
/// project, and a caller naming another one is told what is open instead
/// of silently driving the wrong edit.
fn wrong_project(studio: &Studio, path: &str) -> Option<String> {
    if studio.project_path.is_empty() {
        return Some("No project is open in the window".to_owned());
    }
    if studio.project_path != path {
        return Some(format!(
            "{path} is not the open project - the window has {} open",
            studio.project_path
        ));
    }
    None
}

/// `Ok(())` when the named path is the window's open project.
fn open_path(studio: &Studio, path: &str) -> Result<(), String> {
    if let Some(error) = wrong_project(studio, path) {
        return Err(error);
    }
    Ok(())
}

/// Writes RGBA pixels as a PNG, creating the folder above the file.
fn write_png(output: &Path, width: u32, height: u32, pixels: &[u8]) -> Result<(), String> {
    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    }
    let file = std::fs::File::create(output)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder
        .write_header()
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    writer
        .write_image_data(pixels)
        .map_err(|error| format!("could not write {}: {error}", output.display()))?;
    writer
        .finish()
        .map_err(|error| format!("could not write {}: {error}", output.display()))
}
