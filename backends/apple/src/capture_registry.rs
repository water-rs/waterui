//! The render environment's registry of capturable outputs — the
//! Apple-private `CaptureRegistry` replacing the two process-global statics
//! the `IOSurface` path kept (`gpu_surface`'s `REGISTRY` and `filtered`'s
//! `FILTERS`, #1683).
//!
//! One registry lives per render `Environment`, installed by
//! [`crate::gpu_runtime::prepare`] before any dispatcher, view renderer or
//! leaf is created. Mounts register weak entries keyed by their platform
//! view's address; the mount guards remove them, so a dropped leaf can never
//! be captured again. GPU surfaces and filtered outputs register the same
//! way — a filtered output is a capturable too, or a nested filter would
//! capture a blank Metal layer. Capture resolution and first-frame
//! readiness read the same registry: the resolver `ViewCapture` consults,
//! and the per-output waiters `wait_for_first_frames` arms.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use cocoa_ui::capture::CapturableSurface;

/// How a platform view becomes a registry key — its object address, stable
/// for the mount's lifetime.
#[must_use]
pub fn view_key(view: &cocoa_ui::PlatformView) -> usize {
    std::ptr::from_ref(view).cast::<()>() as usize
}

/// A registered capturable output — weak, so a mounted leaf's own retain
/// decides whether a capture can still reach it.
struct CapturableEntry {
    surface: Weak<dyn CapturableSurface>,
}

/// The environment-owned registry: no process-global state.
pub struct CaptureRegistry {
    /// Capturable outputs by view key.
    capturables: RefCell<HashMap<usize, CapturableEntry>>,
}

impl std::fmt::Debug for CaptureRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CaptureRegistry")
            .field("capturables", &self.capturables.borrow().len())
            .finish_non_exhaustive()
    }
}

impl CaptureRegistry {
    /// An empty registry — installed once per environment.
    #[must_use]
    pub fn new() -> Self {
        Self {
            capturables: RefCell::new(HashMap::new()),
        }
    }

    /// Installs the registry in `env`. Idempotent: a second install on one
    /// environment keeps the live registry so mounts never lose entries.
    pub fn install(env: &mut waterui_backend_core::Environment) {
        if env.get::<Rc<Self>>().is_none() {
            env.insert(Rc::new(Self::new()));
        }
    }

    /// The environment's registry.
    ///
    /// # Panics
    ///
    /// When no registry was installed — [`install`] runs inside
    /// [`crate::gpu_runtime::prepare`] before any mount exists.
    #[must_use]
    pub fn get(env: &waterui_backend_core::Environment) -> Rc<Self> {
        env.get::<Rc<Self>>()
            .expect("capture registry is not installed in the WaterUI environment")
            .clone()
    }

    /// Registers `surface` for `view`; weak — the mount guard owns removal.
    pub fn insert_capturable(
        &self,
        view: &cocoa_ui::PlatformView,
        surface: &Rc<dyn CapturableSurface>,
    ) {
        self.capturables.borrow_mut().insert(
            view_key(view),
            CapturableEntry {
                surface: Rc::downgrade(surface),
            },
        );
    }

    /// The mount guard's removal — a dropped surface is never captured.
    pub fn remove_capturable(&self, view: &cocoa_ui::PlatformView) {
        self.capturables.borrow_mut().remove(&view_key(view));
    }

    /// The capturable `view` itself presents, if it is registered.
    pub fn resolve(&self, view: &cocoa_ui::PlatformView) -> Option<Rc<dyn CapturableSurface>> {
        self.capturables
            .borrow()
            .get(&view_key(view))
            .and_then(|entry| entry.surface.upgrade())
    }

    /// The resolver closure `ViewCapture::new` takes — bound to this
    /// registry, not a global.
    pub fn resolver(
        self: &Rc<Self>,
    ) -> impl Fn(&cocoa_ui::PlatformView) -> Option<Rc<dyn CapturableSurface>> + 'static {
        let registry = self.clone();
        move |view| registry.resolve(view)
    }

    /// Every registered capturable inside `root`'s subtree — `f` receives
    /// each live surface.
    pub fn collect_capturables(
        &self,
        root: &cocoa_ui::PlatformView,
        f: &mut impl FnMut(&Rc<dyn CapturableSurface>),
    ) {
        self.collect_into(root, f);
    }

    /// Recursive walk — resolved views stop the descent (a capturable's own
    /// subtree renders through its producer, not its children; its
    /// readiness covers them).
    fn collect_into(
        &self,
        view: &cocoa_ui::PlatformView,
        f: &mut impl FnMut(&Rc<dyn CapturableSurface>),
    ) {
        if let Some(surface) = self.resolve(view) {
            f(&surface);
            return;
        }
        for subview in cocoa_ui::view::subviews(view) {
            self.collect_into(&subview, f);
        }
    }
}

impl Default for CaptureRegistry {
    fn default() -> Self {
        Self::new()
    }
}
