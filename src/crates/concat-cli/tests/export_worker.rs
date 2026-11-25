// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! The API's exports as `concat-cli api` runs them: in a worker of its own,
//! so a crash in the render is an `export.failed` for the caller and not
//! the process gone (#762). The crate's other tests drive the API inside
//! their own process, which serves no workers and exports in-process; this
//! is the one place the binary's own path is run.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use serde_json::{Value, json};

/// `concat-cli api`, its requests in and its lines out.
struct Cli {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<Value>,
    next: u64,
}

impl Cli {
    /// The binary in `api` mode, with `abort` as the export worker's test
    /// hook when given.
    fn start(abort: Option<&str>) -> Cli {
        let mut command = Command::new(env!("CARGO_BIN_EXE_concat-cli"));
        command
            .arg("api")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env_remove("CONCAT_EXPORT_IN_PROCESS");
        match abort {
            Some(abort) => command.env("CONCAT_TEST_EXPORT_WORKER_ABORT", abort),
            None => command.env_remove("CONCAT_TEST_EXPORT_WORKER_ABORT"),
        };
        let mut child = command.spawn().expect("concat-cli starts");
        let stdin = child.stdin.take().expect("its input");
        let stdout = child.stdout.take().expect("its output");
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if let Ok(value) = serde_json::from_str::<Value>(&line)
                    && sender.send(value).is_err()
                {
                    break;
                }
            }
        });
        Cli {
            child,
            stdin,
            lines,
            next: 0,
        }
    }

    /// One call, and its result; panics on an error.
    fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(self.stdin, "{line}").expect("a request goes in");
        self.stdin.flush().expect("and is sent");
        loop {
            let message = self
                .lines
                .recv_timeout(Duration::from_secs(120))
                .unwrap_or_else(|_| panic!("no answer to {method}: the process is gone"));
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(error) = message.get("error") {
                    panic!("{method} failed: {error}");
                }
                return message["result"].clone();
            }
        }
    }

    /// The event that ends the export: `export.done` or `export.failed`.
    fn export_end(&mut self) -> Value {
        loop {
            let message = self
                .lines
                .recv_timeout(Duration::from_secs(600))
                .expect("the export ends with an event, the process still up");
            let method = message.get("method").and_then(Value::as_str);
            if matches!(method, Some("export.done" | "export.failed")) {
                return message;
            }
        }
    }
}

impl Drop for Cli {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A small still to put on the timeline.
fn still(path: &Path) {
    let (width, height) = (64u32, 36u32);
    let file = std::fs::File::create(path).expect("a still");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let pixels: Vec<u8> = (0..width * height)
        .flat_map(|_| [200u8, 120, 40, 255])
        .collect();
    let mut writer = encoder.write_header().expect("a header");
    writer.write_image_data(&pixels).expect("its pixels");
}

/// A project with the still on its timeline, saved, and an export of it
/// started; how the export ended.
fn export(cli: &mut Cli, folder: &Path) -> Value {
    std::fs::create_dir_all(folder).expect("a folder");
    let source = folder.join("still.png");
    still(&source);
    let location = folder.to_string_lossy().into_owned();
    cli.call(
        "project.create",
        json!({"location": location, "name": "Worker",
               "video": {"width": 320, "height": 180, "rateNum": 30, "rateDen": 1}}),
    );
    let project = folder.join("Worker").to_string_lossy().into_owned();
    let view = cli.call(
        "media.import",
        json!({"path": project, "file": source.to_string_lossy()}),
    );
    let media = view["project"]["media"]
        .as_array()
        .and_then(|media| media.last())
        .and_then(|media| media["id"].as_str())
        .expect("the still's id")
        .to_owned();
    cli.call(
        "edit.apply",
        json!({"path": project,
               "command": {"op": "addClipAtFirstFree", "mediaId": media, "start": 0.0}}),
    );
    cli.call("project.save", json!({"path": project}));
    let output = folder.join("out.mp4").to_string_lossy().into_owned();
    cli.call("export.run", json!({"path": project, "output": output}));
    cli.export_end()
}

/// A fresh folder for one test.
fn folder(name: &str) -> std::path::PathBuf {
    let folder = std::env::temp_dir().join(format!(
        "concat-cli-export-worker-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&folder);
    folder
}

#[test]
fn an_api_export_runs_in_a_worker_to_its_end() {
    let folder = folder("done");
    let mut cli = Cli::start(None);
    let end = export(&mut cli, &folder);
    assert_eq!(end["method"], "export.done", "{end}");
    assert!(folder.join("out.mp4").is_file(), "no file written: {end}");
    let _ = std::fs::remove_dir_all(&folder);
}

/// A worker that dies outside the graphics driver, as #762's heap
/// corruption kills one: the caller hears `export.failed` in plain words,
/// and the process goes on answering.
#[test]
fn a_worker_that_crashes_is_an_export_failed_and_the_process_answers_on() {
    let folder = folder("crash");
    let mut cli = Cli::start(Some("elsewhere"));
    let end = export(&mut cli, &folder);
    assert_eq!(end["method"], "export.failed", "{end}");
    let message = end["params"]["error"]["message"]
        .as_str()
        .expect("a message");
    assert!(
        message.starts_with("The export stopped before it finished"),
        "{message}"
    );
    assert!(!message.contains("0x"), "a status code in: {message}");
    let version = cli.call("version", json!({}));
    assert!(!version.is_null(), "the process answers after the crash");
    let _ = std::fs::remove_dir_all(&folder);
}

/// A worker that dies in the graphics driver costs one try: the export is
/// drawn again on the CPU, and finishes.
#[test]
fn a_worker_that_crashes_in_the_driver_is_drawn_again_on_the_cpu() {
    let folder = folder("driver");
    let mut cli = Cli::start(Some("1"));
    let end = export(&mut cli, &folder);
    assert_eq!(end["method"], "export.done", "{end}");
    assert!(folder.join("out.mp4").is_file(), "no file written: {end}");
    let _ = std::fs::remove_dir_all(&folder);
}
