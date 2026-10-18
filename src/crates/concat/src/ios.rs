// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The iPhone's ways in and out: the system's pickers for Import, and
//! Photos for a finished export.
//!
//! Until this, iOS installed no picker at all, and Import did nothing on
//! every iPhone the app was ever put on (#332, #338). An iPhone's videos
//! live in Photos, and its other files in Files, which are two different
//! pickers - PHPickerViewController and UIDocumentPickerViewController -
//! so Import asks which, with a sheet naming the two. Neither needs a
//! permission: PHPicker runs outside the app and hands over only what was
//! picked, and the document picker copies what was picked into the app's
//! own tmp/ (`asCopy`).
//!
//! What either answers with is moved into Documents/Imported, as
//! Android's picker copies into its Imported: the engine opens files by
//! path, and a project keeps pointing at its media long after the picker's
//! temporary copies are gone. Documents is also the one folder Files shows
//! for the app (UIFileSharingEnabled in scripts/ios-app.sh).
//!
//! Everything here is Objective-C through `objc2`'s `msg_send!`, against
//! classes looked up by name: PhotosUI and Photos have no binding crate in
//! the build, and UIKit's would take a feature per class for the handful
//! of calls made. The window's crate already links UIKit through Slint's
//! winit backend; the three frameworks below are linked here.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use block2::{Block, RcBlock};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{MainThreadMarker, MainThreadOnly, class, define_class, msg_send};
use objc2_foundation::{NSArray, NSError, NSString, NSURL};

use crate::i18n::t;
use crate::platform::PickKinds;

#[link(name = "PhotosUI", kind = "framework")]
#[link(name = "Photos", kind = "framework")]
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
#[link(name = "AVFAudio", kind = "framework")]
unsafe extern "C" {}

/// Sets the app's audio session to play and record at once, for a
/// voiceover: the microphone open while the timeline plays. An iPhone's
/// session plays only until told otherwise, and an input opened under it
/// hears silence. The timeline goes on playing from the speaker rather
/// than the earpiece the category would otherwise pick, and a Bluetooth
/// headset's microphone may be the input. Asked each time the voiceover
/// panel opens; setting what is already set costs nothing.
pub fn record_alongside_playback() -> Result<(), String> {
    /// AVAudioSessionCategoryOptions: DefaultToSpeaker | AllowBluetooth.
    const OPTIONS: usize = 0x8 | 0x4;
    let class = AnyClass::get(c"AVAudioSession").ok_or("this device has no audio session")?;
    // SAFETY: `sharedInstance` takes nothing and returns the app's session;
    // the two setters take what their selectors name and report through
    // the trailing error, which `error: _` turns into the Result.
    let session: Option<Retained<AnyObject>> = unsafe { msg_send![class, sharedInstance] };
    let session = session.ok_or("this device has no audio session")?;
    // The category constant's value is its own name.
    let category = NSString::from_str("AVAudioSessionCategoryPlayAndRecord");
    let set: Result<(), Retained<NSError>> =
        unsafe { msg_send![&session, setCategory: &*category, withOptions: OPTIONS, error: _] };
    set.map_err(|error| error.localizedDescription().to_string())?;
    let active: Result<(), Retained<NSError>> =
        unsafe { msg_send![&session, setActive: true, error: _] };
    active.map_err(|error| error.localizedDescription().to_string())
}

type Picked = Box<dyn FnOnce(Vec<PathBuf>) + Send>;

/// The pick in flight, waiting for a picker to answer.
static PENDING: Mutex<Option<Picked>> = Mutex::new(None);

/// What the pick in flight asked for: media, which comes from Photos or
/// Files, or files by extension - subtitles, fonts - which only Files has.
static WANTED: Mutex<Option<PickKinds>> = Mutex::new(None);

/// What the pick in flight asked for; every kind of media when unsaid.
fn wanted() -> PickKinds {
    WANTED
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or_else(|| PickKinds::of(None))
}

thread_local! {
    /// The pickers' delegate. A view controller holds its delegate weakly,
    /// so something else has to; one for the life of the app, made on the
    /// first pick, on the main thread where every picker lives.
    static DELEGATE: std::cell::OnceCell<Retained<PickDelegate>> =
        const { std::cell::OnceCell::new() };
}

/// Installs the iPhone's picker and its export publisher into the window's
/// crate. Before the window runs; see `platform::install_file_picker`.
pub fn install() {
    crate::platform::install_file_picker(Box::new(|kinds, on_picked| {
        let previous = PENDING
            .lock()
            .map(|mut slot| slot.replace(on_picked))
            .ok()
            .flatten();
        if let Some(previous) = previous {
            // A pick was already up; the one that asked first is told it
            // got nothing rather than left waiting for ever.
            previous(Vec::new());
        }
        // Files by extension, and sound, are not in Photos: straight to
        // Files.
        let files_only = match &kinds {
            PickKinds::Files(_) => true,
            PickKinds::Media { video, image, .. } => !video && !image,
        };
        if let Ok(mut slot) = WANTED.lock() {
            *slot = Some(kinds);
        }
        // `pick_files_async` is called from the event loop, which on iOS
        // is the main thread; UIKit is not to be touched from any other.
        let shown = match MainThreadMarker::new() {
            Some(mtm) if files_only => show_files(mtm),
            Some(mtm) => choose(mtm),
            None => Err("asked for a picker off the main thread"),
        };
        if let Err(error) = shown {
            log::error!("could not show the picker: {error}");
            finish(Vec::new());
        }
    }));
    crate::platform::install_export_publisher(crate::platform::ExportPublisher {
        folder: "Photos".into(),
        publish: Box::new(save_to_photos),
        publish_document: Box::new(|path, _mime| keep_in_exports(path)),
    });
}

/// Hands the pick in flight what it got.
fn finish(paths: Vec<PathBuf>) {
    if let Some(pending) = PENDING.lock().ok().and_then(|mut slot| slot.take()) {
        pending(paths);
    }
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class
    // has no Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ConcatPickDelegate"]
    struct PickDelegate;

    impl PickDelegate {
        /// PHPickerViewControllerDelegate: called once, with nothing for
        /// a cancel. The picker does not go away by itself.
        #[unsafe(method(picker:didFinishPicking:))]
        fn picker_did_finish_picking(&self, picker: &AnyObject, results: &NSArray<AnyObject>) {
            let none: Option<&Block<dyn Fn()>> = None;
            // SAFETY: a view controller, on the main thread.
            let _: () = unsafe { msg_send![picker, dismissViewControllerAnimated: true, completion: none] };
            photos_picked(results);
        }

        /// UIDocumentPickerDelegate: the copies the picker made, in tmp/.
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn document_picker_did_pick(&self, _picker: &AnyObject, urls: &NSArray<NSURL>) {
            let picked: Vec<PathBuf> = urls.iter().filter_map(|url| url.to_file_path()).collect();
            // Off the main thread: a move within the app's container is a
            // rename, but one that falls back to a copy takes a while.
            let moved = std::thread::Builder::new()
                .name("ios import".into())
                .spawn(move || {
                    let paths = picked
                        .iter()
                        .filter_map(|from| {
                            let name = from.file_name()?.to_string_lossy().into_owned();
                            adopt(from, &name)
                        })
                        .collect();
                    finish(paths);
                });
            if moved.is_err() {
                finish(Vec::new());
            }
        }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn document_picker_was_cancelled(&self, _picker: &AnyObject) {
            finish(Vec::new());
        }
    }
);

impl PickDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(());
        // SAFETY: NSObject's init.
        unsafe { msg_send![super(this), init] }
    }

    /// The one delegate, as an object a picker's `setDelegate:` takes.
    fn shared(mtm: MainThreadMarker) -> Retained<AnyObject> {
        DELEGATE.with(|cell| {
            let delegate = cell.get_or_init(|| Self::new(mtm));
            Retained::into_super(Retained::into_super(delegate.clone()))
        })
    }
}

/// The view controller to present on: the key window's root, or what it
/// already shows on top of itself.
fn top_view_controller() -> Result<Retained<AnyObject>, &'static str> {
    // SAFETY: UIKit's own getters, on the main thread (the caller's
    // MainThreadMarker); each answer is nil or an object of the type named.
    unsafe {
        let app: Option<Retained<AnyObject>> = msg_send![class!(UIApplication), sharedApplication];
        let app = app.ok_or("no UIApplication")?;
        let scenes: Retained<AnyObject> = msg_send![&app, connectedScenes];
        let scenes: Retained<NSArray<AnyObject>> = msg_send![&scenes, allObjects];
        let window_scene = AnyClass::get(c"UIWindowScene").ok_or("no UIWindowScene")?;
        let mut root: Option<Retained<AnyObject>> = None;
        for scene in scenes.iter() {
            let is_window_scene: bool = msg_send![&scene, isKindOfClass: window_scene];
            if !is_window_scene {
                continue;
            }
            let key: Option<Retained<AnyObject>> = msg_send![&scene, keyWindow];
            let window = match key {
                Some(window) => Some(window),
                None => {
                    let windows: Retained<NSArray<AnyObject>> = msg_send![&scene, windows];
                    windows.firstObject()
                }
            };
            if let Some(window) = window {
                root = msg_send![&window, rootViewController];
                if root.is_some() {
                    break;
                }
            }
        }
        let mut top = root.ok_or("no window with a root view controller")?;
        loop {
            let presented: Option<Retained<AnyObject>> = msg_send![&top, presentedViewController];
            match presented {
                Some(presented) => top = presented,
                None => return Ok(top),
            }
        }
    }
}

/// Asks where from - Photos or Files - and shows that picker. A sheet from
/// the bottom on an iPhone; an iPad's would be a popover, which needs a
/// place on the screen to point at, so there it is an alert in the middle.
fn choose(_mtm: MainThreadMarker) -> Result<(), &'static str> {
    let top = top_view_controller()?;
    let title = NSString::from_str(&t("phone.importMedia"));
    let message: Option<&NSString> = None;
    // SAFETY: UIKit on the main thread; the constants are UIKit's
    // (UIUserInterfaceIdiomPad 1, UIAlertControllerStyleActionSheet 0 and
    // Alert 1, UIAlertActionStyleDefault 0 and Cancel 1).
    unsafe {
        let device: Retained<AnyObject> = msg_send![class!(UIDevice), currentDevice];
        let idiom: isize = msg_send![&device, userInterfaceIdiom];
        let style: isize = if idiom == 1 { 1 } else { 0 };
        let alert: Retained<AnyObject> = msg_send![
            class!(UIAlertController),
            alertControllerWithTitle: &*title,
            message: message,
            preferredStyle: style
        ];
        let actions: [(
            String,
            isize,
            fn(MainThreadMarker) -> Result<(), &'static str>,
        ); 3] = [
            (t("phone.fromPhotos"), 0, show_photos),
            (t("phone.fromFiles"), 0, show_files),
            (t("common.cancel"), 1, |_| Err("cancelled")),
        ];
        for (label, action_style, show) in actions {
            let label = NSString::from_str(&label);
            let handler = RcBlock::new(move |_action: *mut AnyObject| {
                // UIKit calls an action's handler on the main thread, once
                // the sheet is gone.
                let shown = match MainThreadMarker::new() {
                    Some(mtm) => show(mtm),
                    None => Err("an alert action off the main thread"),
                };
                if let Err(error) = shown {
                    if error != "cancelled" {
                        log::error!("could not show the picker: {error}");
                    }
                    finish(Vec::new());
                }
            });
            let action: Retained<AnyObject> = msg_send![
                class!(UIAlertAction),
                actionWithTitle: &*label,
                style: action_style,
                handler: &*handler
            ];
            let _: () = msg_send![&alert, addAction: &*action];
        }
        let none: Option<&Block<dyn Fn()>> = None;
        let _: () =
            msg_send![&top, presentViewController: &*alert, animated: true, completion: none];
    }
    Ok(())
}

/// The photo library's picker: videos and pictures, any number, as they
/// are in the library. `Current` rather than `Automatic` so an HEVC video
/// comes over as it is instead of being transcoded first, which for a long
/// clip is minutes of a spinner; the engine decodes HEVC and HEIC itself.
fn show_photos(mtm: MainThreadMarker) -> Result<(), &'static str> {
    let top = top_view_controller()?;
    let delegate = PickDelegate::shared(mtm);
    // SAFETY: PhotosUI on the main thread; the constants are its own
    // (PHPickerConfigurationAssetRepresentationModeCurrent 1; a selection
    // limit of 0 is no limit).
    unsafe {
        let config: Allocated<AnyObject> = msg_send![class!(PHPickerConfiguration), alloc];
        let config: Retained<AnyObject> = msg_send![config, init];
        let _: () = msg_send![&config, setSelectionLimit: 0isize];
        let _: () = msg_send![&config, setPreferredAssetRepresentationMode: 1isize];
        // Photos holds videos and pictures; a pick for sound alone never
        // comes here (`install` goes straight to Files for it).
        let (video, image) = match wanted() {
            PickKinds::Media { video, image, .. } => (video, image),
            PickKinds::Files(_) => (false, false),
        };
        let mut filters: Vec<Retained<AnyObject>> = Vec::new();
        if video || !image {
            let videos: Retained<AnyObject> = msg_send![class!(PHPickerFilter), videosFilter];
            filters.push(videos);
        }
        if image || !video {
            let images: Retained<AnyObject> = msg_send![class!(PHPickerFilter), imagesFilter];
            filters.push(images);
        }
        let both = NSArray::from_retained_slice(&filters);
        let filter: Retained<AnyObject> =
            msg_send![class!(PHPickerFilter), anyFilterMatchingSubfilters: &*both];
        let _: () = msg_send![&config, setFilter: &*filter];
        let picker: Allocated<AnyObject> = msg_send![class!(PHPickerViewController), alloc];
        let picker: Retained<AnyObject> = msg_send![picker, initWithConfiguration: &*config];
        let _: () = msg_send![&picker, setDelegate: &*delegate];
        let none: Option<&Block<dyn Fn()>> = None;
        let _: () =
            msg_send![&top, presentViewController: &*picker, animated: true, completion: none];
    }
    Ok(())
}

/// The document picker: what the pick asked for from Files - iCloud
/// Drive, the phone, a USB drive, anything a file provider offers. Media
/// by its families; anything else - a subtitle, a font - by the type its
/// extension names, which for one the system has never heard of is a
/// type made from the extension, and still matches a file that has it.
/// The picker copies each into the app's tmp/ (`asCopy`), so nothing
/// needs a security-scoped bookmark to be read.
fn show_files(mtm: MainThreadMarker) -> Result<(), &'static str> {
    let top = top_view_controller()?;
    let delegate = PickDelegate::shared(mtm);
    // SAFETY: UIKit and UniformTypeIdentifiers on the main thread.
    unsafe {
        let mut types = Vec::new();
        match wanted() {
            PickKinds::Media {
                video,
                audio,
                image,
            } => {
                for (on, identifier) in [
                    (video, "public.movie"),
                    (audio, "public.audio"),
                    (image, "public.image"),
                ] {
                    if !on {
                        continue;
                    }
                    let identifier = NSString::from_str(identifier);
                    let kind: Option<Retained<AnyObject>> =
                        msg_send![class!(UTType), typeWithIdentifier: &*identifier];
                    types.extend(kind);
                }
            }
            PickKinds::Files(extensions) => {
                for extension in extensions {
                    let extension = NSString::from_str(&extension);
                    let kind: Option<Retained<AnyObject>> =
                        msg_send![class!(UTType), typeWithFilenameExtension: &*extension];
                    types.extend(kind);
                }
            }
        }
        if types.is_empty() {
            // Nothing the system could name: any file at all, rather than
            // a picker with nothing in it.
            let identifier = NSString::from_str("public.item");
            let kind: Option<Retained<AnyObject>> =
                msg_send![class!(UTType), typeWithIdentifier: &*identifier];
            types.extend(kind);
        }
        let types = NSArray::from_retained_slice(&types);
        let picker: Allocated<AnyObject> = msg_send![class!(UIDocumentPickerViewController), alloc];
        let picker: Retained<AnyObject> =
            msg_send![picker, initForOpeningContentTypes: &*types, asCopy: true];
        let _: () = msg_send![&picker, setAllowsMultipleSelection: true];
        let _: () = msg_send![&picker, setDelegate: &*delegate];
        let none: Option<&Block<dyn Fn()>> = None;
        let _: () =
            msg_send![&top, presentViewController: &*picker, animated: true, completion: none];
    }
    Ok(())
}

/// What the photo picker answered with, each fetched as a file and moved
/// into Imported. Every fetch answers on a queue of its own, in no
/// particular order; the pick is answered once all have, in the order
/// they were picked.
fn photos_picked(results: &NSArray<AnyObject>) {
    let count = results.count();
    if count == 0 {
        finish(Vec::new());
        return;
    }
    struct Tally {
        left: usize,
        paths: Vec<Option<PathBuf>>,
    }
    let tally = Arc::new(Mutex::new(Tally {
        left: count,
        paths: vec![None; count],
    }));
    let done = |tally: &Mutex<Tally>, index: usize, path: Option<PathBuf>| {
        let finished = tally.lock().ok().and_then(|mut tally| {
            tally.paths[index] = path;
            tally.left -= 1;
            (tally.left == 0).then(|| tally.paths.drain(..).flatten().collect::<Vec<_>>())
        });
        if let Some(paths) = finished {
            finish(paths);
        }
    };
    for (index, result) in results.iter().enumerate() {
        // SAFETY: a PHPickerResult's itemProvider is an NSItemProvider,
        // never nil; the identifiers are system UTIs.
        let asked = unsafe {
            let provider: Retained<AnyObject> = msg_send![&result, itemProvider];
            let movie = NSString::from_str("public.movie");
            let has_movie: bool = msg_send![&provider, hasItemConformingToTypeIdentifier: &*movie];
            let kind = if has_movie {
                movie
            } else {
                NSString::from_str("public.image")
            };
            let suggested: Option<Retained<NSString>> = msg_send![&provider, suggestedName];
            let suggested = suggested.map(|name| name.to_string());
            let tally = Arc::clone(&tally);
            // The file is the provider's for as long as this block runs,
            // and is deleted after; it is moved or copied before then.
            let handler = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
                let from = url.as_ref().and_then(|url| url.to_file_path());
                let path = match from {
                    Some(from) => {
                        let name = import_name(suggested.as_deref(), &from);
                        adopt(&from, &name)
                    }
                    None => {
                        let why = error
                            .as_ref()
                            .map(|error| error.localizedDescription().to_string())
                            .unwrap_or_default();
                        log::warn!("could not fetch a picked photo or video: {why}");
                        None
                    }
                };
                done(&tally, index, path);
            });
            let progress: Option<Retained<AnyObject>> = msg_send![
                &provider,
                loadFileRepresentationForTypeIdentifier: &*kind,
                completionHandler: &*handler
            ];
            progress.is_some()
        };
        if !asked {
            done(&tally, index, None);
        }
    }
}

/// The name a picked photo or video is kept under: the library's own
/// ("IMG_0042"), with the extension of the file it came as, or the file's
/// name where the library gave none.
fn import_name(suggested: Option<&str>, file: &Path) -> String {
    let file_name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "import".into());
    match (suggested.filter(|name| !name.is_empty()), file.extension()) {
        (Some(name), Some(ext)) => format!("{name}.{}", ext.to_string_lossy()),
        (Some(name), None) => name.to_owned(),
        (None, _) => file_name,
    }
}

/// Moves `from` into Documents/Imported as `name` - or `name (2)` and on
/// where that is taken - and answers with where it went. A copy where the
/// move fails (another volume, a file the app may read but not move).
fn adopt(from: &Path, name: &str) -> Option<PathBuf> {
    let Some(home) = std::env::home_dir() else {
        log::warn!("no home folder to import into");
        return None;
    };
    let dir = home.join("Documents").join("Imported");
    if let Err(error) = std::fs::create_dir_all(&dir) {
        log::warn!("could not make {}: {error}", dir.display());
        return None;
    }
    // Whatever the provider called it, it is one file name.
    let name = name.replace(['/', '\\'], "_");
    let to = unique(&dir, &name);
    if std::fs::rename(from, &to).is_ok() {
        return Some(to);
    }
    match std::fs::copy(from, &to) {
        Ok(_) => Some(to),
        Err(error) => {
            log::warn!("could not import {}: {error}", from.display());
            let _ = std::fs::remove_file(&to);
            None
        }
    }
}

/// Moves a written document - a subtitle file - into Documents/Exports,
/// beside the exported videos, under its own name or `name (2)` where that
/// is taken; and answers with where Files shows it: On My iPhone, Concat,
/// Exports.
fn keep_in_exports(from: &Path) -> Result<String, String> {
    let home = std::env::home_dir().ok_or("no home folder to keep it in")?;
    let dir = home.join("Documents").join("Exports");
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let name = from
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .ok_or("the file has no name")?;
    let to = unique(&dir, &name);
    if std::fs::rename(from, &to).is_err() {
        std::fs::copy(from, &to).map_err(|error| error.to_string())?;
        let _ = std::fs::remove_file(from);
    }
    let kept = to
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(name);
    Ok(format!("Concat › Exports › {kept}"))
}

/// `name`, or `name (2)`, `name (3)`... until nothing is overwritten.
fn unique(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rfind('.') {
        Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
        _ => (name, ""),
    };
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|path| !path.exists())
        .unwrap_or(first)
}

/// Adds a finished export to Photos, where an iPhone looks for its videos,
/// and answers with the file's own path. The file stays where it was
/// written, in Documents, which Files shows: Photos keeps its own copy,
/// and a save Photos refused loses nothing. Blocking - Photos copies the
/// whole video - so it is called off the main thread, which
/// `performChangesAndWait:` requires.
fn save_to_photos(path: &Path) -> Result<String, String> {
    // PHAccessLevelAddOnly 1; PHAuthorizationStatus NotDetermined 0,
    // Authorized 3, Limited 4.
    const ADD_ONLY: isize = 1;
    // SAFETY: Photos' class methods, from any thread; the block is called
    // once, from a queue of Photos'.
    let mut status: isize =
        unsafe { msg_send![class!(PHPhotoLibrary), authorizationStatusForAccessLevel: ADD_ONLY] };
    if status == 0 {
        let (tx, rx) = std::sync::mpsc::channel();
        let handler = RcBlock::new(move |answer: isize| {
            let _ = tx.send(answer);
        });
        // SAFETY: as above.
        let _: () = unsafe {
            msg_send![
                class!(PHPhotoLibrary),
                requestAuthorizationForAccessLevel: ADD_ONLY,
                handler: &*handler
            ]
        };
        status = rx.recv().unwrap_or(0);
    }
    if status != 3 && status != 4 {
        return Err("Concat may not add to Photos; allow it under Settings, Concat, Photos".into());
    }
    let url = NSURL::from_file_path(path).ok_or("the export's path is not a file URL")?;
    let change = RcBlock::new(move || {
        // SAFETY: a PHAssetChangeRequest, made inside a change block as
        // Photos requires.
        let _: Option<Retained<AnyObject>> = unsafe {
            msg_send![
                class!(PHAssetChangeRequest),
                creationRequestForAssetFromVideoAtFileURL: &*url
            ]
        };
    });
    // SAFETY: Photos' shared library; `performChangesAndWait:error:` off
    // the main thread, its block called before it returns.
    let saved: Result<(), Retained<NSError>> = unsafe {
        let library: Retained<AnyObject> = msg_send![class!(PHPhotoLibrary), sharedPhotoLibrary];
        msg_send![&library, performChangesAndWait: &*change, error: _]
    };
    saved
        .map(|()| path.to_string_lossy().into_owned())
        .map_err(|error| error.localizedDescription().to_string())
}

/// Opens a web address in Safari, or in whichever app claims it: the
/// launch screen's guides and the repository. From the main thread, where
/// the window's callbacks run.
pub fn open_link(url: &str) -> Result<(), String> {
    let text = NSString::from_str(url);
    // SAFETY: NSURL's class constructor, and UIApplication's shared
    // instance and `openURL:options:completionHandler:` on the main
    // thread; the options are an empty dictionary and the handler nil,
    // both of which the method takes.
    unsafe {
        let link: Option<Retained<NSURL>> = msg_send![class!(NSURL), URLWithString: &*text];
        let link = link.ok_or_else(|| format!("{url} is not a web address"))?;
        let app: Option<Retained<AnyObject>> = msg_send![class!(UIApplication), sharedApplication];
        let app = app.ok_or("no UIApplication")?;
        let options: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
        let handler: Option<&Block<dyn Fn(objc2::runtime::Bool)>> = None;
        let _: () = msg_send![
            &app,
            openURL: &*link,
            options: &*options,
            completionHandler: handler
        ];
    }
    Ok(())
}
