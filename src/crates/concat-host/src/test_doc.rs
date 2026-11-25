// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah

//! A document the tests edit by commands, with no history: one timeline
//! to start, and seconds where the commands take ticks.

use concat_doc::commands::{self, Applied, Command};
use concat_doc::model::{ItemId, Media, MediaId, MediaKind, Model, Size};
use concat_doc::{Document, Tick, TickLen};

/// A document and the commands made to it.
pub struct Doc(pub Document<Model>);

impl Doc {
    /// One timeline, its lanes, showing.
    pub fn new() -> Doc {
        let mut doc = Doc(Document::new());
        doc.apply(Command::AddTimeline);
        doc
    }

    /// Makes a command, which must not be refused.
    pub fn apply(&mut self, command: Command) -> Applied {
        let (after, applied) = commands::made(&self.0, &command)
            .unwrap_or_else(|error| panic!("{command:?} is refused: {error}"));
        self.0 = after;
        applied
    }

    /// Makes a command that creates an item, and gives its id.
    pub fn item(&mut self, command: Command) -> ItemId {
        let created = self.apply(command).created.expect("it makes something");
        created.typed().expect("an item")
    }

    /// Adds a file to the bin, and gives its id.
    pub fn media(&mut self, media: Media) -> MediaId {
        let created = self
            .apply(Command::AddMedia { media })
            .created
            .expect("a file");
        created.typed().expect("a file's id")
    }

    /// The model as it stands.
    pub fn model(&self) -> &Model {
        &self.0
    }
}

/// A place, in seconds.
pub fn at(seconds: f64) -> Tick {
    Tick::from_display_seconds(seconds).expect("a place")
}

/// A length, in seconds.
pub fn long(seconds: f64) -> TickLen {
    TickLen::try_from(at(seconds).get()).expect("a length")
}

/// A probed video file of `seconds`, 1920 by 1080, with sound.
pub fn video(path: &str, seconds: f64) -> Media {
    Media {
        path: path.to_owned(),
        name: path.rsplit('/').next().unwrap_or(path).to_owned(),
        kind: MediaKind::Video,
        length: Some(long(seconds)),
        rate: Some(concat_doc::Rate::FPS_30),
        size: Size::new(1920, 1080),
        has_sound: true,
        ..Media::default()
    }
}
