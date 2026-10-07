//! `ext-background-effect-v1` blur behind a Wayland window (water-rs/waterui#1857).
//!
//! The compositor applies background effects to a `wl_surface` through an
//! `ext_background_effect_surface_v1` object created by
//! `ext_background_effect_manager_v1`, bound on the `wl_display` winit already
//! owns — borrowed through the raw display handle, never a second connection.
//! A compositor that does not advertise the global leaves the translucent
//! window unblurred: the decided unsupported case `Material`'s documentation
//! states. One that advertises it applies blur by its own policy, announced
//! through its `capabilities` event — the ask is forwarded regardless.

use std::ffi::c_void;

use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_compositor::WlCompositor;
use wayland_client::protocol::wl_region::WlRegion;
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
    Event as ManagerEvent, ExtBackgroundEffectManagerV1,
};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::{
    Event as SurfaceEvent, ExtBackgroundEffectSurfaceV1,
};

/// The window's relationship to the compositor's behind-window blur —
/// [`WaylandBlur::new`] runs once, on the first blur ask, and an unsupported
/// answer is remembered rather than re-polled on every ask.
#[derive(Debug, Default)]
pub(super) enum WaylandBlurSupport {
    /// The window was never asked to blur behind: the manager has not been
    /// bound yet.
    #[default]
    NotAsked,
    /// The compositor does not advertise `ext_background_effect_manager_v1`
    /// — the decided unsupported case, which leaves the window translucent
    /// and unblurred.
    Unsupported,
    /// The bound manager, queue and surface-effect state.
    Bound(WaylandBlur),
}

/// The dispatch state type for the borrowed connection's own queue — this
/// binding acts on no event, so the state carries nothing.
#[derive(Debug)]
pub(super) struct WaylandBlurState;

/// The surface-level blur ask bound on the window's `wl_display`.
///
/// Keeps the borrowed connection's own event queue: every object this
/// binding creates is queued on it rather than on winit's queues, so the
/// compositor's `capabilities` policy events land here and go nowhere else.
#[derive(Debug)]
pub(super) struct WaylandBlur {
    /// The borrowed connection, kept for wrapping the window's `wl_surface`
    /// on the first ask.
    connection: Connection,
    /// Bound `ext_background_effect_manager_v1` (version 1 — the only one
    /// the protocol ships).
    manager: ExtBackgroundEffectManagerV1,
    /// `wl_compositor` for the `wl_region` the blur region is described in.
    compositor: WlCompositor,
    /// The surface's `ext_background_effect_surface_v1`, created lazily at
    /// the first blur ask — `get_background_effect` on a surface that
    /// already has one is the `background_effect_exists` protocol error.
    effect: Option<ExtBackgroundEffectSurfaceV1>,
    /// The queue the bound objects' events land on.
    queue: EventQueue<WaylandBlurState>,
}

impl WaylandBlur {
    /// Binds `ext_background_effect_manager_v1` on the `wl_display` behind
    /// `display`.
    ///
    /// `None` when the compositor does not expose the global — the decided
    /// unsupported case, which leaves the window translucent and unblurred.
    /// When it does, whether it applies blur is its own policy, so the ask
    /// is forwarded unconditionally.
    ///
    /// `display` must be the `wl_display` pointer from a winit window's raw
    /// display handle: the event loop that owns it outlives the window, and
    /// the backend only ever borrows it.
    ///
    /// # Panics
    /// Panics when the registry queue cannot be initialised or when a global
    /// the compositor advertised fails to bind — neither is an unsupported
    /// case.
    pub(super) fn new(display: *mut c_void) -> Option<Self> {
        // SAFETY: `display` is the live `wl_display` winit owns for the event
        // loop, which outlives the window this state is stored on;
        // `from_foreign_display` only borrows it and never disconnects it.
        let backend = unsafe { Backend::from_foreign_display(display.cast()) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<WaylandBlurState>(&connection)
            .expect("the borrowed wl_display refused its registry queue init");
        // A compositor that never advertised the manager is the decided
        // unsupported case; one that advertised it but fails the bind is a
        // bug, told apart by the registry's own global list.
        let advertised = globals.contents().with_list(|list| {
            list.iter()
                .any(|global| global.interface == ExtBackgroundEffectManagerV1::interface().name)
        });
        if !advertised {
            return None;
        }
        let manager: ExtBackgroundEffectManagerV1 =
            globals.bind(&queue.handle(), 1..=1, ()).expect(
                "the compositor advertised ext_background_effect_manager_v1 but the bind failed",
            );
        let compositor: WlCompositor = globals
            .bind(&queue.handle(), 1..=1, ())
            .expect("every compositor advertises wl_compositor, but the bind failed");
        Some(Self {
            connection,
            manager,
            compositor,
            effect: None,
            queue,
        })
    }

    /// Blurs everything behind `surface` — the window's `wl_surface` pointer
    /// from its raw window handle — or stops blurring it.
    ///
    /// The blur region is "specified in the surface-local coordinates, and
    /// clipped by the compositor to the surface size" (`set_blur_region` in
    /// `ext-background-effect-v1`), so a region larger than any surface is
    /// always the whole surface and needs no resize tracking. A `NULL`
    /// region removes the effect. The region is double-buffered state: it
    /// lands on the surface's next commit, which the frame the renderer
    /// presents after this per-frame application provides.
    ///
    /// # Panics
    /// Panics when dispatching or flushing the borrowed connection fails,
    /// when the window's `wl_surface` pointer does not resolve on this
    /// connection, or when a clear arrives with no effect bound — a clear
    /// follows a set, which binds the effect.
    pub(super) fn set_blur(&mut self, blur: bool, surface: *mut c_void) {
        // winit's reads of the shared socket queue this binding's events —
        // capability announcements, later globals — here; nothing acts on
        // them, but they are drained so the queue does not grow with the
        // window's lifetime.
        self.queue
            .dispatch_pending(&mut WaylandBlurState)
            .expect("dispatching the borrowed wl_display's queue failed");
        if blur {
            let effect = self.effect.get_or_insert_with(|| {
                // Wrap the `wl_surface` winit owns only long enough to pass
                // it to `get_background_effect` — the request keeps the
                // server-side association, not this proxy.
                // SAFETY: `surface` is the live `wl_surface` of the window
                // this state is stored on; `ObjectId::from_ptr` checks the
                // proxy's interface against `wl_surface`'s.
                let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }
                    .expect("the window's wl_surface pointer names no live wl_surface proxy");
                let wl_surface = WlSurface::from_id(&self.connection, id)
                    .expect("the window's wl_surface proxy is not on this connection");
                self.manager
                    .get_background_effect(&wl_surface, &self.queue.handle(), ())
            });
            let region = self.compositor.create_region(&self.queue.handle(), ());
            region.add(0, 0, i32::MAX, i32::MAX);
            effect.set_blur_region(Some(&region));
            // The blur region copies on request — the `wl_region` is done.
            region.destroy();
        } else {
            self.effect
                .as_ref()
                .expect("a blur clear follows a blur set, which binds the effect")
                .set_blur_region(None);
        }
        self.queue
            .flush()
            .expect("flushing the borrowed wl_display's queue failed");
    }
}

impl Drop for WaylandBlur {
    /// Destroys the protocol objects with the window that owns them. The
    /// manager's `destroy` leaves created objects alone, so the surface
    /// effect goes first. Teardown can follow the connection's own close,
    /// so the final flush's error has nowhere to report to and is dropped.
    fn drop(&mut self) {
        if let Some(effect) = self.effect.take() {
            effect.destroy();
        }
        self.manager.destroy();
        let _ = self.queue.flush();
    }
}

impl Dispatch<WlRegistry, GlobalListContents> for WaylandBlurState {
    fn event(
        _state: &mut Self,
        _registry: &WlRegistry,
        _event: <WlRegistry as Proxy>::Event,
        _contents: &GlobalListContents,
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        // Globals announced after init are `GlobalListContents`'s to record;
        // this binding only ever uses the two it bound in `new`.
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for WaylandBlurState {
    fn event(
        _state: &mut Self,
        _manager: &ExtBackgroundEffectManagerV1,
        _event: ManagerEvent,
        _data: &(),
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        // `capabilities` is compositor policy, not a client gate: "when the
        // capability goes away, the corresponding effect is no longer
        // applied by the compositor, even if it was set before" — so the
        // ask is forwarded regardless and the event needs no handling.
    }
}

impl Dispatch<WlCompositor, ()> for WaylandBlurState {
    fn event(
        _state: &mut Self,
        _compositor: &WlCompositor,
        _event: <WlCompositor as Proxy>::Event,
        _data: &(),
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        unreachable!("no events defined for wl_compositor");
    }
}

impl Dispatch<WlRegion, ()> for WaylandBlurState {
    fn event(
        _state: &mut Self,
        _region: &WlRegion,
        _event: <WlRegion as Proxy>::Event,
        _data: &(),
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        unreachable!("no events defined for wl_region");
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for WaylandBlurState {
    fn event(
        _state: &mut Self,
        _effect: &ExtBackgroundEffectSurfaceV1,
        _event: SurfaceEvent,
        _data: &(),
        _connection: &Connection,
        _handle: &QueueHandle<Self>,
    ) {
        unreachable!("no events defined for ext_background_effect_surface_v1");
    }
}
