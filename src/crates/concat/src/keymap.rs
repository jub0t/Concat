// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The keyboard: which chord does what, as one table Rust owns.
//!
//! The window used to spell this out twice — as `KeyBinding` blocks and
//! as a chain of `if`s in app.slint — which meant the shortcut and the
//! menu row naming it could come apart, and neither could be moved
//! without editing the .slint. Now the table is here: the defaults, the
//! resolution of a pressed chord to an action, and the overrides in
//! [`Preferences::keybinds`] that Settings › Shortcuts writes. Menus and
//! the tray ask this module how to write each label, so a rebind moves
//! the key and the words for it together.
//!
//! A chord is lower-case tokens joined by `+`, the modifiers first in a
//! fixed order and then the key: "ctrl+shift+z", "q", "space",
//! "shift+delete", "ctrl+,". "ctrl" is the platform's own primary
//! modifier — Slint's backend stands ⌘ in for Control on a Mac before
//! the event is seen, and [`crate::platform::keys`] writes it back the
//! way that platform does.
//!
//! Some keys never enter the table: the menu bar's F10 and Alt+F/E/V,
//! and the window's Ctrl+O/W/Q stay `KeyBinding`s in app.slint, ahead
//! of everything this module sees. [`reserved`] names their chords so a
//! capture in Settings is turned away rather than shadowing them.

use crate::i18n;
use crate::prefs::Preferences;

/// One action the keyboard reaches, and the chords it ships on.
///
/// The action's *name* is [`name`], not a field: the string inventory
/// that keeps en.json honest reads `t("…")` calls, and a key held as
/// data here would be dropped from it as never asked for.
pub struct Action {
    /// The verb the dispatch tables speak: "undo", "split-tool", ...
    pub id: &'static str,
    /// The chords it ships on. The first is the one the row shows; the
    /// rest are aliases one press should keep working through —
    /// Backspace deletes as Delete does, and the `+` key zooms as `=`
    /// does. An override replaces the whole list: whoever pressed one
    /// chord for an action asked for that chord, not for the aliases
    /// underneath it.
    pub defaults: &'static [&'static str],
}

/// Every action the shortcuts page lists, in the order it lists them.
pub const ACTIONS: &[Action] = &[
    // The verbs a project has from the first moment.
    Action {
        id: "save",
        defaults: &["ctrl+s"],
    },
    Action {
        id: "import",
        defaults: &["ctrl+i"],
    },
    Action {
        id: "export",
        defaults: &["ctrl+e"],
    },
    Action {
        id: "settings",
        defaults: &["ctrl+,"],
    },
    // Editing.
    Action {
        id: "undo",
        defaults: &["ctrl+z"],
    },
    Action {
        id: "redo",
        // Windows and Linux redo on Ctrl+Y as well; see [`resolve`].
        defaults: &["ctrl+shift+z"],
    },
    Action {
        id: "select-all",
        defaults: &["ctrl+a"],
    },
    Action {
        id: "copy",
        defaults: &["ctrl+c"],
    },
    Action {
        id: "paste",
        defaults: &["ctrl+v"],
    },
    Action {
        id: "duplicate",
        defaults: &["ctrl+d"],
    },
    Action {
        id: "split",
        defaults: &["ctrl+b"],
    },
    Action {
        id: "delete",
        defaults: &["delete", "backspace"],
    },
    // Shift closes the gap the deletion leaves — the ripple delete every
    // other editor has, on the key they all use for it.
    // https://github.com/jub0t/Concat/issues/106
    Action {
        id: "ripple-delete",
        defaults: &["shift+delete", "shift+backspace"],
    },
    Action {
        id: "freeze",
        defaults: &["f"],
    },
    // The big editors' trim keys: the selected clip is cut at the
    // playhead, Q dropping its left, E its right.
    Action {
        id: "cut-left",
        defaults: &["q"],
    },
    Action {
        id: "cut-right",
        defaults: &["e"],
    },
    Action {
        id: "split-tool",
        defaults: &["b"],
    },
    Action {
        id: "mute",
        defaults: &["m"],
    },
    // The hand: the pan tool, on and off, the key every editor's canvas
    // grabs with.
    Action {
        id: "pan",
        defaults: &["h"],
    },
    // The view.
    Action {
        id: "snap",
        defaults: &["n"],
    },
    Action {
        id: "preview-axis",
        defaults: &["s"],
    },
    Action {
        id: "zoom-in",
        defaults: &["ctrl+=", "ctrl+plus"],
    },
    Action {
        id: "zoom-out",
        defaults: &["ctrl+-"],
    },
    // The playhead.
    Action {
        id: "play",
        defaults: &["space"],
    },
    Action {
        id: "start",
        defaults: &["home"],
    },
    Action {
        id: "end",
        defaults: &["end"],
    },
    Action {
        id: "step-back",
        defaults: &["left"],
    },
    Action {
        id: "step-forward",
        defaults: &["right"],
    },
];

/// The action a row and a dispatch speak, by its id.
pub fn action(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|action| action.id == id)
}

/// The action's name, in the language the window speaks now: what
/// Settings › Shortcuts puts on the row, and what the toast naming a
/// displaced action calls it. Each key is spelled in a literal call
/// because that is what `scripts/locales.py` reads when it keeps
/// en.json the inventory of the strings the source asks for — a key
/// this function looked up as data would be dropped as never asked.
/// An id no row speaks is a row with no name, which draws nothing.
pub fn name(id: &str) -> String {
    match id {
        "save" => i18n::t("studio.save"),
        "import" => i18n::t("studio.importMedia"),
        "export" => i18n::t("studio.export"),
        "settings" => i18n::t("common.settings"),
        "undo" => i18n::t("studio.undo"),
        "redo" => i18n::t("studio.redo"),
        "select-all" => i18n::t("shortcuts.selectAll"),
        "copy" => i18n::t("common.copy"),
        "paste" => i18n::t("studio.paste"),
        "duplicate" => i18n::t("studio.duplicate"),
        "split" => i18n::t("studio.splitAtPlayhead"),
        "delete" => i18n::t("common.delete"),
        "ripple-delete" => i18n::t("shortcuts.rippleDelete"),
        "freeze" => i18n::t("timelinePane.freeze"),
        "cut-left" => i18n::t("shortcuts.cutLeft"),
        "cut-right" => i18n::t("shortcuts.cutRight"),
        "split-tool" => i18n::t("shortcuts.splitTool"),
        "mute" => i18n::t("studio.mute"),
        "pan" => i18n::t("shortcuts.pan"),
        "snap" => i18n::t("common.snapToEdges"),
        "preview-axis" => i18n::t("shortcuts.previewAxis"),
        "zoom-in" => i18n::t("common.zoomIn"),
        "zoom-out" => i18n::t("common.zoomOut"),
        "play" => i18n::t("previewPane.play"),
        "start" => i18n::t("common.goToStart"),
        "end" => i18n::t("common.goToEnd"),
        "step-back" => i18n::t("shortcuts.stepBack"),
        "step-forward" => i18n::t("shortcuts.stepForward"),
        _ => String::new(),
    }
}

/// The chord `id` is on now: what was bound over the default, the
/// default when nothing was, and `None` when a rebind took the action's
/// key away and left it holding none.
pub fn bound<'a>(prefs: &'a Preferences, id: &str) -> Option<&'a str> {
    let action = action(id)?;
    match prefs.keybinds.get(id) {
        Some(custom) if custom.is_empty() => None,
        Some(custom) => Some(custom.as_str()),
        None => action.defaults.first().copied(),
    }
}

/// The action's chord as this platform's menus write it — "" when the
/// action is unbound, so a menu row simply omits what there is no key
/// for.
pub fn label(prefs: &Preferences, id: &str) -> String {
    bound(prefs, id).map(display).unwrap_or_default()
}

/// Whether `chord` is what `action` currently answers to.
fn answers(prefs: &Preferences, action: &Action, chord: &str) -> bool {
    match prefs.keybinds.get(action.id) {
        Some(custom) => !custom.is_empty() && custom == chord,
        None => action.defaults.contains(&chord),
    }
}

/// Which action a pressed chord takes, overrides before defaults.
///
/// Two things resolve outside the rows: Shift steps ten frames with
/// whatever step key is bound — the variant is a rule about Shift, not
/// a binding of its own — and Ctrl+Y redoes on Windows and Linux while
/// redo still ships where it does, the way the window's old `Ctrl+Y`
/// binding did.
pub fn resolve(prefs: &Preferences, chord: &str) -> Option<&'static str> {
    for action in ACTIONS {
        if answers(prefs, action, chord) {
            return Some(action.id);
        }
    }
    if let Some(base) = chord.strip_prefix("shift+") {
        let held = ACTIONS.iter().find(|action| answers(prefs, action, base));
        match held.map(|action| action.id) {
            Some("step-back") => return Some("step-back-10"),
            Some("step-forward") => return Some("step-forward-10"),
            _ => {}
        }
    }
    if !cfg!(target_os = "macos") && chord == "ctrl+y" && !prefs.keybinds.contains_key("redo") {
        return Some("redo");
    }
    None
}

/// Binds `chord` to `id`, taking it from whoever held it: a chord is
/// one action's, so the loser is left with none — its row reads "—" and
/// its Reset puts it back. Binding an action to one of its own defaults
/// clears the override rather than recording it, so the row reads as
/// shipped until something moves it. Returns the action that lost the
/// chord, if any.
pub fn bind(prefs: &mut Preferences, id: &str, chord: &str) -> Option<&'static str> {
    let action = action(id)?;
    let mut lost = None;
    for other in ACTIONS {
        if other.id != id && answers(prefs, other, chord) {
            prefs.keybinds.insert(other.id.to_owned(), String::new());
            lost = Some(other.id);
        }
    }
    if action.defaults.contains(&chord) {
        prefs.keybinds.remove(id);
    } else {
        prefs.keybinds.insert(id.to_owned(), chord.to_owned());
    }
    lost
}

/// Forgets the override on `id`, taking its shipped chord back from
/// whoever holds it now. Returns the action that lost it, if any.
pub fn reset(prefs: &mut Preferences, id: &str) -> Option<&'static str> {
    let action = action(id)?;
    prefs.keybinds.remove(id);
    let default = action.defaults.first()?;
    let mut lost = None;
    for other in ACTIONS {
        if other.id != id && answers(prefs, other, default) {
            prefs.keybinds.insert(other.id.to_owned(), String::new());
            lost = Some(other.id);
        }
    }
    lost
}

/// Forgets every override: the table as it ships.
pub fn reset_all(prefs: &mut Preferences) {
    prefs.keybinds.clear();
}

/// The chords that are not the table's to give: the menu bar's own keys
/// and the window's, which stay `KeyBinding`s in app.slint ahead of
/// anything this module sees. A capture that asks for one is turned
/// away, because a binding that fired first would make the new
/// shortcut a lie.
pub fn reserved(chord: &str) -> bool {
    // Ctrl+O and Ctrl+W are the window's everywhere; the rest only hold
    // where their bindings are enabled — a Mac's menus and quit are the
    // system's own.
    let apple = cfg!(any(target_os = "macos", target_os = "ios"));
    matches!(chord, "ctrl+o" | "ctrl+w")
        || (!apple && matches!(chord, "ctrl+q" | "f10" | "alt+f" | "alt+e" | "alt+v"))
}

/// The chord the key event makes, or `None` when the press is no chord:
/// a modifier waiting for its key, the Super key or a Mac's physical
/// Control — Slint hands those over as Meta, and nobody binds them —
/// text that is not one character, or a key this table does not name.
pub fn chord(text: &str, shift: bool, alt: bool, control: bool, meta: bool) -> Option<String> {
    if meta {
        return None;
    }
    let mut chars = text.chars();
    let key = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    if is_modifier(key) {
        return None;
    }
    let name = if let Some(named) = named(key) {
        named
    } else if key == '+' {
        // `+` is Shift on most layouts, and the press meant the key
        // whether Shift was held or not — the way `Control + Plus`
        // matched either.
        "plus".to_owned()
    } else if key.is_control() {
        // Control characters the platform sends as text: not keys a
        // person binds.
        return None;
    } else {
        // Lower-cased, so Caps Lock leaves the letters alone.
        key.to_lowercase().to_string()
    };
    let mut chord = String::new();
    if control {
        chord.push_str("ctrl+");
    }
    if alt {
        chord.push_str("alt+");
    }
    if shift && name != "plus" {
        chord.push_str("shift+");
    }
    chord.push_str(&name);
    Some(chord)
}

/// Whether the character is a modifier holding out for the key it
/// modifies: those arrive as key events of their own before it.
fn is_modifier(key: char) -> bool {
    use slint::platform::Key;
    [
        Key::Shift,
        Key::ShiftR,
        Key::Control,
        Key::ControlR,
        Key::Alt,
        Key::AltGr,
        Key::Meta,
        Key::MetaR,
    ]
    .iter()
    .any(|modifier| char::from(*modifier) == key)
}

/// The chord's name for a key that is not a letter or a mark: Slint's
/// own special keys as the lower-cased word the table spells them with.
fn named(key: char) -> Option<String> {
    use slint::platform::Key;
    let name = match key {
        k if k == char::from(Key::Space) => "space",
        k if k == char::from(Key::Delete) => "delete",
        k if k == char::from(Key::Backspace) => "backspace",
        k if k == char::from(Key::Home) => "home",
        k if k == char::from(Key::End) => "end",
        k if k == char::from(Key::UpArrow) => "up",
        k if k == char::from(Key::DownArrow) => "down",
        k if k == char::from(Key::LeftArrow) => "left",
        k if k == char::from(Key::RightArrow) => "right",
        k if k == char::from(Key::Return) => "return",
        k if k == char::from(Key::Escape) => "escape",
        k if k == char::from(Key::Tab) => "tab",
        k if k == char::from(Key::F1) => "f1",
        k if k == char::from(Key::F2) => "f2",
        k if k == char::from(Key::F3) => "f3",
        k if k == char::from(Key::F4) => "f4",
        k if k == char::from(Key::F5) => "f5",
        k if k == char::from(Key::F6) => "f6",
        k if k == char::from(Key::F7) => "f7",
        k if k == char::from(Key::F8) => "f8",
        k if k == char::from(Key::F9) => "f9",
        k if k == char::from(Key::F10) => "f10",
        k if k == char::from(Key::F11) => "f11",
        k if k == char::from(Key::F12) => "f12",
        _ => return None,
    };
    Some(name.to_owned())
}

/// The chord as this platform writes it: "Ctrl+Shift+Z" where that is
/// how it is written, ⇧⌘Z where it is not — through the same
/// [`crate::platform::keys`] the menus use, so a row and a menu never
/// spell one shortcut two ways.
pub fn display(chord: &str) -> String {
    let tokens: Vec<&str> = chord.split('+').filter(|token| !token.is_empty()).collect();
    let Some((key, modifiers)) = tokens.split_last() else {
        return String::new();
    };
    // The two delete keys: ⌫ and ⇧⌫ on a Mac, Del and Shift+Del
    // elsewhere, which is the name each platform's own menus use.
    if *key == "delete" && modifiers.iter().all(|modifier| *modifier == "shift") {
        return crate::platform::delete_key(modifiers.contains(&"shift"));
    }
    let mut parts: Vec<String> = modifiers
        .iter()
        .map(|modifier| match *modifier {
            "ctrl" => "Control".to_owned(),
            "alt" => "Alt".to_owned(),
            "shift" => "Shift".to_owned(),
            "meta" => "Meta".to_owned(),
            // A hand-edited settings.json can say anything; show it.
            other => other.to_owned(),
        })
        .collect();
    parts.push(key_part(key));
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let rendered = crate::platform::keys(&parts);
    if rendered.is_empty() {
        // `from_parts` took no part of it: spell it plainly rather than
        // draw an empty chip in the row.
        parts.join("+")
    } else {
        rendered
    }
}

/// The key's spelling in `@keys` syntax. Letters and marks are
/// themselves — the parser takes a single lower-cased character as a
/// literal — but a key the platform gives a name wants that name, and
/// one whose name is its own character wants the character.
fn key_part(key: &str) -> String {
    if let Some(number) = key.strip_prefix('f')
        && let Ok(number) = number.parse::<u8>()
        && (1..=12).contains(&number)
    {
        return format!("F{number}");
    }
    match key {
        "space" => "Space",
        "delete" => "Delete",
        "backspace" => "Backspace",
        "home" => "Home",
        "end" => "End",
        "up" => "UpArrow",
        "down" => "DownArrow",
        "left" => "LeftArrow",
        "right" => "RightArrow",
        "return" => "Return",
        "escape" => "Escape",
        "tab" => "Tab",
        "plus" => "+",
        other => other,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    /// A pair of keys as the window would hand them over: the text and
    /// the modifiers as Slint reports them after the platform's own
    /// remapping of ⌘ and Control.
    fn press(text: &str, shift: bool, control: bool) -> Option<String> {
        chord(text, shift, false, control, false)
    }

    #[test]
    fn a_chord_is_built_from_the_text_and_the_modifiers() {
        assert_eq!(press("z", false, true).as_deref(), Some("ctrl+z"));
        assert_eq!(press("Z", true, true).as_deref(), Some("ctrl+shift+z"));
        // Caps Lock presses an upper-case letter with no Shift held:
        // the letters stay themselves.
        assert_eq!(press("Q", false, false).as_deref(), Some("q"));
        assert_eq!(press(",", false, true).as_deref(), Some("ctrl+,"));
    }

    #[test]
    fn the_keys_that_are_not_chords_build_nothing() {
        // The Super key, and a Mac's physical Control — both arrive as
        // Meta — and a modifier holding out for its key.
        assert_eq!(chord("z", false, false, true, true), None);
        assert_eq!(chord("z", true, false, false, true), None);
        assert_eq!(
            chord(
                &char::from(slint::platform::Key::Control).to_string(),
                false,
                false,
                false,
                false
            ),
            None
        );
        assert_eq!(
            chord(
                &char::from(slint::platform::Key::Shift).to_string(),
                true,
                false,
                false,
                false
            ),
            None
        );
        // Text that is not one key.
        assert_eq!(chord("", false, false, false, false), None);
        assert_eq!(chord("ab", false, false, false, false), None);
    }

    #[test]
    fn the_special_keys_name_themselves() {
        assert_eq!(press(" ", false, false).as_deref(), Some("space"));
        assert_eq!(
            press(
                &char::from(slint::platform::Key::Delete).to_string(),
                false,
                false
            )
            .as_deref(),
            Some("delete")
        );
        assert_eq!(
            press(
                &char::from(slint::platform::Key::Delete).to_string(),
                true,
                false
            )
            .as_deref(),
            Some("shift+delete")
        );
        assert_eq!(
            press(
                &char::from(slint::platform::Key::LeftArrow).to_string(),
                true,
                false
            )
            .as_deref(),
            Some("shift+left")
        );
        assert_eq!(
            press(
                &char::from(slint::platform::Key::Home).to_string(),
                false,
                false
            )
            .as_deref(),
            Some("home")
        );
        assert_eq!(
            press(
                &char::from(slint::platform::Key::F10).to_string(),
                false,
                false
            )
            .as_deref(),
            Some("f10")
        );
    }

    #[test]
    fn the_plus_key_zooms_whether_shift_was_held_or_not() {
        assert_eq!(press("+", true, true).as_deref(), Some("ctrl+plus"));
        assert_eq!(press("+", false, true).as_deref(), Some("ctrl+plus"));
        assert_eq!(press("=", false, true).as_deref(), Some("ctrl+="));
    }

    #[test]
    fn the_defaults_resolve_to_their_actions() {
        let prefs = Preferences::default();
        assert_eq!(resolve(&prefs, "q"), Some("cut-left"));
        assert_eq!(resolve(&prefs, "ctrl+shift+z"), Some("redo"));
        assert_eq!(resolve(&prefs, "backspace"), Some("delete"));
        assert_eq!(resolve(&prefs, "shift+delete"), Some("ripple-delete"));
        assert_eq!(resolve(&prefs, "left"), Some("step-back"));
        assert_eq!(resolve(&prefs, "ctrl+plus"), Some("zoom-in"));
        assert_eq!(resolve(&prefs, "ctrl+="), Some("zoom-in"));
        assert_eq!(resolve(&prefs, "ctrl+,"), Some("settings"));
        // Unmapped, so nothing takes it.
        assert_eq!(resolve(&prefs, "ctrl+h"), None);
        assert_eq!(resolve(&prefs, "shift+q"), None);
    }

    #[test]
    fn shift_steps_ten_frames_with_whatever_key_steps() {
        let prefs = Preferences::default();
        assert_eq!(resolve(&prefs, "shift+left"), Some("step-back-10"));
        assert_eq!(resolve(&prefs, "shift+right"), Some("step-forward-10"));
        // A step key rebound: the ten follow it. J was nobody's.
        let mut prefs = prefs;
        assert_eq!(bind(&mut prefs, "step-back", "j"), None);
        assert_eq!(resolve(&prefs, "j"), Some("step-back"));
        assert_eq!(resolve(&prefs, "shift+j"), Some("step-back-10"));
    }

    #[test]
    fn ctrl_y_redoes_where_the_platform_redoes_that_way() {
        let found = resolve(&Preferences::default(), "ctrl+y");
        assert_eq!(
            found,
            if cfg!(target_os = "macos") {
                None
            } else {
                Some("redo")
            }
        );
        // The alias is the default's: rebind redo and it goes with it.
        let mut prefs = Preferences::default();
        prefs.keybinds.insert("redo".into(), "ctrl+r".into());
        assert_eq!(resolve(&prefs, "ctrl+y"), None);
    }

    #[test]
    fn a_rebind_moves_the_chord_and_leaves_the_loser_none() {
        let mut prefs = Preferences::default();
        // Mute ships on M; taking M for the trim key leaves mute none.
        assert_eq!(bind(&mut prefs, "cut-left", "m"), Some("mute"));
        assert_eq!(resolve(&prefs, "m"), Some("cut-left"));
        assert_eq!(bound(&prefs, "mute"), None);
        assert_eq!(resolve(&prefs, "q"), None);
        // The override is what is stored, not the whole table.
        assert_eq!(
            prefs.keybinds,
            [
                ("cut-left".to_owned(), "m".to_owned()),
                ("mute".to_owned(), String::new())
            ]
            .into_iter()
            .collect::<BTreeMap<_, _>>()
        );
    }

    #[test]
    fn binding_an_action_back_to_its_default_clears_the_override() {
        let mut prefs = Preferences::default();
        // `bind` hands back whoever lost the chord, and nobody held
        // Ctrl+Shift+S: None here is "no one to take it from".
        bind(&mut prefs, "save", "ctrl+shift+s");
        assert!(prefs.keybinds.contains_key("save"));
        bind(&mut prefs, "save", "ctrl+s");
        assert!(!prefs.keybinds.contains_key("save"));
        assert_eq!(bound(&prefs, "save"), Some("ctrl+s"));
    }

    #[test]
    fn reset_takes_the_shipped_chord_back() {
        let mut prefs = Preferences::default();
        // The trim key took M from mute, leaving mute none.
        assert_eq!(bind(&mut prefs, "cut-left", "m"), Some("mute"));
        assert_eq!(bound(&prefs, "mute"), None);
        assert_eq!(bound(&prefs, "cut-left"), Some("m"));
        // Mute's own Reset puts M back where it ships, taking it off
        // the trim key — a chord is one action's, and mute is the
        // action it belongs to.
        assert_eq!(reset(&mut prefs, "mute"), Some("cut-left"));
        assert_eq!(bound(&prefs, "mute"), Some("m"));
        assert_eq!(bound(&prefs, "cut-left"), None);
        // The trim key's Reset returns it to Q, which nobody holds, so
        // there is no loser to name.
        assert_eq!(reset(&mut prefs, "cut-left"), None);
        assert_eq!(bound(&prefs, "cut-left"), Some("q"));
        assert_eq!(bound(&prefs, "mute"), Some("m"));
        reset_all(&mut prefs);
        assert!(prefs.keybinds.is_empty());
    }

    #[test]
    fn the_windows_own_chords_are_reserved() {
        assert!(reserved("ctrl+o"));
        assert!(reserved("ctrl+w"));
        assert!(!reserved("ctrl+z"));
        assert!(!reserved("alt+m"));
        if cfg!(not(any(target_os = "macos", target_os = "ios"))) {
            assert!(reserved("f10"));
            assert!(reserved("alt+f"));
            assert!(reserved("ctrl+q"));
        }
    }

    #[test]
    fn every_default_displays_the_way_the_platform_writes_it() {
        for action in ACTIONS {
            for default in action.defaults {
                let shown = display(default);
                assert!(!shown.is_empty(), "{default} displays as nothing");
            }
        }
        let undo = display("ctrl+z");
        assert!(undo.to_lowercase().contains('z'), "{undo}");
        assert!(undo.contains("Ctrl") || undo.contains('⌃'), "{undo}");
        let ripple = display("shift+delete");
        assert!(!ripple.is_empty(), "the ripple delete displays as nothing");
        // A row for an action with no chord reads "" for its menu and
        // "—" where the row draws one.
        assert_eq!(display(""), "");
    }

    /// Two actions shipping on one chord would make one of them a lie:
    /// the table resolves the first it finds, and the other would never
    /// fire.
    #[test]
    fn no_two_actions_ship_on_the_same_chord() {
        let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
        for action in ACTIONS {
            for default in action.defaults {
                if let Some(owner) = seen.insert(default, action.id) {
                    panic!("{default} ships for both {owner} and {}", action.id);
                }
            }
        }
    }

    /// A row with no name would draw an empty line where the action's
    /// word belongs, and a name missing from en.json would read as its
    /// own key.
    #[test]
    fn every_action_has_a_name() {
        for action in ACTIONS {
            let named = name(action.id);
            assert!(!named.is_empty(), "{} has no name", action.id);
            assert!(
                !named.starts_with("shortcuts."),
                "{}'s name is a key",
                action.id
            );
        }
    }
}
