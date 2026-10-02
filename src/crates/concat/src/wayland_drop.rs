// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Files dropped on the window from outside it, on native Wayland.
//!
//! winit 0.30 reports a dropped file on macOS, Windows and X11 and not on
//! Wayland: its Wayland backend never asks the seat for a data device, so
//! a drag from a file manager over the window is simply not heard
//! (<https://github.com/jub0t/Concat/issues/158>, #196). The window asks
//! for one itself. It joins the connection winit opened - the same
//! `wl_display`, as a guest that does not close it - with an event queue
//! of its own on a thread of its own, binds the seat and the data device
//! manager from a fresh registry, and takes a data device for the seat.
//! The compositor then tells that device about every drag over this
//! client's surfaces: when the offer carries `text/uri-list` it is
//! accepted, and on the drop the list is read through a pipe, decoded to
//! paths and handed to the window's drop handler, which treats them as it
//! treats the paths winit reports elsewhere.
//!
//! Reading the display is shared with winit the way libwayland means it to
//! be: prepare, poll the socket, read once for every queue, dispatch what
//! landed on this one. The thread wakes every fifth of a second to see
//! whether it has been told to stop, which it is before winit tears the
//! display down.

use std::ffi::c_void;
use std::io::Read;
use std::os::fd::{AsRawFd, BorrowedFd, FromRawFd, OwnedFd};
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};

/// The one type of drag the window takes: a list of file URIs.
const URI_LIST: &str = "text/uri-list";
/// How long the thread sleeps on the socket before looking at its stop
/// flag, in milliseconds.
const TICK_MS: i32 = 200;
/// How long a drop's source gets to write the list before it is given up
/// on, in milliseconds.
const SOURCE_MS: i32 = 2000;

/// The listening thread. Dropping it stops the thread and waits for it.
pub struct Listener {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Starts listening on `display`, the `*mut wl_display` winit opened,
    /// for drops on the surface at `surface`, winit's `*mut wl_surface`;
    /// the paths of each drop go to `sink`. None when the thread could not
    /// be spawned.
    ///
    /// # Safety
    ///
    /// `display` must stay alive until the listener is dropped. winit
    /// keeps its display for the life of the event loop, and the handler
    /// drops the listener as the loop exits.
    pub unsafe fn start(
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
        sink: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) -> Option<Listener> {
        // Which of this client's surfaces is the window: a drag over a
        // popup is not a drop on the editor. Unknowable means any.
        // SAFETY: the caller's surface pointer is winit's live proxy.
        let surface = unsafe {
            wayland_client::backend::ObjectId::from_ptr(
                WlSurface::interface(),
                surface.as_ptr().cast(),
            )
        }
        .ok()
        .map(|id| id.protocol_id());
        let stop = Arc::new(AtomicBool::new(false));
        let flag = Arc::clone(&stop);
        // A raw pointer is not Send; its address is.
        let address = display.as_ptr() as usize;
        let thread = std::thread::Builder::new()
            .name("concat wayland drops".into())
            .spawn(move || listen(address, surface, Box::new(sink), flag))
            .ok()?;
        Some(Listener {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The mime types an offer has announced, filled by the offer's own
/// events before the drag enters.
#[derive(Default)]
struct OfferData(Mutex<Vec<String>>);

/// What the thread knows: the globals, the device, and the offer of the
/// drag in progress.
struct State {
    seat: Option<WlSeat>,
    manager: Option<WlDataDeviceManager>,
    device: Option<WlDataDevice>,
    /// The window's surface, by protocol id; None takes any surface.
    surface: Option<u32>,
    /// The offer the pointer is dragging over the window, once accepted.
    current: Option<WlDataOffer>,
    sink: Box<dyn Fn(Vec<PathBuf>) + Send>,
}

fn listen(
    display: usize,
    surface: Option<u32>,
    sink: Box<dyn Fn(Vec<PathBuf>) + Send>,
    stop: Arc<AtomicBool>,
) {
    // SAFETY: `display` is winit's live wl_display, which `Listener::start`'s
    // caller keeps alive until the listener - and so this thread - is gone.
    let backend =
        unsafe { wayland_client::backend::Backend::from_foreign_display(display as *mut _) };
    let conn = Connection::from_backend(backend);
    let mut queue = conn.new_event_queue::<State>();
    let qh = queue.handle();
    let _registry = conn.display().get_registry(&qh, ());
    let mut state = State {
        seat: None,
        manager: None,
        device: None,
        surface,
        current: None,
        sink,
    };
    if queue.roundtrip(&mut state).is_err() {
        log::warn!("wayland drops: the registry did not answer");
        return;
    }
    let (Some(seat), Some(manager)) = (state.seat.clone(), state.manager.clone()) else {
        log::warn!("wayland drops: no seat or data device manager; file drops are off");
        return;
    };
    state.device = Some(manager.get_data_device(&seat, &qh, ()));
    log::info!("wayland drops: listening on the window's display");

    while !stop.load(Ordering::Relaxed) {
        if queue.dispatch_pending(&mut state).is_err() {
            break;
        }
        // A flush that would block is not a failure: the socket drains.
        let _ = queue.flush();
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fds = [libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: one pollfd, and the fd is the guard's, open for its life.
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 1, TICK_MS) };
        if ready > 0 {
            if guard.read().is_err() {
                break;
            }
        } else {
            // Dropping the guard cancels the read, as the protocol asks.
            drop(guard);
        }
    }
    if let Some(offer) = state.current.take() {
        offer.destroy();
    }
    if let Some(device) = state.device.take()
        && device.version() >= 2
    {
        device.release();
    }
    let _ = queue.flush();
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut State,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<State>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind::<WlSeat, _, _>(name, version.min(5), qh, ()));
                }
                "wl_data_device_manager" if state.manager.is_none() => {
                    state.manager = Some(registry.bind::<WlDataDeviceManager, _, _>(
                        name,
                        version.min(3),
                        qh,
                        (),
                    ));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut State,
        _: &WlSeat,
        _: wayland_client::protocol::wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut State,
        _: &WlDataDeviceManager,
        _: wayland_client::protocol::wl_data_device_manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
    }
}

impl Dispatch<WlDataOffer, OfferData> for State {
    fn event(
        _: &mut State,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        data: &OfferData,
        _: &Connection,
        _: &QueueHandle<State>,
    ) {
        if let wl_data_offer::Event::Offer { mime_type } = event
            && let Ok(mut mimes) = data.0.lock()
        {
            mimes.push(mime_type);
        }
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut State,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        conn: &Connection,
        _: &QueueHandle<State>,
    ) {
        match event {
            wl_data_device::Event::Enter {
                serial,
                surface,
                id: Some(offer),
                ..
            } => {
                let on_window = state
                    .surface
                    .is_none_or(|wanted| surface.id().protocol_id() == wanted);
                let files = offer
                    .data::<OfferData>()
                    .and_then(|data| data.0.lock().ok())
                    .is_some_and(|mimes| mimes.iter().any(|mime| mime == URI_LIST));
                if on_window && files {
                    offer.accept(serial, Some(URI_LIST.to_owned()));
                    if offer.version() >= 3 {
                        offer.set_actions(DndAction::Copy, DndAction::Copy);
                    }
                    state.current = Some(offer);
                } else {
                    offer.accept(serial, None);
                    offer.destroy();
                }
            }
            wl_data_device::Event::Leave => {
                if let Some(offer) = state.current.take() {
                    offer.destroy();
                }
            }
            wl_data_device::Event::Drop => {
                let Some(offer) = state.current.take() else {
                    return;
                };
                let paths = receive(conn, &offer);
                if offer.version() >= 3 {
                    offer.finish();
                }
                offer.destroy();
                if !paths.is_empty() {
                    (state.sink)(paths);
                }
            }
            // The clipboard is not this device's business; the offer is
            // let go so the compositor need not keep it for us.
            wl_data_device::Event::Selection { id: Some(offer) } => offer.destroy(),
            // An offer's mime types arrive on the offer itself before the
            // drag enters, and the pointer's motion changes nothing here.
            _ => {}
        }
    }

    wayland_client::event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferData::default()),
    ]);
}

/// Asks the drag's source for its URI list through a pipe and reads it
/// to the end, or gives up on a source that does not write.
fn receive(conn: &Connection, offer: &WlDataOffer) -> Vec<PathBuf> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: a two-element array, as pipe writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return Vec::new();
    }
    // SAFETY: both ends are fresh descriptors this function owns.
    let (read_end, write_end) =
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    for fd in [&read_end, &write_end] {
        // SAFETY: an open descriptor of ours; the flag only closes it on exec.
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    offer.receive(
        URI_LIST.to_owned(),
        // SAFETY: `write_end` is open until the drop below.
        unsafe { BorrowedFd::borrow_raw(write_end.as_raw_fd()) },
    );
    let _ = conn.flush();
    // The source's write end is the one it was handed; this copy closes so
    // the read below sees its end of file.
    drop(write_end);

    let mut bytes = Vec::new();
    let mut file = std::fs::File::from(read_end);
    let mut chunk = [0u8; 4096];
    loop {
        let mut poll = [libc::pollfd {
            fd: file.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        // SAFETY: one pollfd over the open read end.
        if unsafe { libc::poll(poll.as_mut_ptr(), 1, SOURCE_MS) } <= 0 {
            break;
        }
        match file.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => bytes.extend_from_slice(&chunk[..n]),
        }
    }
    paths_in(&bytes)
}

/// The local files a `text/uri-list` names: one `file:` URI per line,
/// percent-decoded, with comment lines and other schemes left out.
fn paths_in(list: &[u8]) -> Vec<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    list.split(|byte| *byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .filter(|line| !line.is_empty() && line[0] != b'#')
        .filter_map(|line| {
            let rest = line.strip_prefix(b"file://")?;
            // `file:///path`, or `file://host/path` with a host to skip.
            let path = if rest.first() == Some(&b'/') {
                rest
            } else {
                &rest[rest.iter().position(|byte| *byte == b'/')?..]
            };
            Some(PathBuf::from(std::ffi::OsString::from_vec(decoded(path))))
        })
        .collect()
}

/// `%XX` escapes replaced by their bytes; anything malformed is kept as is.
fn decoded(text: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        let hex = |byte: u8| (byte as char).to_digit(16).map(|digit| digit as u8);
        if text[i] == b'%'
            && i + 2 < text.len()
            && let (Some(high), Some(low)) = (hex(text[i + 1]), hex(text[i + 2]))
        {
            out.push(high << 4 | low);
            i += 3;
        } else {
            out.push(text[i]);
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_uri_list_becomes_local_paths() {
        let list = b"# a comment\r\nfile:///home/me/My%20Clip.mp4\r\nfile://localhost/tmp/b.mov\nhttps://example.com/c.mp4\n\n";
        assert_eq!(
            paths_in(list),
            vec![
                PathBuf::from("/home/me/My Clip.mp4"),
                PathBuf::from("/tmp/b.mov")
            ]
        );
    }

    #[test]
    fn malformed_escapes_are_left_alone() {
        assert_eq!(decoded(b"a%2"), b"a%2");
        assert_eq!(decoded(b"%zz"), b"%zz");
        assert_eq!(decoded(b"%41%"), b"A%");
    }
}
