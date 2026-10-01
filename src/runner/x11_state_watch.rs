//! The X11 minimize/restore signal winit 0.30 receives but drops.
//!
//! Winit selects `PROPERTY_CHANGE` on every window it creates, so the
//! server already delivers `PropertyNotify` for `_NET_WM_STATE` and
//! `WM_STATE` — but `x11/event_processor.rs`'s `property_notify` handles
//! only the `RESOURCE_MANAGER`/`_XSETTINGS_SETTINGS` atoms, and
//! `UnmapNotify` has no match arm at all. An unfocused window minimized
//! from a taskbar, a pager, or `wmctrl` therefore produces no
//! `WindowEvent` of any kind, and the pump's `_NET_WM_STATE_HIDDEN`
//! re-query in `refresh_visibility_signals` never gets a wake to run on.
//!
//! A connection's event queue belongs to its owner, so seeing those
//! notifications means subscribing on a second connection — each client
//! selects its own `event_mask`, and this one's costs winit nothing. The
//! reader blocks in `wait_for_event` — a blocking read, not a poll — and
//! reports the transition over the winit event proxy. The state itself is
//! still read through winit's public `Window::is_minimized` (the
//! `_NET_WM_STATE` query): the events here only wake that query, they
//! never decide it.
//!
//! On an X11 window the app's own connection to the server is already up,
//! so every step here is expected to succeed — a failure is a broken
//! environment, and the runner fails the window's creation rather than
//! silently running without minimize detection.

use std::fmt;
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{Atom, ChangeWindowAttributesAux, ConnectionExt as _, EventMask};
use x11rb::rust_connection::RustConnection;

use super::winit_runner::RunnerEvent;

/// A step in the X11 state watch's setup or event selection failed.
/// On an X11 display the app already holds a working connection, so these
/// are environment failures, not availability reports — the window's
/// creation fails with the failing step named rather than running
/// without minimize detection.
#[derive(Debug)]
pub(super) struct X11WatchError(&'static str);

impl fmt::Display for X11WatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hydrolysis runner: X11 state watch failed: {}", self.0)
    }
}

impl std::error::Error for X11WatchError {}

/// Watches mounted X11 windows for the state-change notifications winit
/// drops, on a second connection to the same display.
pub(super) struct X11StateWatch {
    /// Shared with the reader thread: it blocks in `wait_for_event` while
    /// `select` issues requests from the runner thread — `RustConnection`
    /// synchronizes the write side itself.
    connection: Arc<RustConnection>,
}

impl X11StateWatch {
    /// Connects to the X server and starts the reader thread.
    pub(super) fn connect(proxy: EventLoopProxy<RunnerEvent>) -> Result<Self, X11WatchError> {
        let (connection, _screen) =
            x11rb::connect(None).map_err(|_| X11WatchError("x11rb::connect"))?;
        let intern = |name: &'static [u8]| -> Result<Atom, X11WatchError> {
            connection
                .intern_atom(false, name)
                .map_err(|_| X11WatchError("intern_atom request"))?
                .reply()
                .map(|reply| reply.atom)
                .map_err(|_| X11WatchError("intern_atom reply"))
        };
        let net_wm_state = intern(b"_NET_WM_STATE")?;
        let wm_state = intern(b"WM_STATE")?;
        let connection = Arc::new(connection);
        let watching = Arc::clone(&connection);
        std::thread::Builder::new()
            .name("hydrolysis-x11-state-watch".to_owned())
            .spawn(move || watch_events(watching, net_wm_state, wm_state, proxy))
            .map_err(|_| X11WatchError("reader thread spawn"))?;
        Ok(Self { connection })
    }

    /// Adds a window's XID to the set whose state notifications wake the
    /// loop — `PROPERTY_CHANGE` for `_NET_WM_STATE`/`WM_STATE`,
    /// `STRUCTURE_NOTIFY` for the unmap/map edge itself. The mask lives on
    /// this connection only; winit's own selection is untouched.
    ///
    /// Once selected, nothing unsubscribes: a destroyed XID generates no
    /// further events, and the request against it errors asynchronously.
    pub(super) fn select(&self, window: u32) -> Result<(), X11WatchError> {
        let aux = ChangeWindowAttributesAux::new()
            .event_mask(EventMask::PROPERTY_CHANGE | EventMask::STRUCTURE_NOTIFY);
        self.connection
            .change_window_attributes(window, &aux)
            .map_err(|_| X11WatchError("change_window_attributes request"))?
            .check()
            .map_err(|_| X11WatchError("change_window_attributes reply"))?;
        self.connection
            .flush()
            .map_err(|_| X11WatchError("flush"))?;
        Ok(())
    }
}

/// The winit window's XID, for [`X11StateWatch::select`]. `None` on a
/// non-X11 window handle (a Wayland surface has no XID to watch).
pub(super) fn x11_window_id(native_window: &winit::window::Window) -> Option<u32> {
    match native_window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xcb(xcb) => Some(xcb.window.get()),
        RawWindowHandle::Xlib(xlib) => Some(xlib.window as u32),
        _ => None,
    }
}

fn watch_events(
    connection: Arc<RustConnection>,
    net_wm_state: Atom,
    wm_state: Atom,
    proxy: EventLoopProxy<RunnerEvent>,
) {
    loop {
        let visibility_moved = match connection.wait_for_event() {
            Ok(Event::PropertyNotify(event)) => {
                event.atom == net_wm_state || event.atom == wm_state
            }
            // Minimize unmaps the window and restore maps it back; either
            // edge is a cue to re-read `is_minimized`. The query decides —
            // the event only wakes it.
            Ok(Event::UnmapNotify(_) | Event::MapNotify(_)) => true,
            Ok(_) => false,
            Err(error) => {
                // The connection died while the loop may still be alive —
                // an environment failure, not a state the pump should
                // tolerate: without the watch, an unfocused minimize goes
                // unnoticed and the pump renders a hidden window. Name it
                // and end the watch explicitly.
                tracing::error!(
                    %error,
                    "hydrolysis runner: X11 state watch lost its connection; \
                     minimize detection ends with it"
                );
                break;
            }
        };
        if visibility_moved {
            // send_event fails only once the loop is exiting — nothing is
            // left to wake then.
            let _ = proxy.send_event(RunnerEvent::X11VisibilitySignal);
        }
    }
}
