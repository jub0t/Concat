// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Looks made from `.cube` tables.
//!
//! A table from a grading tool becomes a package folder of its own: a
//! manifest, a shader that reads the table and nothing else, and the table
//! beside them. The package is a look - a filter, mixed by intensity - drawn
//! on the GPU in the space its table was made for: the display encoding, for
//! nearly every table there is (a creative look, or a camera's log to Rec.
//! 709 over footage read as it was recorded), or log for a table made for
//! ACEScct. Either way a level past the table's ends is carried on past
//! them (`look()` in the shader prelude), so a highlight keeps its place
//! above the table's white.
//!
//! Earlier builds made a format 1 package of a table, run through the
//! wrapper that hands it the gamma-encoded picture; [`upgrade`] rewrites
//! those onto the format 2 look, the same table read the same way on SDR.

use std::path::Path;

use crate::manifest::{Kind, Manifest, Space};

/// What a look's table is called in its folder.
pub const TABLE: &str = "look.cube";

/// The shader every look made from a table runs.
pub const SHADER: &str = "// The table, carried past its ends, and nothing else; the host mixes it\n\
    // by intensity.\n\
    fn effect(uv: vec2<f32>) -> vec4<f32> {\n    let c = sample(uv);\n    return vec4<f32>(look(c.rgb), c.a);\n}\n";

/// The shader earlier builds wrote for a table, as it reads with its
/// comments and spacing taken out.
const FORMAT_1_SHADER: &str =
    "fneffect(uv:vec2<f32>)->vec4<f32>{letc=sample(uv);returnvec4<f32>(lut(c.rgb),c.a);}";

/// The space a table is read in: log for one made for ACEScct, which its
/// file's name or its title says, and the display encoding for the rest.
pub fn space_of(path: &Path, table: &str) -> Space {
    let title = table
        .lines()
        .find_map(|line| line.trim().strip_prefix("TITLE"))
        .unwrap_or("");
    let name = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let said = |text: &str| text.to_ascii_lowercase().contains("acescct");
    if said(&name) || said(title) {
        Space::Log
    } else {
        Space::Display
    }
}

/// The manifest of the look `id`, called `name`, reading the table `file`
/// in `space`.
pub fn manifest(id: &str, name: &str, file: &str, space: Space) -> String {
    let space = match space {
        Space::Log => "log",
        Space::Linear | Space::Display => "display",
    };
    format!(
        "format = 2\n\n[effect]\nid = \"{id}\"\nname = {name:?}\nkind = \"filter\"\ncategory = \"Imported\"\n\
         description = \"A look imported from a .cube table.\"\n\n[lut]\nfile = {file:?}\n\n\
         [wgsl]\nentry = \"effect.wgsl\"\nspace = \"{space}\"\n"
    )
}

/// Makes a package folder under `dir` from the table at `path`, and
/// returns the package's id: `user.` and the file's name, slugged. A second
/// import of the same name replaces the first.
pub fn import(dir: &Path, path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut slug: String = stem
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = "look".to_owned();
    }
    let id = format!("user.{slug}");
    let text = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    crate::cube::parse(&text)?;
    let folder = dir.join(&id);
    std::fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let written = manifest(&id, stem.trim(), TABLE, space_of(path, &text));
    std::fs::write(folder.join("effect.toml"), written).map_err(|error| error.to_string())?;
    std::fs::write(folder.join("effect.wgsl"), SHADER).map_err(|error| error.to_string())?;
    std::fs::write(folder.join(TABLE), &text).map_err(|error| error.to_string())?;
    // A still an earlier import drew for this name: the card drawn from the
    // package's own shader replaces it.
    let _ = std::fs::remove_file(folder.join("preview.png"));
    Ok(id)
}

/// Rewrites every look an earlier build made from a table in `dir` onto
/// the format 2 look, in the display encoding it was read in, and returns
/// their ids. A look is known by its shape - a filter of the user's with a
/// table, no knobs, and the one-line shader the import wrote - so a package
/// someone wrote by hand is left alone, and a look already rewritten is not
/// touched again. One whose folder cannot be written keeps loading as it
/// was.
pub fn upgrade(dir: &Path) -> Vec<String> {
    let Ok(folders) = crate::catalogue::package_folders(dir) else {
        return Vec::new();
    };
    let mut upgraded = Vec::new();
    for folder in folders {
        let read = |name: &str| std::fs::read_to_string(folder.join(name)).ok();
        let (Some(text), Some(shader)) = (read("effect.toml"), read("effect.wgsl")) else {
            continue;
        };
        let Ok(old) = Manifest::parse(&text) else {
            continue;
        };
        let chained = old
            .ffmpeg
            .as_ref()
            .is_none_or(|ffmpeg| ffmpeg.chain == "lut3d=file={lut}" && ffmpeg.lets.is_empty());
        let Some(table) = &old.lut else {
            continue;
        };
        let imported = !old.scene_linear()
            && old.effect.kind == Kind::Filter
            && old.effect.id.starts_with("user.")
            && old.params.is_empty()
            && old.wgsl.is_some()
            && chained
            && bare(&shader) == FORMAT_1_SHADER;
        if !imported {
            continue;
        }
        let written = manifest(
            &old.effect.id,
            &old.effect.name,
            &table.file,
            Space::Display,
        );
        let wrote = std::fs::write(folder.join("effect.wgsl"), SHADER)
            .and_then(|()| std::fs::write(folder.join("effect.toml"), written));
        if wrote.is_ok() {
            let _ = std::fs::remove_file(folder.join("preview.png"));
            upgraded.push(old.effect.id.clone());
        }
    }
    upgraded
}

/// A shader's text with its comments and whitespace taken out: what two
/// spellings of one shader have in common.
fn bare(shader: &str) -> String {
    shader
        .lines()
        .map(|line| line.split("//").next().unwrap_or(""))
        .flat_map(str::chars)
        .filter(|c| !c.is_whitespace())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{Catalogue, Package};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("concat-looks-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch folder");
        dir
    }

    fn table(title: &str) -> String {
        let mut text = format!("TITLE \"{title}\"\nLUT_3D_SIZE 2\n");
        for b in 0..2 {
            for g in 0..2 {
                for r in 0..2 {
                    text.push_str(&format!("{r} {} {b}\n", g as f32 * 0.5));
                }
            }
        }
        text
    }

    /// An imported table is a format 2 look that loads, reads its table in
    /// the display encoding or, for one made for ACEScct, in log, and
    /// carries a level past the table's ends.
    #[test]
    fn a_table_imports_as_a_format_2_look() {
        let dir = scratch("import");
        let source = dir.join("Teal & Orange v2.cube");
        std::fs::write(&source, table("Teal")).expect("a table");
        let looks = dir.join("looks");
        let id = import(&looks, &source).expect("imports");
        assert_eq!(id, "user.teal-orange-v2");
        let package = Package::from_folder(&looks.join(&id)).expect("loads");
        assert!(package.manifest.scene_linear());
        assert_eq!(package.manifest.effect.name, "Teal & Orange v2");
        assert_eq!(
            package.manifest.wgsl.as_ref().map(|wgsl| wgsl.space),
            Some(Space::Display)
        );
        assert!(
            package
                .shader()
                .is_some_and(|shader| shader.source().contains("look(c.rgb)"))
        );
        assert_eq!(package.lut().map(|lut| lut.size), Some(2));

        let log = dir.join("Film.cube");
        std::fs::write(&log, table("Film emulation ACEScct")).expect("a table");
        let id = import(&looks, &log).expect("imports");
        let package = Package::from_folder(&looks.join(&id)).expect("loads");
        assert_eq!(
            package.manifest.wgsl.as_ref().map(|wgsl| wgsl.space),
            Some(Space::Log)
        );
        assert!(import(&looks, &dir.join("missing.cube")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A look an earlier build imported is rewritten onto the format 2 look,
    /// once; a package of the same shape written by hand with anything
    /// more in it is left as it was.
    #[test]
    fn an_earlier_import_is_rewritten_once() {
        let dir = scratch("upgrade");
        let write = |id: &str, shader: &str, chain: bool| {
            let folder = dir.join(id);
            std::fs::create_dir_all(&folder).expect("a folder");
            let chain = if chain {
                "[ffmpeg]\nchain = \"lut3d=file={lut}\"\n\n"
            } else {
                ""
            };
            std::fs::write(
                folder.join("effect.toml"),
                format!(
                    "format = 1\n\n[effect]\nid = \"{id}\"\nname = \"Old\"\nkind = \"filter\"\n\
                     category = \"Imported\"\ndescription = \"A look imported from a .cube table.\"\n\n\
                     [lut]\nfile = \"look.cube\"\n\n{chain}[wgsl]\nentry = \"effect.wgsl\"\n"
                ),
            )
            .expect("a manifest");
            std::fs::write(folder.join("effect.wgsl"), shader).expect("a shader");
            std::fs::write(folder.join(TABLE), table("Old")).expect("a table");
            std::fs::write(folder.join("preview.png"), b"old").expect("a still");
        };
        let imported = "// The table, and nothing else; the host mixes it by intensity.\n\
            fn effect(uv: vec2<f32>) -> vec4<f32> {\n    let c = sample(uv);\n    return vec4<f32>(lut(c.rgb), c.a);\n}\n";
        write("user.old", imported, true);
        write(
            "user.older",
            "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(lut(c.rgb), c.a); }",
            false,
        );
        write(
            "user.mine",
            "fn effect(uv: vec2<f32>) -> vec4<f32> { let c = sample(uv); return vec4<f32>(lut(c.rgb) * 0.9, c.a); }",
            true,
        );
        let mut upgraded = upgrade(&dir);
        upgraded.sort();
        assert_eq!(upgraded, ["user.old", "user.older"]);
        assert!(upgrade(&dir).is_empty(), "once");
        let package = Package::from_folder(&dir.join("user.old")).expect("loads");
        assert!(package.manifest.scene_linear());
        assert_eq!(package.manifest.effect.name, "Old");
        assert!(package.manifest.ffmpeg.is_none());
        assert!(!dir.join("user.old").join("preview.png").exists());
        let mine = Package::from_folder(&dir.join("user.mine")).expect("loads");
        assert!(!mine.manifest.scene_linear(), "a package of someone's own");
        let mut catalogue = Catalogue::new();
        assert!(catalogue.load_dir(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
