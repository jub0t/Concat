// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! AppKit menus and Finder documents, without replacing winit's lifecycle.

use std::cell::RefCell;
use std::path::PathBuf;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{MainThreadMarker, class, msg_send, sel};
use objc2_foundation::{NSArray, NSString, NSURL};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;

use crate::host::Shell;
use crate::i18n::t;
use crate::studio::Studio;
use crate::ui::{MenuItemData, MenuRow};

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    item: MenuItemData,
    chord: String,
}

#[derive(Clone, Debug, PartialEq)]
struct Snapshot {
    titles: [String; 3],
    menus: [Vec<Row>; 3],
}

#[derive(Default)]
struct Menus {
    installed: bool,
    pending: Option<Snapshot>,
    shown: Option<Snapshot>,
    main: Option<Retained<AnyObject>>,
}

thread_local! {
    static MENUS: RefCell<Menus> = RefCell::default();
}

/// Add only the selectors winit does not implement. The subclass has no ivars,
/// so winit's typed delegate and all inherited launch/termination hooks remain
/// valid. Replacing the delegate with a forwarding proxy would break its casts.
pub(crate) fn install() {
    let Some(_main) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: AppKit is initialized by the winit backend, on the main thread.
    unsafe {
        let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
        let Some(delegate): Option<Retained<AnyObject>> = msg_send![&*app, delegate] else {
            log::error!("macOS: no application delegate for document and Dock events");
            return;
        };
        let mut builder = ClassBuilder::new(c"ConcatApplicationDelegate", delegate.class())
            .expect("macOS bridge installed once");
        builder.add_method(
            sel!(application:openURLs:),
            open_urls as extern "C" fn(_, _, _, _),
        );
        builder.add_method(
            sel!(applicationDockMenu:),
            dock_menu as extern "C" fn(_, _, _) -> _,
        );
        builder.add_method(sel!(concatAction:), action as extern "C" fn(_, _, _));
        let subclass = builder.register();
        // SAFETY: a no-ivar subclass of this object's current class. No methods
        // are overridden; AppKit and the delegate are confined to this thread.
        AnyObject::set_class(&delegate, subclass);
        // Refresh AppKit's cached optional delegate selectors before launch.
        let none: Option<&AnyObject> = None;
        let _: () = msg_send![&*app, setDelegate: none];
        let _: () = msg_send![&*app, setDelegate: &*delegate];
        let mut menu = ClassBuilder::new(c"ConcatMainMenu", class!(NSMenu)).unwrap();
        menu.add_method(
            sel!(performKeyEquivalent:),
            key_equivalent as extern "C" fn(_, _, _) -> _,
        );
        menu.register();
    }
    MENUS.with_borrow_mut(|menus| menus.installed = true);
    log::info!("macOS: Finder and Dock callbacks installed");
}

/// Native menu shortcuts are displayed by AppKit, but the editor's keys still
/// go through Slint's focus scope: typing into a field must not split/delete
/// clips or undo an edit. Keep the system application's own ⌘Q/⌘H handling.
extern "C" fn key_equivalent(menu: &AnyObject, _: Sel, event: &AnyObject) -> Bool {
    // SAFETY: NSMenu and NSEvent arguments supplied by AppKit on the main thread.
    unsafe {
        let item: Option<Retained<AnyObject>> = msg_send![menu, itemAtIndex: 0isize];
        let app_menu: Option<Retained<AnyObject>> =
            item.and_then(|item| msg_send![&*item, submenu]);
        if app_menu.is_some_and(|menu| {
            let handled: Bool = msg_send![&*menu, performKeyEquivalent: event];
            handled.as_bool()
        }) {
            return Bool::YES;
        }
        let modifiers: usize = msg_send![event, modifierFlags];
        if modifiers & (1 << 20) != 0 {
            // AppKit does not reliably deliver an unhandled Command key as a
            // keyDown. Dispatch to Slint itself, preserving its field focus.
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let window: Option<Retained<AnyObject>> = msg_send![&*app, keyWindow];
            let responder: Option<Retained<AnyObject>> =
                window.and_then(|window| msg_send![&*window, firstResponder]);
            if let Some(responder) = responder {
                // A native file dialog keeps its own keyboard routing.
                if let Some(view_class) = AnyClass::get(c"WinitView") {
                    let ours: Bool = msg_send![&*responder, isKindOfClass: view_class];
                    if ours.as_bool() {
                        let text: Retained<NSString> =
                            msg_send![event, charactersIgnoringModifiers];
                        Shell::with(|_, app| forward_command(&app, &text.to_string(), modifiers));
                        return Bool::YES;
                    }
                }
            }
        }
        Bool::NO
    }
}

/// Use Slint's public input API so fields handle their own copy/undo shortcuts
/// before the editor's keymap. Physical modifier releases still arrive from
/// winit; do not synthesize releases while Command is actually held down.
pub(crate) fn forward_command(app: &crate::ui::App, text: &str, modifiers: usize) {
    use slint::platform::{Key, WindowEvent};
    for (mask, key) in [
        (1 << 20, Key::Control),
        (1 << 17, Key::Shift),
        (1 << 19, Key::Alt),
        (1 << 18, Key::Meta),
    ] {
        if modifiers & mask != 0 {
            app.window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        }
    }
    app.window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.into() });
    app.window()
        .dispatch_event(WindowEvent::KeyReleased { text: text.into() });
}

extern "C" fn open_urls(_: &AnyObject, _: Sel, _: &AnyObject, urls: &NSArray<NSURL>) {
    let paths: Vec<PathBuf> = urls.iter().filter_map(|url| url.to_file_path()).collect();
    crate::desktop_open::enqueue(paths);
}

extern "C" fn action(_: &AnyObject, _: Sel, item: &AnyObject) {
    // SAFETY: every item with this target stores an NSString representedObject.
    let id: Option<Retained<NSString>> = unsafe { msg_send![item, representedObject] };
    let Some(id) = id.map(|id| id.to_string()) else {
        return;
    };
    // Leave AppKit's menu-tracking loop before borrowing the editor or opening
    // a dialog. The same callback serves both native and in-window commands.
    let _ = slint::invoke_from_event_loop(move || {
        Shell::with(|_, app| {
            if let Some(path) = id.strip_prefix("recent:") {
                app.invoke_start_open_recent(path.into());
                let _ = app.show();
                app.window()
                    .with_winit_window(|window| window.focus_window());
            } else {
                app.invoke_app_menu_selected(id.as_str().into());
            }
        });
    });
}

extern "C" fn dock_menu(delegate: &AnyObject, _: Sel, _: &AnyObject) -> *mut AnyObject {
    let menu = new_menu("");
    // SAFETY: this native menu's explicit missing-project states must win.
    unsafe {
        let _: () = msg_send![&*menu, setAutoenablesItems: Bool::NO];
    }
    Shell::with(|shell, _| {
        let studio = shell.studio.borrow();
        for recent in studio.recents.iter().take(10) {
            let row = new_item(&recent.info.name, Some(sel!(concatAction:)), "");
            // SAFETY: main-thread AppKit menu objects. representedObject is
            // retained by the item, as is its menu ownership of the row.
            unsafe {
                let _: () = msg_send![&*row, setTarget: delegate];
                let _: () = msg_send![&*row, setRepresentedObject: &*NSString::from_str(&format!("recent:{}", recent.info.path))];
                let _: () = msg_send![&*row, setEnabled: Bool::new(recent.present)];
                let selected = studio
                    .session
                    .as_ref()
                    .is_some_and(|session| session.path() == recent.info.path);
                let _: () = msg_send![&*row, setState: isize::from(selected)];
                let _: () = msg_send![&*row, setToolTip: &*NSString::from_str(&recent.info.path)];
                let _: () = msg_send![&*menu, addItem: &*row];
            }
        }
    });
    Retained::autorelease_ptr(menu)
}

/// Cache a model, without entering AppKit while Studio is borrowed. Test
/// backends never install this bridge and therefore never touch AppKit here.
pub(crate) fn publish(studio: &Studio) {
    MENUS.with_borrow_mut(|menus| {
        if !menus.installed {
            return;
        }
        let menus_data = std::array::from_fn(|index| {
            studio
                .menu_bar_items(index as i32)
                .into_iter()
                .map(|mut item| {
                    if studio.on_start
                        && !matches!(item.id.as_str(), "open" | "settings" | "close-window")
                    {
                        item.enabled = false;
                    }
                    let chord = match item.id.as_str() {
                        "open" => "ctrl+o",
                        "close-window" => "ctrl+w",
                        id => crate::keymap::bound(&studio.prefs, id).unwrap_or(""),
                    }
                    .to_owned();
                    Row { item, chord }
                })
                .collect()
        });
        menus.pending = Some(Snapshot {
            titles: [t("titleBar.file"), t("titleBar.edit"), t("titleBar.view")],
            menus: menus_data,
        });
    });
}

/// Called before the loop sleeps, after winit and Slint install their default
/// application menu. Rebuild only when labels, enabled states or keys change.
pub(crate) fn tick() {
    MENUS.with_borrow_mut(|menus| {
        let Some(snapshot) = menus.pending.as_ref() else { return };
        // SAFETY: about_to_wait is the main thread, outside Studio callbacks.
        unsafe {
            let app: Retained<AnyObject> = msg_send![class!(NSApplication), sharedApplication];
            let current: Option<Retained<AnyObject>> = msg_send![&*app, mainMenu];
            // Slint installs its default application menu on window activation.
            // Wait for it rather than letting File become macOS's first item.
            let Some(current_menu) = current.as_ref() else { return };
            let count: isize = msg_send![&**current_menu, numberOfItems];
            if count == 0 { return; }
            if menus.shown.as_ref() == Some(snapshot)
                && current.as_deref().map(std::ptr::from_ref) == menus.main.as_deref().map(std::ptr::from_ref) {
                return;
            }
            let delegate: Retained<AnyObject> = msg_send![&*app, delegate];
            let allocated: Allocated<AnyObject> = msg_send![AnyClass::get(c"ConcatMainMenu").unwrap(), alloc];
            let main: Retained<AnyObject> = msg_send![allocated, initWithTitle: &*NSString::from_str("")];
            // Copy the platform's default app menu (About, Services, Hide,
            // Quit). Slint keeps and reinstalls its own menu on activation:
            // moving the first item out would leave that menu empty next time.
            if let Some(current) = current {
                let count: isize = msg_send![&*current, numberOfItems];
                if count > 0 {
                    let first: Retained<AnyObject> = msg_send![&*current, itemAtIndex: 0isize];
                    let copied: Retained<AnyObject> = msg_send![&*first, copy];
                    let _: () = msg_send![&*main, addItem: &*copied];
                }
            }
            for (title, rows) in snapshot.titles.iter().zip(&snapshot.menus) {
                let submenu = new_menu(title);
                let _: () = msg_send![&*submenu, setAutoenablesItems: Bool::NO];
                for row in rows {
                    if row.item.kind == MenuRow::Separator {
                        let rule: Retained<AnyObject> = msg_send![class!(NSMenuItem), separatorItem];
                        let _: () = msg_send![&*submenu, addItem: &*rule];
                        continue;
                    }
                    let (key, modifiers) = accelerator(&row.chord);
                    let item = new_item(row.item.label.as_str(), Some(sel!(concatAction:)), &key);
                    let _: () = msg_send![&*item, setKeyEquivalentModifierMask: modifiers];
                    let _: () = msg_send![&*item, setTarget: &*delegate];
                    let _: () = msg_send![&*item, setRepresentedObject: &*NSString::from_str(row.item.id.as_str())];
                    let _: () = msg_send![&*item, setEnabled: Bool::new(row.item.enabled)];
                    let _: () = msg_send![&*item, setState: isize::from(row.item.checkable && row.item.checked)];
                    let _: () = msg_send![&*submenu, addItem: &*item];
                }
                let item = new_item(title, None, "");
                let _: () = msg_send![&*item, setSubmenu: &*submenu];
                let _: () = msg_send![&*main, addItem: &*item];
            }
            let _: () = msg_send![&*app, setMainMenu: &*main];
            menus.main = Some(main);
            menus.shown = Some(snapshot.clone());
        }
    });
    crate::desktop_open::tick();
}

fn new_menu(title: &str) -> Retained<AnyObject> {
    // SAFETY: NSMenu creation on the main thread; returned object is owned.
    unsafe {
        let menu: Allocated<AnyObject> = msg_send![class!(NSMenu), alloc];
        msg_send![menu, initWithTitle: &*NSString::from_str(title)]
    }
}

fn new_item(title: &str, action: Option<Sel>, key: &str) -> Retained<AnyObject> {
    // SAFETY: NSMenuItem initializer; action is nil or our registered selector.
    unsafe {
        let item: Allocated<AnyObject> = msg_send![class!(NSMenuItem), alloc];
        msg_send![item, initWithTitle: &*NSString::from_str(title), action: action, keyEquivalent: &*NSString::from_str(key)]
    }
}

fn accelerator(chord: &str) -> (String, usize) {
    let mut parts: Vec<&str> = chord.split('+').collect();
    let key = parts.pop().unwrap_or("");
    let mut flags = 0usize;
    for modifier in parts {
        flags |= match modifier {
            "ctrl" => 1 << 20, // NSEventModifierFlagCommand: the editor's primary modifier.
            "shift" => 1 << 17,
            "alt" => 1 << 19,
            "meta" => 1 << 18,
            _ => 0,
        };
    }
    let key = match key {
        "delete" => "\u{f728}",
        "backspace" => "\u{8}",
        "home" => "\u{f729}",
        "end" => "\u{f72b}",
        "space" => " ",
        "plus" => "+",
        key if key.chars().count() <= 1 => key,
        _ => "",
    };
    (key.to_owned(), flags)
}
