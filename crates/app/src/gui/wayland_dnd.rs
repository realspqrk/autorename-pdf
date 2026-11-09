//! File drops on native Wayland (Linux only).
//!
//! winit 0.30 (eframe 0.37) has no Wayland drag-and-drop: only its X11
//! backend fills `RawInput::{hovered_files, dropped_files}`. Forcing XWayland
//! works but is blurry on scaled outputs, so this module listens for drops
//! itself and injects them into egui's raw input (`App::raw_input_hook`), so
//! every platform keeps one code path.
//!
//! How: wrap winit's `wl_display` in a second, non-owning wayland-rs backend
//! (`Backend::from_foreign_display`). That backend has its own libwayland
//! event queue, so our objects never touch winit's queue. A dedicated thread
//! dispatches it with libwayland's multi-reader protocol (prepare_read → poll →
//! read_events), the same pattern Mesa's EGL/Vulkan WSI and smithay-clipboard
//! use next to a toolkit's queue. We bind `wl_data_device_manager` and every
//! `wl_seat`, get one data device per seat and accept `text/uri-list` offers
//! with the copy action (never move: the source would delete the file).
//!
//! Hover: the URI list is requested right on enter so the overlay can count
//! PDFs (like winit's X11 backend); until it arrives (some sources only answer
//! after the drop) one unknown file is reported. Drop: the list is (re)read,
//! the offer finished, the paths handed to egui.
//!
//! Lifetime: the listener must stop before winit disconnects the display.
//! `App::on_exit` (called by eframe inside the event loop) drops it, which
//! wakes and joins the thread; dropping the backend destroys our proxies and
//! queue. Errors are logged once to stderr and end the thread (no panic).

use std::ffi::OsString;
use std::fs::File;
use std::io::{ErrorKind, Read as _};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use rustix::event::{PollFd, PollFlags};
use rustix::pipe::PipeFlags;
use wayland_client::backend::{Backend, WaylandError};
use wayland_client::protocol::wl_data_device::{self, WlDataDevice};
use wayland_client::protocol::wl_data_device_manager::{DndAction, WlDataDeviceManager};
use wayland_client::protocol::wl_data_offer::{self, WlDataOffer};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::protocol::wl_seat::WlSeat;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, event_created_child,
};

const URI_LIST: &str = "text/uri-list";
/// Upper bound for a URI list (a few hundred thousand paths).
const MAX_LIST: usize = 16 << 20;

/// Drop state shared with the UI thread.
#[derive(Default)]
struct Shared {
    hovered: Vec<egui::HoveredFile>,
    dropped: Vec<PathBuf>,
}

/// Running listener; dropping it stops the thread.
pub struct Listener {
    shared: Arc<Mutex<Shared>>,
    /// Closing the write end (POLLHUP) tells the thread to quit.
    wake: Option<OwnedFd>,
    thread: Option<JoinHandle<()>>,
}

impl Listener {
    /// Start when eframe runs on Wayland; `None` on X11 or on any setup error.
    pub fn start(cc: &eframe::CreationContext<'_>) -> Option<Self> {
        use raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        let RawDisplayHandle::Wayland(h) = cc.display_handle().ok()?.as_raw() else {
            return None;
        };
        // SAFETY: `h.display` is winit's live `wl_display`. It stays valid
        // while this listener exists: the App drops the listener in
        // `on_exit`, which eframe calls inside the event loop, before winit
        // disconnects. The backend does not own (never disconnects) it.
        #[allow(unsafe_code)]
        let backend = unsafe { Backend::from_foreign_display(h.display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);
        let (wake_r, wake_w) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC).ok()?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let mut state = State {
            ctx: cc.egui_ctx.clone(),
            shared: shared.clone(),
            manager: None,
            seats: Vec::new(),
            drag: None,
        };
        let thread = std::thread::Builder::new()
            .name("wayland-dnd".into())
            .spawn(move || {
                if let Err(e) = run(conn, wake_r, &mut state) {
                    eprintln!("warning: Wayland drag and drop disabled: {e}");
                    state.set_hover(Vec::new()); // no stuck drop overlay
                }
            })
            .ok()?;
        Some(Listener {
            shared,
            wake: Some(wake_w),
            thread: Some(thread),
        })
    }

    /// Merge the current hover and any finished drop into this frame's input.
    pub fn inject(&self, raw: &mut egui::RawInput) {
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        raw.hovered_files.extend(s.hovered.iter().cloned());
        raw.dropped_files.extend(
            s.dropped
                .drain(..)
                .map(|path| Arc::new(NativeFile { path }) as egui::DroppedFileHandle),
        );
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.wake = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// egui's `DroppedFile` for a local path (egui-winit's own type is private).
#[derive(Debug)]
struct NativeFile {
    path: PathBuf,
}

impl egui::DroppedFile for NativeFile {
    fn path(&self) -> &Path {
        &self.path
    }

    fn bytes(&self) -> Result<Vec<u8>, String> {
        std::fs::read(&self.path).map_err(|e| e.to_string())
    }
}

/// Dispatch loop: our queue, the wake pipe and a pending URI-list read.
fn run(conn: Connection, wake: OwnedFd, state: &mut State) -> Result<(), String> {
    let mut queue: EventQueue<State> = conn.new_event_queue();
    let qh = queue.handle();
    conn.display().get_registry(&qh, ());
    loop {
        queue.dispatch_pending(state).map_err(|e| e.to_string())?;
        match queue.flush() {
            Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
            r => r.map_err(|e| e.to_string())?,
        }
        // `None`: events were queued meanwhile → dispatch them first.
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let (wl_ready, quit, pipe_ready) = {
            let reading = state.drag.as_ref().and_then(|d| d.read.as_ref());
            let wl_fd = guard.connection_fd();
            let mut fds = vec![
                PollFd::new(&wl_fd, PollFlags::IN),
                PollFd::new(&wake, PollFlags::IN),
            ];
            if let Some(r) = reading {
                fds.push(PollFd::new(&r.file, PollFlags::IN));
            }
            match rustix::event::poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e.to_string()),
            }
            (
                !fds[0].revents().is_empty(),
                !fds[1].revents().is_empty(),
                fds.get(2).is_some_and(|f| !f.revents().is_empty()),
            )
        };
        if quit {
            return Ok(());
        }
        if wl_ready {
            match guard.read() {
                Err(WaylandError::Io(e)) if e.kind() == ErrorKind::WouldBlock => {}
                r => {
                    r.map_err(|e| e.to_string())?;
                }
            }
        } else {
            drop(guard); // cancel our read intent
        }
        if pipe_ready {
            state.pump_read();
        }
    }
}

struct State {
    ctx: egui::Context,
    shared: Arc<Mutex<Shared>>,
    manager: Option<WlDataDeviceManager>,
    /// Seats and their data device (created once the manager is bound).
    seats: Vec<(WlSeat, Option<WlDataDevice>)>,
    drag: Option<Drag>,
}

/// The offer currently over our window (one at a time, newest wins).
struct Drag {
    offer: WlDataOffer,
    uri_list: bool,
    dropped: bool,
    read: Option<Reader>,
}

/// Non-blocking read end of a `wl_data_offer.receive` pipe.
struct Reader {
    file: File,
    buf: Vec<u8>,
}

impl State {
    fn devices(&mut self, qh: &QueueHandle<State>) {
        let Some(m) = &self.manager else { return };
        for (seat, dev) in &mut self.seats {
            if dev.is_none() {
                *dev = Some(m.get_data_device(seat, qh, ()));
            }
        }
    }

    fn set_hover(&self, hovered: Vec<egui::HoveredFile>) {
        if let Ok(mut s) = self.shared.lock() {
            s.hovered = hovered;
        }
        self.ctx.request_repaint();
    }

    /// Forget the current offer (left, replaced or delivered).
    fn end_drag(&mut self) {
        if let Some(d) = self.drag.take() {
            d.offer.destroy();
        }
        self.set_hover(Vec::new());
    }

    /// Ask the source for the URI list; the answer arrives on a pipe.
    fn start_read(&mut self) {
        let Some(d) = &mut self.drag else { return };
        d.read = None;
        let Ok((r, w)) = rustix::pipe::pipe_with(PipeFlags::CLOEXEC) else {
            return;
        };
        // Only our read end is non-blocking (the write end is the source's).
        if rustix::io::ioctl_fionbio(&r, true).is_err() {
            return;
        }
        d.offer.receive(URI_LIST.into(), w.as_fd());
        drop(w); // the source holds its own copy; EOF once it closes it
        d.read = Some(Reader {
            file: File::from(r),
            buf: Vec::new(),
        });
    }

    /// Drain the pipe; on EOF update the hover or deliver the drop.
    fn pump_read(&mut self) {
        let Some(d) = &mut self.drag else { return };
        let Some(r) = &mut d.read else { return };
        let mut chunk = [0u8; 8192];
        let eof = loop {
            match r.file.read(&mut chunk) {
                Ok(0) => break true,
                Ok(n) if r.buf.len() + n <= MAX_LIST => r.buf.extend_from_slice(&chunk[..n]),
                Ok(_) => break true, // absurdly large: parse what we have
                Err(e) if e.kind() == ErrorKind::WouldBlock => break false,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break true,
            }
        };
        if !eof {
            return;
        }
        let paths = parse_uri_list(&r.buf);
        d.read = None;
        if d.dropped {
            let copy = d
                .offer
                .data::<OfferData>()
                .is_some_and(|o| o.copy.load(Ordering::Relaxed));
            if copy && d.offer.version() >= 3 {
                d.offer.finish();
            }
            if let Ok(mut s) = self.shared.lock() {
                s.dropped.extend(paths);
            }
            self.end_drag();
        } else if !paths.is_empty() {
            let hovered = paths
                .into_iter()
                .map(|p| egui::HoveredFile {
                    path: Some(p),
                    mime: String::new(),
                })
                .collect();
            self.set_hover(hovered);
        }
    }
}

/// What an offer announced. Its events can precede `enter` (Hyprland sends
/// `action` right after `data_offer`), so they are stored on the offer.
#[derive(Default)]
struct OfferData {
    uri_list: AtomicBool,
    /// The compositor chose `copy` (required for `finish`).
    copy: AtomicBool,
}

impl Dispatch<WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                // v3: action negotiation (`set_actions`, `finish`).
                "wl_data_device_manager" if state.manager.is_none() => {
                    state.manager = Some(registry.bind(name, version.min(3), qh, ()));
                }
                // v1 suffices: the seat is only an argument.
                "wl_seat" => state.seats.push((registry.bind(name, 1, qh, ()), None)),
                _ => return,
            }
            state.devices(qh);
        }
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDeviceManager, ()> for State {
    fn event(
        _: &mut Self,
        _: &WlDataDeviceManager,
        _: <WlDataDeviceManager as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlDataDevice, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlDataDevice,
        event: wl_data_device::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use wl_data_device::Event as E;
        match event {
            E::Enter { serial, id, .. } => {
                if state.drag.is_some() {
                    state.end_drag();
                }
                let Some(offer) = id else { return };
                let uri_list = offer
                    .data::<OfferData>()
                    .is_some_and(|d| d.uri_list.load(Ordering::Relaxed));
                offer.accept(serial, uri_list.then(|| URI_LIST.into()));
                if uri_list && offer.version() >= 3 {
                    offer.set_actions(DndAction::Copy, DndAction::Copy);
                }
                state.drag = Some(Drag {
                    offer,
                    uri_list,
                    dropped: false,
                    read: None,
                });
                if uri_list {
                    state.set_hover(vec![egui::HoveredFile::default()]);
                    state.start_read();
                }
            }
            E::Leave => {
                // wlroots and Hyprland send leave right after drop: keep a
                // dropped offer until its list is read.
                if state.drag.as_ref().is_some_and(|d| !d.dropped) {
                    state.end_drag();
                }
            }
            E::Drop => {
                let Some(d) = &mut state.drag else { return };
                if !d.uri_list {
                    return state.end_drag();
                }
                d.dropped = true;
                // Re-request: a source may have ignored the pre-drop request.
                state.start_read();
                if state.drag.as_ref().is_some_and(|d| d.read.is_none()) {
                    state.end_drag(); // no pipe: give up instead of hanging
                }
            }
            // Clipboard offers: not ours to keep (arboard has the clipboard).
            E::Selection { id: Some(offer) } => offer.destroy(),
            _ => {}
        }
    }

    event_created_child!(State, WlDataDevice, [
        wl_data_device::EVT_DATA_OFFER_OPCODE => (WlDataOffer, OfferData::default()),
    ]);
}

impl Dispatch<WlDataOffer, OfferData> for State {
    fn event(
        _: &mut Self,
        _: &WlDataOffer,
        event: wl_data_offer::Event,
        data: &OfferData,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_data_offer::Event::Offer { mime_type } if mime_type == URI_LIST => {
                data.uri_list.store(true, Ordering::Relaxed);
            }
            wl_data_offer::Event::Action {
                dnd_action: WEnum::Value(a),
            } => data.copy.store(a == DndAction::Copy, Ordering::Relaxed),
            _ => {}
        }
    }
}

/// Local paths from a `text/uri-list` (RFC 2483): CRLF (or LF) separated,
/// `#` comments, percent-encoded `file://` URIs. Other schemes and remote
/// hosts are skipped; non-UTF-8 names survive (bytes → `OsString`).
pub fn parse_uri_list(data: &[u8]) -> Vec<PathBuf> {
    data.split(|&b| b == b'\n')
        .map(<[u8]>::trim_ascii)
        .filter(|l| !l.is_empty() && !l.starts_with(b"#"))
        .filter_map(file_uri_path)
        .collect()
}

fn file_uri_path(uri: &[u8]) -> Option<PathBuf> {
    let (scheme, rest) = uri.split_at_checked(7)?;
    if !scheme.eq_ignore_ascii_case(b"file://") {
        return None;
    }
    // `file:///p` or `file://localhost/p`; other hosts are not local.
    let path = if rest.starts_with(b"/") {
        rest
    } else {
        rest.strip_prefix(b"localhost")
            .filter(|p| p.starts_with(b"/"))?
    };
    let bytes: Vec<u8> = percent_encoding::percent_decode(path).collect();
    Some(PathBuf::from(OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::parse_uri_list;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::path::PathBuf;

    #[test]
    fn uri_list_decodes_file_uris() {
        let list = b"# comment\r\nfile:///home/u/My%20Invoice.pdf\r\n\
            file://localhost/tmp/a%C3%A4.pdf\r\n\
            https://example.com/x.pdf\r\nfile://otherhost/x.pdf\r\n\
            FILE:///caps.pdf\nfile:///raw space.pdf\r\n\r\n";
        assert_eq!(
            parse_uri_list(list),
            [
                "/home/u/My Invoice.pdf",
                "/tmp/aä.pdf",
                "/caps.pdf",
                "/raw space.pdf"
            ]
            .map(PathBuf::from)
        );
    }

    #[test]
    fn uri_list_keeps_non_utf8_and_ignores_junk() {
        assert_eq!(
            parse_uri_list(b"file:///x%FF.pdf"),
            [PathBuf::from(OsStr::from_bytes(b"/x\xff.pdf"))]
        );
        assert!(parse_uri_list(b"").is_empty());
        assert!(parse_uri_list(b"file:/short\r\nfile://\r\nnot a uri").is_empty());
    }
}
