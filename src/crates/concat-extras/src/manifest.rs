// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Concat contributors

//! `extra.toml`: what an extra is, what it brings, what it needs, and every
//! file it carries.
//!
//! ```toml
//! format = 1
//!
//! [extra]
//! id = "studio.cinematic-transitions"
//! name = "Cinematic Transitions"
//! version = "1.0.0"
//! summary = "Twelve camera-move transitions."
//! author = "A Studio"
//! licence = "CC-BY-4.0"
//! category = "transitions"
//! min_app = "0.2.11"
//!
//! [[provides]]
//! kind = "effects"          # effect packages: filters, effects, transitions, sound
//! root = "effects"          # the folder inside the archive that holds them
//!
//! [[requires]]
//! id = "studio.base-looks"
//! version = ">=1.2"
//! reason = "Shares its colour tables."
//!
//! [[file]]
//! path = "effects/studio.whip-pan/effect.toml"
//! size = 812
//! sha256 = "…"
//! ```
//!
//! The `[[file]]` list is the whole of what the archive may hold besides the
//! manifest: anything unlisted, any listed file that is missing, and any
//! file whose size or digest differs refuses the archive. `concat-cli extras
//! pack` writes the list; an author writes the rest.
//!
//! Every rule an archive's contents must keep is checked here, on the
//! manifest, before a byte of the payload is read: a path that could escape
//! the folder it lands in, a name Windows reserves, a file type the kind it
//! sits under does not take, sizes past the limits.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::Error;

/// The manifest format this build reads and writes.
pub const FORMAT: u32 = 1;

/// The manifest's name, at the root of the archive and of an installed
/// extra's folder.
pub const MANIFEST_FILE: &str = "extra.toml";

/// The most files an extra may carry.
pub const MAX_FILES: usize = 20_000;

/// The largest one file may be: enough for a big model, not for a disk.
pub const MAX_FILE_SIZE: u64 = 4 * 1024 * 1024 * 1024;

/// The most an extra may unpack to, in all.
pub const MAX_TOTAL_SIZE: u64 = 8 * 1024 * 1024 * 1024;

/// The largest a manifest may be.
pub const MAX_MANIFEST_SIZE: u64 = 4 * 1024 * 1024;

/// The longest path inside an extra. Short enough that an installed extra
/// stays inside Windows' classic path limit under a long user name.
pub const MAX_PATH_LEN: usize = 160;

/// The files an extra may carry outside every root: about the extra, not
/// part of what it provides.
pub const ROOT_FILES: [&str; 10] = [
    "icon.png",
    "preview.png",
    "preview.jpg",
    "README.md",
    "CHANGELOG.md",
    "LICENSE",
    "LICENSE.txt",
    "LICENSE.md",
    "NOTICE",
    "NOTICE.txt",
];

/// What part of Concat an extra's contents go to: the extension point a
/// root is handed to.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    /// Effect packages, each a folder with `effect.toml`: filters, effects,
    /// transitions, generators and sound effects - everything the effects
    /// catalogue loads. A transition is an effect package in Concat.
    Effects,
    /// Font files the font picker offers.
    Fonts,
    /// Title presets: `*.toml`, or folders with `preset.toml` and the font
    /// they bring.
    TitlePresets,
    /// Still images to place: stickers, overlays, backgrounds.
    Stickers,
    /// Sound files: effects, ambiences, music beds.
    Sounds,
}

impl Kind {
    /// Every kind.
    pub const ALL: [Kind; 5] = [
        Kind::Effects,
        Kind::Fonts,
        Kind::TitlePresets,
        Kind::Stickers,
        Kind::Sounds,
    ];

    /// The name a manifest writes.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Effects => "effects",
            Kind::Fonts => "fonts",
            Kind::TitlePresets => "title-presets",
            Kind::Stickers => "stickers",
            Kind::Sounds => "sounds",
        }
    }

    /// The file extensions a root of this kind may hold, lowercase.
    /// Everything is data some existing part of Concat reads: shaders are
    /// compiled and trialled by the effects catalogue, and nothing an extra
    /// carries is ever executed.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Effects => &[
                "toml", "wgsl", "cube", "png", "jpg", "jpeg", "webp", "md", "txt",
            ],
            Kind::Fonts => &["ttf", "otf", "ttc", "md", "txt"],
            Kind::TitlePresets => &["toml", "ttf", "otf", "ttc", "png", "md", "txt"],
            // What the media pipeline decodes as a still and the cards can
            // draw: no SVG (nothing here rasterises it) and no GIF (it
            // would come in as a clip of its own frames, not a sticker).
            Kind::Stickers => &["png", "jpg", "jpeg", "webp", "md", "txt"],
            Kind::Sounds => &[
                "wav", "mp3", "flac", "ogg", "opus", "m4a", "aac", "aif", "aiff", "md", "txt",
            ],
        }
    }
}

/// Where an extra is shelved in the Extras sheet. Display only: what an
/// extra does is its [`Kind`]s.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    /// Looks, filters and visual effects.
    Effects,
    /// Transitions.
    Transitions,
    /// Fonts and typography.
    Fonts,
    /// Title presets and text templates.
    Titles,
    /// Stickers, overlays and other images.
    Stickers,
    /// Sound effects, audio effects and presets.
    Audio,
    /// Anything else.
    #[default]
    Other,
}

impl Category {
    /// Every category, in the order the sheet shelves them.
    pub const ALL: [Category; 7] = [
        Category::Effects,
        Category::Transitions,
        Category::Fonts,
        Category::Titles,
        Category::Stickers,
        Category::Audio,
        Category::Other,
    ];

    /// The name a manifest writes.
    pub fn key(self) -> &'static str {
        match self {
            Category::Effects => "effects",
            Category::Transitions => "transitions",
            Category::Fonts => "fonts",
            Category::Titles => "titles",
            Category::Stickers => "stickers",
            Category::Audio => "audio",
            Category::Other => "other",
        }
    }
}

/// The whole of `extra.toml`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The manifest format; [`FORMAT`] for this build.
    pub format: u32,
    /// What the extra is.
    pub extra: Info,
    /// What it brings, and where in the archive.
    #[serde(default)]
    pub provides: Vec<Provide>,
    /// What must be installed for it to work.
    #[serde(default)]
    pub requires: Vec<Requirement>,
    /// Every file besides the manifest.
    #[serde(default, rename = "file")]
    pub files: Vec<FileEntry>,
}

/// The `[extra]` table.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Info {
    /// Unique and stable: `author.name`, lowercase letters, digits, `.`
    /// and `-`.
    pub id: String,
    /// What the card says.
    pub name: String,
    /// Semantic version.
    pub version: String,
    /// One line under the name.
    #[serde(default)]
    pub summary: String,
    /// Paragraphs for the detail page.
    #[serde(default)]
    pub description: String,
    /// Who made it.
    pub author: String,
    /// The licence its contents come under, as an SPDX id where there is one.
    pub licence: String,
    /// Where to read more.
    #[serde(default)]
    pub homepage: String,
    /// Where it is shelved.
    #[serde(default)]
    pub category: Category,
    /// The oldest Concat it works in; empty for any.
    #[serde(default)]
    pub min_app: String,
    /// The platforms it is for, as [`crate::current_platform`] names them;
    /// empty for every one. Content is the same everywhere, so this is
    /// rarely set.
    #[serde(default)]
    pub platforms: Vec<String>,
    /// Words search should find it by.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// The ids of what it provides that a project can name - effect ids -
    /// so a project that uses one can say which extra to install. Effect
    /// ids are also read from the packages themselves at install.
    #[serde(default)]
    pub provides_ids: Vec<String>,
}

/// One `[[provides]]`: a folder in the archive, handed to one kind.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Provide {
    /// The extension point.
    pub kind: Kind,
    /// The folder, relative to the archive's root.
    pub root: String,
}

/// One `[[requires]]`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    /// The extra needed.
    pub id: String,
    /// The versions that will do, as a semver range ("^1.2", ">=1.0, <2");
    /// empty for any.
    #[serde(default)]
    pub version: String,
    /// Why, in a sentence, for the install plan.
    #[serde(default)]
    pub reason: String,
}

impl Requirement {
    /// The range, parsed; any version when none is written.
    pub fn range(&self) -> Result<semver::VersionReq, Error> {
        if self.version.trim().is_empty() {
            return Ok(semver::VersionReq::STAR);
        }
        semver::VersionReq::parse(&self.version).map_err(|error| {
            Error::Invalid(format!(
                "the requirement on {} names `{}`, which is not a version range: {error}",
                self.id, self.version
            ))
        })
    }
}

/// One `[[file]]`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    /// Relative to the archive's root, with forward slashes.
    pub path: String,
    /// In bytes.
    pub size: u64,
    /// Lowercase hex.
    pub sha256: String,
}

impl Manifest {
    /// Parses and validates a manifest.
    pub fn parse(text: &str) -> Result<Manifest, Error> {
        let manifest: Manifest = toml::from_str(text)
            .map_err(|error| Error::Invalid(format!("{MANIFEST_FILE} does not parse: {error}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// The manifest as `extra.toml` text.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).expect("a manifest always serialises")
    }

    /// The extra's id.
    pub fn id(&self) -> &str {
        &self.extra.id
    }

    /// The version, parsed. Validation has already made sure it parses.
    pub fn version(&self) -> semver::Version {
        semver::Version::parse(&self.extra.version)
            .unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }

    /// The kinds it provides, once each.
    pub fn kinds(&self) -> Vec<Kind> {
        let mut kinds: Vec<Kind> = self.provides.iter().map(|provide| provide.kind).collect();
        kinds.sort();
        kinds.dedup();
        kinds
    }

    /// What every file adds up to.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// Checks every rule the format makes, so that nothing read later has to.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |message: String| Err(Error::Invalid(message));
        if self.format == 0 || self.format > FORMAT {
            return Err(Error::Incompatible(format!(
                "{} is written in extra format {}; this Concat reads format {FORMAT} - a newer Concat is needed",
                self.extra.id, self.format
            )));
        }
        let info = &self.extra;
        check_id(&info.id)?;
        let id = info.id.as_str();
        if semver::Version::parse(&info.version).is_err() {
            return invalid(format!(
                "{id}: `{}` is not a version (it should read like 1.0.0)",
                info.version
            ));
        }
        check_text(id, "name", &info.name, 1, 80)?;
        check_text(id, "summary", &info.summary, 0, 200)?;
        check_text(id, "description", &info.description, 0, 8000)?;
        check_text(id, "author", &info.author, 1, 80)?;
        check_text(id, "licence", &info.licence, 1, 80)?;
        if !info.homepage.is_empty() && !info.homepage.starts_with("https://") {
            return invalid(format!("{id}: the homepage must be an https:// address"));
        }
        if !info.min_app.is_empty() && semver::Version::parse(&info.min_app).is_err() {
            return invalid(format!("{id}: min_app `{}` is not a version", info.min_app));
        }
        for platform in &info.platforms {
            if !KNOWN_PLATFORMS.contains(&platform.as_str()) {
                return invalid(format!(
                    "{id}: `{platform}` is not a platform Concat runs on"
                ));
            }
        }
        if info.keywords.len() > 32 || info.keywords.iter().any(|word| word.len() > 40) {
            return invalid(format!("{id}: too many keywords, or one too long"));
        }
        if info.provides_ids.len() > 4096 {
            return invalid(format!("{id}: provides_ids lists too many ids"));
        }

        // What it provides.
        if self.provides.is_empty() {
            return invalid(format!(
                "{id}: it provides nothing - add a [[provides]] table"
            ));
        }
        let mut roots: Vec<(&str, Kind)> = Vec::new();
        for provide in &self.provides {
            safe_path(&provide.root).map_err(|why| {
                Error::Invalid(format!("{id}: the root `{}` {why}", provide.root))
            })?;
            for (held, _) in &roots {
                if nested(held, &provide.root) || nested(&provide.root, held) {
                    return invalid(format!(
                        "{id}: the roots `{held}` and `{}` overlap; each kind needs a folder of its own",
                        provide.root
                    ));
                }
            }
            roots.push((&provide.root, provide.kind));
        }

        // What it needs.
        let mut needed = HashSet::new();
        for requirement in &self.requires {
            check_id(&requirement.id)?;
            if requirement.id == info.id {
                return invalid(format!("{id}: it requires itself"));
            }
            if !needed.insert(requirement.id.as_str()) {
                return invalid(format!("{id}: it requires {} twice", requirement.id));
            }
            requirement.range()?;
            check_text(id, "a requirement's reason", &requirement.reason, 0, 200)?;
        }

        // What it carries.
        if self.files.is_empty() {
            return invalid(format!("{id}: it carries no files"));
        }
        if self.files.len() > MAX_FILES {
            return invalid(format!(
                "{id}: it carries {} files; an extra may carry {MAX_FILES}",
                self.files.len()
            ));
        }
        let mut seen = HashSet::new();
        let mut total: u64 = 0;
        for file in &self.files {
            safe_path(&file.path)
                .map_err(|why| Error::Invalid(format!("{id}: the file `{}` {why}", file.path)))?;
            // Windows and macOS fold case: two names that differ only in
            // case would land on one file.
            if !seen.insert(file.path.to_lowercase()) {
                return invalid(format!("{id}: `{}` is listed twice", file.path));
            }
            if file.path.eq_ignore_ascii_case(MANIFEST_FILE) {
                return invalid(format!("{id}: the manifest does not list itself"));
            }
            if !crate::is_sha256(&file.sha256) {
                return invalid(format!(
                    "{id}: `{}` has no SHA-256 (64 lowercase hex digits)",
                    file.path
                ));
            }
            if file.size > MAX_FILE_SIZE {
                return invalid(format!(
                    "{id}: `{}` is larger than one file may be",
                    file.path
                ));
            }
            total = total.saturating_add(file.size);
            let kind = roots
                .iter()
                .find(|(root, _)| nested(root, &file.path))
                .map(|(_, kind)| *kind);
            match kind {
                Some(kind) => {
                    let extension = extension(&file.path);
                    if !kind.extensions().contains(&extension.as_str()) {
                        return invalid(format!(
                            "{id}: `{}` is not a file {} take (they take {})",
                            file.path,
                            kind.key(),
                            kind.extensions().join(", ")
                        ));
                    }
                }
                None if ROOT_FILES.contains(&file.path.as_str()) => {}
                None => {
                    return invalid(format!(
                        "{id}: `{}` is under no [[provides]] root",
                        file.path
                    ));
                }
            }
        }
        if total > MAX_TOTAL_SIZE {
            return invalid(format!("{id}: it unpacks to more than an extra may"));
        }
        // Folders the files imply must not collide with a file of the
        // same name.
        for file in &self.files {
            let lower = file.path.to_lowercase();
            if self
                .files
                .iter()
                .any(|other| other.path.to_lowercase().starts_with(&format!("{lower}/")))
            {
                return invalid(format!("{id}: `{}` is both a file and a folder", file.path));
            }
        }
        Ok(())
    }

    /// Whether this build of Concat on this machine can use the extra; the
    /// reason when not.
    pub fn compatible(&self, app_version: &str, platform: &str) -> Result<(), Error> {
        compatible(
            &self.extra.min_app,
            &self.extra.platforms,
            app_version,
            platform,
        )
        .map_err(Error::Incompatible)
    }
}

/// Every platform a manifest may name.
pub const KNOWN_PLATFORMS: [&str; 9] = [
    "windows-x86_64",
    "windows-aarch64",
    "macos-x86_64",
    "macos-aarch64",
    "linux-x86_64",
    "linux-aarch64",
    "android-aarch64",
    "android-x86_64",
    "ios-aarch64",
];

/// The compatibility rule, shared by a manifest and an index entry.
pub(crate) fn compatible(
    min_app: &str,
    platforms: &[String],
    app_version: &str,
    platform: &str,
) -> Result<(), String> {
    if !min_app.is_empty()
        && let (Ok(needed), Ok(have)) = (
            semver::Version::parse(min_app),
            semver::Version::parse(app_version),
        )
        && have < needed
    {
        return Err(format!(
            "it needs Concat {min_app} or newer; this is {app_version}"
        ));
    }
    if !platforms.is_empty() && !platforms.iter().any(|p| p == platform) {
        return Err(format!(
            "it is made for {}, not {platform}",
            platforms.join(", ")
        ));
    }
    Ok(())
}

/// Checks an extra's id: 3 to 64 characters of lowercase letters, digits,
/// `.` and `-`, with at least one dot (an author and a name), and no empty
/// part.
pub fn check_id(id: &str) -> Result<(), Error> {
    let fine = (3..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && id.contains('.')
        && id
            .split('.')
            .all(|part| !part.is_empty() && !part.starts_with('-') && !part.ends_with('-'));
    if fine {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "`{id}` is not an extra id: it should read like author.name, in lowercase letters, digits, dots and dashes"
        )))
    }
}

fn check_text(id: &str, field: &str, text: &str, min: usize, max: usize) -> Result<(), Error> {
    let count = text.chars().count();
    if count < min {
        return Err(Error::Invalid(format!("{id}: the {field} is empty")));
    }
    if count > max {
        return Err(Error::Invalid(format!(
            "{id}: the {field} is longer than {max} characters"
        )));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(Error::Invalid(format!(
            "{id}: the {field} holds control characters"
        )));
    }
    Ok(())
}

/// Whether `path` is `root` or lies under it.
pub fn nested(root: &str, path: &str) -> bool {
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The lowercase extension of the last part of a path; "" for none.
pub fn extension(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => extension.to_ascii_lowercase(),
        _ => String::new(),
    }
}

/// Checks that `path` names a place inside the folder it is joined to, on
/// every platform: relative, forward slashes only, no `.` or `..` part, no
/// drive letter or stream (`:`), no character Windows refuses, no name
/// Windows reserves for a device (`CON`, `NUL`, `COM1`…), no part ending in
/// a dot or a space (which Windows silently drops, so two names could be
/// one file). The reason, when not.
pub fn safe_path(path: &str) -> Result<(), String> {
    if path.is_empty() {
        return Err("is empty".to_owned());
    }
    if path.len() > MAX_PATH_LEN {
        return Err(format!("is longer than {MAX_PATH_LEN} characters"));
    }
    if path.starts_with('/') {
        return Err("is absolute".to_owned());
    }
    for c in path.chars() {
        if c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
            return Err(format!("holds the character {c:?}, which a path may not"));
        }
    }
    for part in path.split('/') {
        if part.is_empty() {
            return Err("has an empty part (a doubled or trailing /)".to_owned());
        }
        if part == "." || part == ".." {
            return Err("leaves its folder (a `.` or `..` part)".to_owned());
        }
        if part.ends_with('.') || part.ends_with(' ') || part.starts_with(' ') {
            return Err(
                "has a part that starts with a space or ends with a dot or a space".to_owned(),
            );
        }
        // Windows reads "NUL .txt" as NUL too, and keeps the console's
        // names and COM¹-COM³, LPT¹-LPT³ as well.
        let stem = part
            .split('.')
            .next()
            .unwrap_or(part)
            .trim_end()
            .to_ascii_uppercase();
        let numbered = stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|number| {
                (number.len() == 1 && number.as_bytes()[0].is_ascii_digit())
                    || matches!(number, "¹" | "²" | "³")
            });
        let reserved = numbered
            || matches!(
                stem.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            );
        if reserved {
            return Err(format!("uses `{part}`, a name Windows keeps for a device"));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    pub(crate) fn manifest(id: &str) -> Manifest {
        Manifest {
            format: FORMAT,
            extra: Info {
                id: id.to_owned(),
                name: "A Pack".to_owned(),
                version: "1.0.0".to_owned(),
                summary: String::new(),
                description: String::new(),
                author: "Someone".to_owned(),
                licence: "CC0-1.0".to_owned(),
                homepage: String::new(),
                category: Category::Effects,
                min_app: String::new(),
                platforms: Vec::new(),
                keywords: Vec::new(),
                provides_ids: Vec::new(),
            },
            provides: vec![Provide {
                kind: Kind::Effects,
                root: "effects".to_owned(),
            }],
            requires: Vec::new(),
            files: vec![FileEntry {
                path: "effects/a.b/effect.toml".to_owned(),
                size: 0,
                sha256: SHA.to_owned(),
            }],
        }
    }

    fn refused(manifest: &Manifest, needle: &str) {
        let error = manifest.validate().expect_err(needle).to_string();
        assert!(error.contains(needle), "wanted {needle:?} in {error:?}");
    }

    #[test]
    fn a_plain_manifest_round_trips_through_toml() {
        let manifest = manifest("someone.pack");
        manifest.validate().expect("valid");
        let again = Manifest::parse(&manifest.to_toml()).expect("parses");
        assert_eq!(again, manifest);
    }

    #[test]
    fn unknown_fields_and_a_newer_format_are_refused() {
        let text = manifest("someone.pack")
            .to_toml()
            .replace("format = 1", "format = 2");
        let error = Manifest::parse(&text).expect_err("newer").to_string();
        assert!(error.contains("newer Concat"), "{error}");
        let text = format!("{}\nsurprise = true\n", manifest("someone.pack").to_toml());
        assert!(Manifest::parse(&text).is_err());
    }

    #[test]
    fn ids_are_author_dot_name_in_lowercase() {
        for good in ["a.b", "studio.cinematic-transitions", "x1.y-2.z3"] {
            check_id(good).expect(good);
        }
        for bad in [
            "ab",
            "nodot",
            "Upper.case",
            "a..b",
            ".a.b",
            "a.b.",
            "a.-b",
            "a b.c",
            "a/b.c",
        ] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn paths_stay_inside_their_folder_on_every_platform() {
        for good in [
            "effects/a.b/effect.toml",
            "fonts/Inter Bold.ttf",
            "icon.png",
        ] {
            safe_path(good).expect(good);
        }
        for (bad, why) in [
            ("", "empty"),
            ("/etc/passwd", "absolute"),
            ("../x", "leaves"),
            ("a/../../x", "leaves"),
            ("a/./b", "leaves"),
            ("a//b", "empty part"),
            ("a/", "empty part"),
            ("C:/Windows/x", "':'"),
            ("a\\..\\b", "'\\\\'"),
            ("file.txt:stream", "':'"),
            ("fonts/CON.ttf", "device"),
            ("fonts/com1", "device"),
            ("fonts/lpt9.txt", "device"),
            ("fonts/conin$.txt", "device"),
            ("fonts/CONOUT$", "device"),
            ("fonts/COM¹.txt", "device"),
            ("fonts/NUL .txt", "device"),
            ("fonts/name.", "ends with a dot"),
            ("fonts/name ", "space"),
            ("a\u{0}b", "character"),
        ] {
            let error = safe_path(bad).expect_err(bad);
            assert!(error.contains(why), "{bad:?}: {error}");
        }
        assert!(safe_path(&"a/".repeat(100)).is_err(), "too long");
        // Not a device: COM on its own, or a longer word.
        safe_path("fonts/COMPACT.ttf").expect("a word that starts like a device");
        safe_path("fonts/COM10.ttf").expect("only one digit makes a device");
        safe_path("fonts/console.ttf").expect("a word that starts like CON");
    }

    #[test]
    fn every_file_must_sit_under_a_root_of_a_kind_that_takes_it() {
        let mut m = manifest("someone.pack");
        m.files[0].path = "effects/a.b/run.exe".to_owned();
        refused(&m, "not a file effects take");
        m.files[0].path = "elsewhere/effect.toml".to_owned();
        refused(&m, "under no [[provides]] root");
        m.files[0].path = "README.md".to_owned();
        m.validate().expect("a readme at the root is fine");
        m.files[0].path = "effectsplus/x.toml".to_owned();
        refused(&m, "under no [[provides]] root");
    }

    #[test]
    fn files_are_listed_once_with_a_digest_and_never_twice_by_case() {
        let mut m = manifest("someone.pack");
        m.files.push(FileEntry {
            path: "effects/A.B/EFFECT.toml".to_owned(),
            size: 0,
            sha256: SHA.to_owned(),
        });
        refused(&m, "listed twice");
        let mut m = manifest("someone.pack");
        m.files[0].sha256 = "nope".to_owned();
        refused(&m, "SHA-256");
        let mut m = manifest("someone.pack");
        m.files.clear();
        refused(&m, "no files");
        let mut m = manifest("someone.pack");
        m.files.push(FileEntry {
            path: "effects/a.b/effect.toml/inner.toml".to_owned(),
            size: 0,
            sha256: SHA.to_owned(),
        });
        refused(&m, "both a file and a folder");
    }

    #[test]
    fn roots_may_not_overlap_and_requirements_are_well_formed() {
        let mut m = manifest("someone.pack");
        m.provides.push(Provide {
            kind: Kind::Fonts,
            root: "effects/fonts".to_owned(),
        });
        refused(&m, "overlap");

        let mut m = manifest("someone.pack");
        m.requires.push(Requirement {
            id: "someone.pack".to_owned(),
            version: String::new(),
            reason: String::new(),
        });
        refused(&m, "requires itself");
        let mut m = manifest("someone.pack");
        m.requires.push(Requirement {
            id: "other.pack".to_owned(),
            version: "not a range".to_owned(),
            reason: String::new(),
        });
        refused(&m, "not a version range");
    }

    #[test]
    fn compatibility_reads_the_app_version_and_the_platform() {
        let mut m = manifest("someone.pack");
        m.compatible("0.2.11", "windows-x86_64").expect("no limits");
        m.extra.min_app = "9.0.0".to_owned();
        let error = m
            .compatible("0.2.11", "windows-x86_64")
            .expect_err("too old")
            .to_string();
        assert!(error.contains("9.0.0"), "{error}");
        m.extra.min_app = "0.2.0".to_owned();
        m.extra.platforms = vec!["linux-x86_64".to_owned()];
        assert!(m.compatible("0.2.11", "windows-x86_64").is_err());
        m.compatible("0.2.11", "linux-x86_64")
            .expect("its platform");
        m.extra.platforms = vec!["amiga-68k".to_owned()];
        refused(&m, "not a platform");
    }

    #[test]
    fn text_fields_have_limits() {
        let mut m = manifest("someone.pack");
        m.extra.name = String::new();
        refused(&m, "name is empty");
        let mut m = manifest("someone.pack");
        m.extra.summary = "x".repeat(201);
        refused(&m, "summary is longer");
        let mut m = manifest("someone.pack");
        m.extra.author = "a\u{7}b".to_owned();
        refused(&m, "control characters");
        let mut m = manifest("someone.pack");
        m.extra.homepage = "http://example.com".to_owned();
        refused(&m, "https://");
    }

    #[test]
    fn extensions_are_read_from_the_last_part() {
        assert_eq!(extension("a/b/c.TTF"), "ttf");
        assert_eq!(extension("a.b/README"), "");
        assert_eq!(extension("a/.hidden"), "");
        assert!(nested("effects", "effects/x"));
        assert!(!nested("effects", "effectsx/y"));
    }
}
