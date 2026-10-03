//! `Window::placement` / `Window::activation` support (water-rs/waterui#1302).
//!
//! The runner resolves a window's [`MonitorSelector`] each time the window is
//! shown. A winit window is only ever "shown" at mount — a `Closed` window is
//! destroyed by `remove_closed_windows` and never re-shown, and a `Hidden` ->
//! `WindowState::Normal` write is a `set_visible` on the already-mapped
//! window, whose monitor stays where the desktop put it — so mount-time
//! resolution covers every show the runner performs. The resolved [`Monitor`]
//! feeds the placement's `place` callback, whose rect is written into
//! `Window::frame` before the window's attributes are built, so the frame
//! travels with the map request.
//!
//! Resolution reads the event loop's own monitor enumeration and — on X11 —
//! the display connection winit already holds (through the raw display
//! handle). No second connection, no global cache, no polling: state the
//! selector needs that winit does not answer (the last pointer-focused
//! window, Wayland's only pointer home) lives in `WinitRunner` fields.

use waterui::window::{Activation, Monitor, MonitorSelector};
use waterui_core::Str;
use waterui_core::layout::{Point, Rect, Size};
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::window::Window as NativeWindow;

/// One monitor's bounds — `(x, y, width, height)`, edges exclusive on the
/// far side.
type FrameSpec = (f64, f64, f64, f64);

/// The monitor a [`MonitorSelector`] resolves to.
///
/// `frames`, `pointer` and `focused` share one coordinate space (the caller
/// picks physical pixels or logical points; containment is unaffected by
/// which). `primary` is the platform's primary-monitor index, `focused` the
/// index of the monitor holding the application's focused window, `pointer`
/// the global pointer position, and `wayland` marks a Wayland display — the
/// one platform with neither a global pointer query nor a primary monitor.
///
/// Deliberate rules:
/// - `Focused` with no focused window (or a focused monitor that vanished
///   mid-resolution) resolves as `Primary` — the API's documented rule.
/// - `Pointer` with the pointer outside every frame — a gap between
///   monitors — resolves to the nearest frame by distance.
/// - A missing `primary` is legitimate only on Wayland, where index 0
///   stands in. On every other platform a missing primary with monitors
///   present is a platform bug: panic.
/// - A missing `pointer` is legitimate only on Wayland: it warns once and
///   resolves to the first monitor. Everywhere else the position was
///   queryable and its absence is a failure: panic.
///
/// `None` only when `frames` is empty — the caller panics on that, as zero
/// monitors while a window is being shown is a platform bug, never a
/// skipped placement.
fn resolve_selector(
    selector: MonitorSelector,
    frames: &[FrameSpec],
    primary: Option<usize>,
    pointer: Option<(f64, f64)>,
    focused: Option<usize>,
    wayland: bool,
) -> Option<usize> {
    if frames.is_empty() {
        return None;
    }
    let primary = match primary.filter(|index| *index < frames.len()) {
        Some(index) => index,
        // Wayland reports no primary monitor at all; index 0 stands in.
        None if wayland => 0,
        None => panic!(
            "hydrolysis runner: {selector:?} cannot resolve — primary_monitor() \
             reported none while {} monitors exist",
            frames.len()
        ),
    };
    Some(match selector {
        MonitorSelector::Primary => primary,
        MonitorSelector::Focused => focused
            .filter(|index| *index < frames.len())
            .unwrap_or(primary),
        MonitorSelector::Pointer => match pointer {
            Some(point) => frames
                .iter()
                .position(|frame| frame_contains(*frame, point))
                .unwrap_or_else(|| nearest_frame(frames, point)),
            None if wayland => {
                tracing::warn!(
                    "window placement: MonitorSelector::Pointer cannot see the global pointer \
                     on Wayland; resolving to the first monitor"
                );
                0
            }
            None => panic!(
                "hydrolysis runner: {selector:?} got no pointer position from a platform \
                 that answers the query"
            ),
        },
    })
}

/// Whether `frame` contains `point` — the far edges are exclusive.
fn frame_contains((x, y, width, height): FrameSpec, (px, py): (f64, f64)) -> bool {
    px >= x && px < x + width && py >= y && py < y + height
}

/// Squared distance from `point` to `frame` — zero inside, the gap distance
/// outside. Squared ordering suffices; no square root is needed.
fn frame_distance_sq((x, y, width, height): FrameSpec, (px, py): (f64, f64)) -> f64 {
    let dx = if px < x {
        x - px
    } else if px > x + width {
        px - (x + width)
    } else {
        0.0
    };
    let dy = if py < y {
        y - py
    } else if py > y + height {
        py - (y + height)
    } else {
        0.0
    };
    dx.mul_add(dx, dy * dy)
}

/// The frame nearest `point` — where a pointer resting in a gap between
/// monitors is said to be.
fn nearest_frame(frames: &[FrameSpec], point: (f64, f64)) -> usize {
    frames
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            frame_distance_sq(**a, point).total_cmp(&frame_distance_sq(**b, point))
        })
        .map(|(index, _)| index)
        .expect("nearest_frame: frames is non-empty (the caller checked)")
}

/// A monitor as winit reports it, plus whether the platform calls it the
/// primary one.
struct MonitorSpec {
    handle: MonitorHandle,
    /// `handle.position()`/`handle.size()` — physical pixels, the space X11
    /// `_NET_WORKAREA`, `QueryPointer` and `GetCursorPos` all answer in.
    physical: FrameSpec,
    is_primary: bool,
}

impl MonitorSpec {
    /// The monitor's logical-point frame: physical bounds divided by its
    /// scale factor, the space `Window::frame` is expressed in.
    fn logical_frame(&self) -> FrameSpec {
        let (x, y, width, height) = self.physical;
        let scale = self.handle.scale_factor();
        (x / scale, y / scale, width / scale, height / scale)
    }
}

/// Every monitor the event loop reports, with the primary flagged the same
/// way `monitor_layout` identifies a monitor: position+size match.
fn enumerate_monitors(event_loop: &ActiveEventLoop) -> Vec<MonitorSpec> {
    let primary = event_loop.primary_monitor();
    event_loop
        .available_monitors()
        .map(|handle| {
            let is_primary = primary.as_ref().is_some_and(|primary| {
                primary.position() == handle.position() && primary.size() == handle.size()
            });
            let position = handle.position();
            let size = handle.size();
            MonitorSpec {
                physical: (
                    f64::from(position.x),
                    f64::from(position.y),
                    f64::from(size.width),
                    f64::from(size.height),
                ),
                is_primary,
                handle,
            }
        })
        .collect()
}

/// The index of `handle` in `specs`, matched by physical position and size —
/// the same identity the primary flag and `monitor_layout` use.
fn monitor_index(specs: &[MonitorSpec], handle: &MonitorHandle) -> Option<usize> {
    specs.iter().position(|spec| {
        spec.handle.position() == handle.position() && spec.handle.size() == handle.size()
    })
}

/// Everything pointer resolution needs beyond the event loop: the runner's
/// focused window (for `Focused`) and its last pointer-focused window (the
/// only pointer home Wayland reports).
pub(crate) struct PlacementContext<'a> {
    pub event_loop: &'a ActiveEventLoop,
    pub focused_window: Option<&'a NativeWindow>,
    /// Wayland-only state: the other platforms answer a global pointer
    /// query, so nothing else reads it.
    #[allow(dead_code)]
    pub pointer_window: Option<&'a NativeWindow>,
}

/// Resolves `selector` against the monitors the platform reports right now,
/// with `ActiveEventLoop` in hand, and assembles the public [`Monitor`].
///
/// Panics when the platform reports zero monitors: a window being shown has
/// nowhere to go, and skipping placement silently would map it wherever the
/// compositor feels like — that is a bug, so it fails loudly.
pub(crate) fn resolve_placement_monitor(
    context: &PlacementContext<'_>,
    selector: MonitorSelector,
) -> Monitor {
    let specs = enumerate_monitors(context.event_loop);
    let primary = specs.iter().position(|spec| spec.is_primary);
    let focused = context
        .focused_window
        .and_then(|window| window.current_monitor())
        .and_then(|current| monitor_index(&specs, &current));
    let pointer = pointer_position(context, &specs);
    // The pointer answer's coordinate space differs per platform: macOS
    // `NSEvent.mouseLocation` is in Cocoa logical points, while X11
    // `QueryPointer` and Win32 `GetCursorPos` answer physical pixels.
    let frames: Vec<FrameSpec> = specs
        .iter()
        .map(|spec| {
            if cfg!(target_os = "macos") {
                spec.logical_frame()
            } else {
                spec.physical
            }
        })
        .collect();
    let wayland = event_loop_is_wayland(context.event_loop);
    let index = resolve_selector(selector, &frames, primary, pointer, focused, wayland)
        .unwrap_or_else(|| {
            panic!(
                "hydrolysis runner: Window::placement asked for {selector:?} but the platform reports zero monitors"
            )
        });
    assemble_monitor(context.event_loop, &specs[index])
}

/// Whether the event loop's display connection is Wayland — the only
/// platform family here with neither a global pointer query nor a primary
/// monitor concept.
fn event_loop_is_wayland(event_loop: &ActiveEventLoop) -> bool {
    #[cfg(hydrolysis_wayland_platform)]
    {
        use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
        return matches!(
            event_loop.display_handle().map(|handle| handle.as_raw()),
            Ok(RawDisplayHandle::Wayland(_))
        );
    }
    #[allow(unreachable_code)]
    {
        let _ = event_loop;
        false
    }
}

/// The pointer position in the same coordinate space `resolve_selector` is
/// driven in (physical everywhere but macOS, where it is logical).
///
/// Wayland has no global pointer query: the "pointer monitor" is the monitor
/// of the runner's window that last had pointer focus — its frame centre is
/// returned so containment lands on it.
fn pointer_position(context: &PlacementContext<'_>, specs: &[MonitorSpec]) -> Option<(f64, f64)> {
    #[cfg(target_os = "macos")]
    {
        let _ = (context, specs);
        macos_pointer_position()
    }
    #[cfg(target_os = "windows")]
    {
        let _ = (context, specs);
        windows_pointer_position()
    }
    #[cfg(hydrolysis_wayland_platform)]
    {
        // X11 reports a global pointer; Wayland does not, so the last
        // pointer-focused window's monitor centre stands in for it.
        x11_pointer_position(context.event_loop).or_else(|| {
            context
                .pointer_window
                .and_then(|window| window.current_monitor())
                .and_then(|current| monitor_index(specs, &current))
                .map(|index| {
                    let (x, y, width, height) = specs[index].physical;
                    (x + width / 2.0, y + height / 2.0)
                })
        })
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        hydrolysis_wayland_platform
    )))]
    {
        let _ = (context, specs);
        None
    }
}

/// Assembles the public [`Monitor`] for a resolved spec: the logical frame,
/// the platform's work area as `visible_frame`, the scale factor, and the
/// platform's display name.
fn assemble_monitor(event_loop: &ActiveEventLoop, spec: &MonitorSpec) -> Monitor {
    let (x, y, width, height) = spec.logical_frame();
    let frame = Rect::new(
        Point::new(x as f32, y as f32),
        Size::new(width as f32, height as f32),
    );
    Monitor {
        frame,
        visible_frame: platform_visible_frame(event_loop, spec)
            .map(|(vx, vy, vw, vh)| {
                Rect::new(
                    Point::new(vx as f32, vy as f32),
                    Size::new(vw as f32, vh as f32),
                )
            })
            .unwrap_or(frame),
        scale_factor: spec.handle.scale_factor(),
        name: spec.handle.name().map(Str::from),
    }
}

/// The monitor's work area in logical points, or `None` where the platform
/// reports none (then `visible_frame` equals `frame`).
fn platform_visible_frame(event_loop: &ActiveEventLoop, spec: &MonitorSpec) -> Option<FrameSpec> {
    #[cfg(target_os = "macos")]
    {
        let _ = event_loop;
        macos_visible_frame(spec)
    }
    #[cfg(target_os = "windows")]
    {
        let _ = event_loop;
        windows_visible_frame(spec)
    }
    #[cfg(hydrolysis_wayland_platform)]
    {
        // `_NET_WORKAREA` is X11-only; Wayland has no work-area protocol.
        x11_visible_frame(event_loop, spec)
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        hydrolysis_wayland_platform
    )))]
    {
        let _ = (event_loop, spec);
        None
    }
}

/// Free-Unix display plumbing: the X11 connection and root window of the
/// display winit already owns, or `None` when the runtime backend is Wayland.
///
/// The connection is borrowed, never created: the winit `Xcb` backend hands
/// over its `xcb_connection_t` directly, and the `Xlib` backend's `Display*`
/// yields its XCB connection through `XGetXCBConnection` — the two share one
/// socket, so this is a view on winit's connection, not a second one.
#[cfg(hydrolysis_wayland_platform)]
struct X11Display {
    /// Keeps `libX11-xcb` loaded for the pointer the wrapper hands out.
    /// `XGetXCBConnection`'s answer stays valid for the `Display`'s life.
    _xlib_xcb: Option<x11_dl::xlib_xcb::Xlib_xcb>,
    connection: x11rb::xcb_ffi::XCBConnection,
    root: x11rb::protocol::xproto::Window,
}

/// Winit's own X11 connection and the root window of the display's default
/// screen. `None` only on a non-X11 connection (Wayland): a display handle
/// error or any null/failed piece of the X11 plumbing panics, since placing
/// a window on a broken connection is a bug, not a skipped placement.
#[cfg(hydrolysis_wayland_platform)]
fn x11_display(event_loop: &ActiveEventLoop) -> Option<X11Display> {
    use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
    use x11rb::connection::Connection;
    use x11rb::xcb_ffi::XCBConnection;
    let raw = event_loop
        .display_handle()
        .expect("hydrolysis runner: winit display_handle() failed")
        .as_raw();
    match raw {
        RawDisplayHandle::Xcb(handle) => {
            let ptr = handle
                .connection
                .expect(
                    "hydrolysis runner: winit's XcbDisplayHandle carries a null xcb_connection_t",
                )
                .as_ptr();
            // `should_drop = false`: the connection is winit's; wrapping is a
            // borrow, dropping the wrapper must not disconnect it.
            let connection = unsafe { XCBConnection::from_raw_xcb_connection(ptr, false) }
                .expect("hydrolysis runner: xcb_ffi failed to wrap winit's X11 connection");
            let root = connection
                .setup()
                .roots
                .get(handle.screen as usize)
                .map(|screen| screen.root)
                .expect("hydrolysis runner: X11 connection has no root for the screen winit named");
            Some(X11Display {
                _xlib_xcb: None,
                connection,
                root,
            })
        }
        RawDisplayHandle::Xlib(handle) => {
            let display = handle
                .display
                .expect("hydrolysis runner: winit's XlibDisplayHandle carries a null Display*")
                .as_ptr()
                .cast::<x11_dl::xlib::Display>();
            let xlib_xcb = x11_dl::xlib_xcb::Xlib_xcb::open()
                .expect("hydrolysis runner: Xlib_xcb::open failed — libX11-xcb is missing");
            let ptr = unsafe { (xlib_xcb.XGetXCBConnection)(display) }.cast::<std::ffi::c_void>();
            assert!(
                !ptr.is_null(),
                "hydrolysis runner: XGetXCBConnection returned null for winit's Display"
            );
            let connection = unsafe { XCBConnection::from_raw_xcb_connection(ptr.cast(), false) }
                .expect("hydrolysis runner: xcb_ffi failed to wrap the Xlib XCB connection");
            let root = connection
                .setup()
                .roots
                .get(handle.screen as usize)
                .map(|screen| screen.root)
                .expect("hydrolysis runner: X11 connection has no root for the screen winit named");
            Some(X11Display {
                _xlib_xcb: Some(xlib_xcb),
                connection,
                root,
            })
        }
        _ => None,
    }
}

/// The X11 pointer position in physical pixels: `XQueryPointer`'s
/// root-window coordinates are physical by definition. `None` only on a
/// non-X11 display; a failed request or reply on a live X11 connection
/// panics — a window being placed cannot lose its pointer silently.
#[cfg(hydrolysis_wayland_platform)]
fn x11_pointer_position(event_loop: &ActiveEventLoop) -> Option<(f64, f64)> {
    use x11rb::protocol::xproto::ConnectionExt;
    let display = x11_display(event_loop)?;
    let reply = display
        .connection
        .query_pointer(display.root)
        .expect("hydrolysis runner: X11 QueryPointer request failed")
        .reply()
        .expect("hydrolysis runner: X11 QueryPointer reply failed");
    Some((f64::from(reply.root_x), f64::from(reply.root_y)))
}

/// The X11 work area: `_NET_WORKAREA` of the current desktop (EWMH) in
/// physical pixels, intersected with the monitor's physical frame, converted
/// to logical points. `None` only for legitimate absence — a non-X11
/// connection, or a WM that publishes no `_NET_CURRENT_DESKTOP`/
/// `_NET_WORKAREA` (empty property value). Every request/reply failure on
/// the X11 connection panics.
#[cfg(hydrolysis_wayland_platform)]
fn x11_visible_frame(event_loop: &ActiveEventLoop, spec: &MonitorSpec) -> Option<FrameSpec> {
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};
    let display = x11_display(event_loop)?;
    let conn = &display.connection;
    let workarea = conn
        .intern_atom(false, b"_NET_WORKAREA")
        .expect("hydrolysis runner: X11 InternAtom(_NET_WORKAREA) request failed")
        .reply()
        .expect("hydrolysis runner: X11 InternAtom(_NET_WORKAREA) reply failed")
        .atom;
    let current_desktop = conn
        .intern_atom(false, b"_NET_CURRENT_DESKTOP")
        .expect("hydrolysis runner: X11 InternAtom(_NET_CURRENT_DESKTOP) request failed")
        .reply()
        .expect("hydrolysis runner: X11 InternAtom(_NET_CURRENT_DESKTOP) reply failed")
        .atom;
    let desktop = conn
        .get_property(
            false,
            display.root,
            current_desktop,
            AtomEnum::CARDINAL,
            0,
            1,
        )
        .expect("hydrolysis runner: X11 GetProperty(_NET_CURRENT_DESKTOP) request failed")
        .reply()
        .expect("hydrolysis runner: X11 GetProperty(_NET_CURRENT_DESKTOP) reply failed");
    // A WM that publishes no `_NET_CURRENT_DESKTOP` has no work areas.
    let index = u32::from_le_bytes(*desktop.value.first_chunk::<4>()?) as usize;
    let reply = conn
        .get_property(false, display.root, workarea, AtomEnum::CARDINAL, 0, 1024)
        .expect("hydrolysis runner: X11 GetProperty(_NET_WORKAREA) request failed")
        .reply()
        .expect("hydrolysis runner: X11 GetProperty(_NET_WORKAREA) reply failed");
    // `_NET_WORKAREA` is a flat CARD32 list, four entries per desktop.
    let base = index * 4;
    let (cards, _) = reply.value.as_chunks::<4>();
    let area: Vec<u32> = cards
        .iter()
        .skip(base)
        .take(4)
        .map(|card| u32::from_le_bytes(*card))
        .collect();
    let &[wx, wy, ww, wh] = area.as_slice() else {
        return None;
    };
    let (wx, wy, ww, wh) = (f64::from(wx), f64::from(wy), f64::from(ww), f64::from(wh));
    let (mx, my, mw, mh) = spec.physical;
    let x = wx.max(mx);
    let y = wy.max(my);
    let right = (wx + ww).min(mx + mw);
    let bottom = (wy + wh).min(my + mh);
    if right <= x || bottom <= y {
        // The work area does not intersect this monitor at all: the desktop
        // reserves nothing on it.
        return Some(spec.physical);
    }
    let scale = spec.handle.scale_factor();
    Some((
        x / scale,
        y / scale,
        (right - x) / scale,
        (bottom - y) / scale,
    ))
}

/// The pointer position in Cocoa's logical-point space with y flipped into
/// winit's top-left-origin logical space: `NSEvent.mouseLocation` is
/// measured up from the primary screen's bottom edge.
#[cfg(target_os = "macos")]
fn macos_pointer_position() -> Option<(f64, f64)> {
    use objc2_app_kit::NSEvent;
    let point = NSEvent::mouseLocation();
    let primary_height = primary_screen_height()?;
    Some((point.x, primary_height - point.y))
}

/// The primary `NSScreen`'s frame height in points — the y-flip pivot Cocoa
/// measures `mouseLocation` and `visibleFrame` against.
#[cfg(target_os = "macos")]
fn primary_screen_height() -> Option<f64> {
    use objc2::MainThreadMarker;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSScreen;
    let mtm = MainThreadMarker::new().expect(
        "hydrolysis runner: placement resolved off the main thread (MainThreadMarker unavailable)",
    );
    autoreleasepool(|_| {
        let screens = NSScreen::screens(mtm);
        screens
            .firstObject()
            .map(|screen| screen.frame().size.height)
    })
}

/// `NSScreen.visibleFrame`, matched to `spec` by `CGDirectDisplayID` and
/// flipped into winit's logical space. `None` leaves `visible_frame` equal
/// to `frame`.
#[cfg(target_os = "macos")]
fn macos_visible_frame(spec: &MonitorSpec) -> Option<FrameSpec> {
    use objc2::MainThreadMarker;
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSScreen;
    use winit::platform::macos::MonitorHandleExtMacOS;
    let mtm = MainThreadMarker::new().expect(
        "hydrolysis runner: placement resolved off the main thread (MainThreadMarker unavailable)",
    );
    autoreleasepool(|_| {
        let id = spec.handle.native_id();
        let screens = NSScreen::screens(mtm);
        let screen = screens
            .iter()
            .find(|screen| screen_number_id(screen) == Some(id))?;
        let visible = screen.visibleFrame();
        // `visibleFrame`'s y is measured up from the primary screen's bottom
        // edge; winit logical space measures down from its top, so the rect
        // flips against the primary screen height.
        let primary_height = screens
            .firstObject()
            .map(|primary| primary.frame().size.height)?;
        let (x, y, w, h) = (
            visible.origin.x,
            visible.origin.y,
            visible.size.width,
            visible.size.height,
        );
        Some((x, primary_height - (y + h), w, h))
    })
}

/// An `NSScreen`'s `CGDirectDisplayID`: the `NSScreenNumber` entry in its
/// `deviceDescription` dictionary.
#[cfg(target_os = "macos")]
fn screen_number_id(screen: &objc2_app_kit::NSScreen) -> Option<u32> {
    use objc2_foundation::{NSNumber, ns_string};
    screen
        .deviceDescription()
        .objectForKey(ns_string!("NSScreenNumber"))
        .and_then(|object| object.downcast::<NSNumber>().ok())
        .map(|number| number.unsignedIntValue())
}

/// The Win32 pointer position in physical pixels (`GetCursorPos` answers in
/// the virtual-screen's physical space once winit sets per-monitor-v2 DPI
/// awareness).
#[cfg(target_os = "windows")]
fn windows_pointer_position() -> Option<(f64, f64)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then(|| (f64::from(point.x), f64::from(point.y)))
}

/// The monitor's work area from `GetMonitorInfoW.rcWork` (physical pixels,
/// converted to logical points): `rcWork` is `frame` minus the taskbar and
/// docked app bars.
#[cfg(target_os = "windows")]
fn windows_visible_frame(spec: &MonitorSpec) -> Option<FrameSpec> {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
    };

    /// `EnumDisplayMonitors` callback payload: the `HMONITOR` winit's
    /// `MonitorHandle` wraps, filled with the matching monitor's `rcWork`.
    struct Lookup {
        hmonitor: HMONITOR,
        work_area: Option<RECT>,
    }
    unsafe extern "system" fn find_monitor(
        monitor: HMONITOR,
        _dc: HDC,
        _rect: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let lookup = unsafe { &mut *(data as *mut Lookup) };
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            dwFlags: 0,
        };
        if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
            return TRUE;
        }
        if monitor == lookup.hmonitor {
            lookup.work_area = Some(info.rcWork);
            return 0; // found: stop enumerating
        }
        TRUE
    }

    use winit::platform::windows::MonitorHandleExtWindows;
    let mut lookup = Lookup {
        hmonitor: spec.handle.hmonitor(),
        work_area: None,
    };
    unsafe {
        EnumDisplayMonitors(
            0,
            std::ptr::null(),
            Some(find_monitor),
            (&mut lookup) as *mut _ as LPARAM,
        );
    }
    let work = lookup.work_area?;
    let scale = spec.handle.scale_factor();
    Some((
        f64::from(work.left) / scale,
        f64::from(work.top) / scale,
        f64::from(work.right - work.left) / scale,
        f64::from(work.bottom - work.top) / scale,
    ))
}

/// Shows a freshly mounted window the way its [`Activation`] policy asks.
///
/// Every platform but macOS goes through winit's `set_visible`. macOS
/// distinguishes: `OnShow` uses it too — AppKit's `makeKeyAndOrderFront`
/// makes the window key and activates the app, which is what showing an
/// `OnShow` window means — while `OnClick`/`Never` order the window front
/// with `orderFront:`, which puts it on screen without key status or app
/// activation. winit cannot express that split: `set_visible` is always
/// `makeKeyAndOrderFront`, which activates the app — fatally at process
/// launch, where a resident drop-down terminal must not pull focus from
/// the user's app.
pub(crate) fn show_at_mount(native_window: &NativeWindow, activation: Activation) {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = activation;
        native_window.set_visible(true);
    }
    #[cfg(target_os = "macos")]
    match activation {
        Activation::OnShow => native_window.set_visible(true),
        Activation::OnClick | Activation::Never => macos_order_front(native_window),
    }
}

/// `orderFront:` on the winit `NSWindow`: on-screen, neither key nor
/// activating — the non-activating show `OnClick`/`Never` ask for.
#[cfg(target_os = "macos")]
fn macos_order_front(native_window: &NativeWindow) {
    use objc2_app_kit::NSView;
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = native_window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit guarantees `ns_view` is a valid `NSView` for the window's
    // life; borrowing it for this call does not retain or release.
    let view = unsafe { &*appkit.ns_view.cast::<NSView>().as_ptr() };
    let Some(ns_window) = view.window() else {
        return;
    };
    // `orderFront:` nominally takes the sending object; `None` is the
    // no-sender form AppKit documents.
    ns_window.orderFront(None);
}

/// The activation parts `WindowAttributes::with_active` cannot express,
/// applied to the freshly created window before it maps.
///
/// * X11 `Never`: `WM_HINTS`'s input flag false, so the window manager does
///   not route keyboard focus to the window at all.
/// * macOS `OnClick`/`Never`: the `NonactivatingPanel` bit on the window's
///   style mask. Hydrolysis's NSWindow is the winit-created `NSWindow`
///   subclass (`WinitWindow`), which cannot be re-classed into `NSPanel`
///   after creation, so the panel behaviour is set as the style bit winit
///   leaves unset — the same bit `NSPanel` uses. What AppKit honours on a
///   plain `NSWindow` is noted in the delivery.
/// * Windows `Never`: `WS_EX_NOACTIVATE` on the HWND, so clicks do not
///   activate the window.
pub(crate) fn apply_activation(
    event_loop: &ActiveEventLoop,
    native_window: &NativeWindow,
    activation: Activation,
) {
    match activation {
        Activation::OnShow => {}
        Activation::OnClick | Activation::Never => {
            #[cfg(any(target_os = "macos", target_os = "windows"))]
            let _ = event_loop;
            #[cfg(target_os = "macos")]
            apply_macos_nonactivating(native_window);
            #[cfg(target_os = "windows")]
            apply_windows_noactivate(native_window, activation == Activation::Never);
            #[cfg(hydrolysis_wayland_platform)]
            if activation == Activation::Never {
                apply_x11_input_hint(event_loop, native_window);
            }
            #[cfg(not(any(
                target_os = "macos",
                target_os = "windows",
                hydrolysis_wayland_platform
            )))]
            let _ = (event_loop, native_window, activation);
        }
    }
}

/// X11 `Never`: `WM_HINTS` with `input = false`, merged into any hints the
/// window already carries. Called between `create_window` and
/// `set_visible(true)`, so the property is in place before the map request.
///
/// Returns only on a Wayland connection, where there is no `WM_HINTS` to
/// write (the documented Wayland limitation). On an X11 window every
/// failure — the handle lookup, the hints read, the hints write — panics:
/// `Activation::Never` that silently doesn't happen is worse than a crash,
/// and `unwrap_or_default` on a failed read would clobber the window's
/// other hints.
#[cfg(hydrolysis_wayland_platform)]
fn apply_x11_input_hint(event_loop: &ActiveEventLoop, native_window: &NativeWindow) {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::properties::WmHints;
    let Some(display) = x11_display(event_loop) else {
        return;
    };
    let handle = native_window
        .window_handle()
        .expect("hydrolysis runner: winit window_handle() failed while writing WM_HINTS");
    let window = match handle.as_raw() {
        RawWindowHandle::Xcb(xcb) => xcb.window.get(),
        RawWindowHandle::Xlib(xlib) => xlib.window as u32,
        other => panic!(
            "hydrolysis runner: Activation::Never on X11 got a non-X11 window handle {other:?}"
        ),
    };
    // `Ok(None)` means the window carries no WM_HINTS yet — the only case
    // where defaults are seeded.
    let mut hints = WmHints::get(&display.connection, window)
        .expect("hydrolysis runner: X11 GetProperty(WM_HINTS) request failed")
        .reply()
        .expect("hydrolysis runner: X11 GetProperty(WM_HINTS) reply failed")
        .unwrap_or_default();
    hints.input = Some(false);
    hints
        .set(&display.connection, window)
        .expect("hydrolysis runner: failed to write WM_HINTS input=false");
}

/// macOS `OnClick`/`Never`: `NSWindowStyleMaskNonactivatingPanel` on the
/// existing `NSWindow` — the panel behaviour expressed as the style bit,
/// since a winit `NSWindow` subclass cannot be re-classed to `NSPanel`.
#[cfg(target_os = "macos")]
fn apply_macos_nonactivating(native_window: &NativeWindow) {
    use objc2_app_kit::{NSView, NSWindowStyleMask};
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = native_window.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: winit guarantees `ns_view` is a valid `NSView` for the window's
    // life; borrowing it for this call does not retain or release.
    let view = unsafe { &*appkit.ns_view.cast::<NSView>().as_ptr() };
    let Some(ns_window) = view.window() else {
        return;
    };
    ns_window.setStyleMask(ns_window.styleMask() | NSWindowStyleMask::NonactivatingPanel);
}

/// Windows `Never`: `WS_EX_NOACTIVATE` OR-ed into the extended style, so a
/// click on the window does not activate it.
#[cfg(target_os = "windows")]
fn apply_windows_noactivate(native_window: &NativeWindow, never: bool) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE,
    };
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let Ok(handle) = native_window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(win32) = handle.as_raw() else {
        return;
    };
    let hwnd = win32.hwnd.get() as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let mut extended = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        if never {
            extended |= WS_EX_NOACTIVATE as isize;
        } else {
            extended &= !(WS_EX_NOACTIVATE as isize);
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: FrameSpec = (0.0, 0.0, 1600.0, 1200.0);
    const RIGHT: FrameSpec = (1600.0, 0.0, 1920.0, 1080.0);
    const TALL: FrameSpec = (3520.0, -200.0, 1200.0, 1900.0);

    #[test]
    fn primary_resolves_the_flagged_monitor() {
        let frames = [LEFT, RIGHT, TALL];
        assert_eq!(
            resolve_selector(
                MonitorSelector::Primary,
                &frames,
                Some(1),
                None,
                None,
                false
            ),
            Some(1)
        );
        // No platform-reported primary is legitimate only on Wayland, where
        // index 0 stands in.
        assert_eq!(
            resolve_selector(MonitorSelector::Primary, &frames, None, None, None, true),
            Some(0)
        );
    }

    #[test]
    #[should_panic(expected = "primary_monitor() reported none")]
    fn primary_missing_off_wayland_panics() {
        let frames = [LEFT, RIGHT];
        let _ = resolve_selector(MonitorSelector::Primary, &frames, None, None, None, false);
    }

    #[test]
    fn pointer_resolves_by_containment() {
        let frames = [LEFT, RIGHT];
        assert_eq!(
            resolve_selector(
                MonitorSelector::Pointer,
                &frames,
                Some(0),
                Some((2400.0, 600.0)),
                None,
                false,
            ),
            Some(1)
        );
        // The far edge is exclusive: exactly on the boundary is the next
        // monitor's ground.
        assert_eq!(
            resolve_selector(
                MonitorSelector::Pointer,
                &frames,
                Some(0),
                Some((1600.0, 600.0)),
                None,
                false,
            ),
            Some(1)
        );
        assert_eq!(
            resolve_selector(
                MonitorSelector::Pointer,
                &frames,
                Some(0),
                Some((1599.9, 600.0)),
                None,
                false,
            ),
            Some(0)
        );
    }

    #[test]
    fn pointer_outside_every_monitor_resolves_nearest() {
        let frames = [LEFT, RIGHT];
        // Below LEFT, horizontally past RIGHT's start: LEFT is nearer, so a
        // primary at index 1 does not win it.
        assert_eq!(
            resolve_selector(
                MonitorSelector::Pointer,
                &frames,
                Some(1),
                Some((800.0, 5000.0)),
                None,
                false,
            ),
            Some(0)
        );
        // Deep in the bottom-right gap: RIGHT is nearer than LEFT.
        assert_eq!(
            resolve_selector(
                MonitorSelector::Pointer,
                &frames,
                Some(0),
                Some((9999.0, 9999.0)),
                None,
                false,
            ),
            Some(1)
        );
    }

    #[test]
    fn pointer_missing_resolves_first_on_wayland_panics_elsewhere() {
        let frames = [LEFT, RIGHT];
        // Wayland answers no global pointer position: warn + first monitor.
        assert_eq!(
            resolve_selector(MonitorSelector::Pointer, &frames, Some(1), None, None, true),
            Some(0)
        );
    }

    #[test]
    #[should_panic(expected = "got no pointer position")]
    fn pointer_missing_off_wayland_panics() {
        let frames = [LEFT, RIGHT];
        let _ = resolve_selector(
            MonitorSelector::Pointer,
            &frames,
            Some(0),
            None,
            None,
            false,
        );
    }

    #[test]
    fn focused_resolves_the_focused_monitor_else_primary() {
        let frames = [LEFT, RIGHT];
        assert_eq!(
            resolve_selector(
                MonitorSelector::Focused,
                &frames,
                Some(0),
                None,
                Some(1),
                false
            ),
            Some(1)
        );
        // No focused window: the selector resolves as `Primary`.
        assert_eq!(
            resolve_selector(
                MonitorSelector::Focused,
                &frames,
                Some(1),
                None,
                None,
                false
            ),
            Some(1)
        );
        // A stale index outside the list resolves as primary too.
        assert_eq!(
            resolve_selector(
                MonitorSelector::Focused,
                &frames,
                Some(0),
                None,
                Some(9),
                false
            ),
            Some(0)
        );
    }

    #[test]
    fn zero_monitors_resolves_none() {
        assert_eq!(
            resolve_selector(
                MonitorSelector::Pointer,
                &[],
                None,
                Some((0.0, 0.0)),
                None,
                false
            ),
            None
        );
        assert_eq!(
            resolve_selector(MonitorSelector::Primary, &[], Some(0), None, None, false),
            None
        );
    }
}
