// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! An export that stops part way is taken up where it stopped, and the
//! file it then writes is the file an export that never stopped writes:
//! every frame once, in order (#846, #764).

use super::consolidate::{counting, number_of};
use super::*;

use concat_core::Halt;
use concat_export::ExportRequest;

/// Each frame's number, or None for one showing no footage.
fn numbers(file: &Exported) -> Vec<Option<u32>> {
    file.frames.iter().map(number_of).collect()
}

/// A timeline of the counting clip, its export request to `name`, and the
/// file the same export writes whole.
fn counted(scratch: &Scratch, name: &str) -> (Studio, ExportRequest, PathBuf) {
    let source = scratch.path().join("count.mp4");
    counting(&source);
    let mut studio = Studio::new(scratch.path(), name, video(WIDTH, HEIGHT, 30, 1));
    let media_id = studio.import(&source);
    let track_id = studio.lanes()[0];
    studio.made(Command::AddClip {
        media: media_id,
        lane: track_id,
        start: at(0.0),
        ripple: false,
    });
    let output = studio.exports.join(format!("{name}.mp4"));
    let spec = ExportSpec {
        output: output.to_string_lossy().into_owned(),
        crf: 8,
        preset: "fast".into(),
        ..Default::default()
    };
    let request = export::request(&studio.session, &spec, Vec::new());
    (studio, request, output)
}

/// `request` in parts of ten frames, stopping after `stop_after` of them
/// as a crash would.
fn in_parts(request: &ExportRequest, stop_after: Option<usize>) -> ExportRequest {
    let mut request = request.clone();
    let resume = request
        .resume
        .as_mut()
        .expect("the host says what the edit is");
    resume.part_frames = Some(10);
    resume.stop_after = stop_after;
    request
}

/// Runs `request`; the first frame it said it was rendering.
fn first_rendered(request: &ExportRequest) -> Result<i64, Halt> {
    let mut first = None;
    export::run(request, &AtomicBool::new(false), |progress| {
        if progress.stage == "rendering" && first.is_none() {
            first = Some(progress.frame);
        }
    })?;
    Ok(first.unwrap_or(-1))
}

#[test]
fn an_export_that_stopped_is_taken_up_at_the_part_it_stopped_in() {
    let scratch = Scratch::new("resume-taken-up");
    let (_studio, request, output) = counted(&scratch, "Resumed");

    let mut whole = request.clone();
    whole.resume = None;
    whole.output = output
        .with_file_name("whole.mp4")
        .to_string_lossy()
        .into_owned();
    export::run(&whole, &AtomicBool::new(false), |_| {}).expect("the whole export");
    let reference = Exported::read("whole", Path::new(&whole.output), (30, 1));
    assert_eq!(reference.frames.len(), 48, "the counting clip's 48 frames");

    let folder = output.with_file_name(".Resumed.concat-parts");
    match first_rendered(&in_parts(&request, Some(2))) {
        Err(Halt::Failed(why)) => assert!(why.contains("after 2 parts"), "{why}"),
        other => panic!("the export should have stopped after two parts: {other:?}"),
    }
    assert!(!output.exists(), "a stopped export writes no file");
    assert!(folder.is_dir(), "a stopped export keeps its parts");

    let first = first_rendered(&in_parts(&request, None)).expect("taken up");
    assert_eq!(first, 20, "taken up at the third part, not the first frame");
    assert!(!folder.exists(), "a finished export leaves no parts behind");
    let resumed = Exported::read("resumed", &output, (30, 1));
    assert_eq!(
        numbers(&resumed),
        numbers(&reference),
        "the taken-up file shows every frame once, in order, as the whole one does"
    );
}

#[test]
fn the_parts_of_another_edit_are_not_taken_up() {
    let scratch = Scratch::new("resume-another-edit");
    let (mut studio, request, output) = counted(&scratch, "Edited");
    assert!(matches!(
        first_rendered(&in_parts(&request, Some(2))),
        Err(Halt::Failed(_))
    ));

    // The edit changes: the clip moves later, and the timeline with it.
    let clip = studio.clip_ids()[0];
    let track_id = studio.lanes()[0];
    studio.apply(Command::MoveClips {
        moves: vec![ClipMove {
            item: clip,
            start: at(0.5),
            lane: Some(track_id),
        }],
    });
    let spec = ExportSpec {
        output: output.to_string_lossy().into_owned(),
        crf: 8,
        preset: "fast".into(),
        ..Default::default()
    };
    let edited = export::request(&studio.session, &spec, Vec::new());
    assert_ne!(
        edited.resume.as_ref().map(|resume| &resume.key),
        request.resume.as_ref().map(|resume| &resume.key),
        "another edit, another key"
    );
    let first = first_rendered(&in_parts(&edited, None)).expect("exported afresh");
    assert_eq!(first, 0, "nothing of the other edit's parts is used");
    let file = Exported::read("edited", &output, (30, 1));
    let shown = numbers(&file);
    assert_eq!(
        shown.len(),
        48 + 15,
        "half a second of nothing, then the clip"
    );
    assert!(shown[..15].iter().all(Option::is_none), "the gap is dark");
    assert_eq!(
        shown[15..].to_vec(),
        (0..48).map(Some).collect::<Vec<_>>(),
        "then every frame of the clip once"
    );
}
