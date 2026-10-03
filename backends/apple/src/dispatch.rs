//! Instance-owned native view dispatch. Composers expand through their public
//! bodies and engine hooks; registered leaves stay entirely in Rust.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
#[cfg(debug_assertions)]
use alloc::vec::Vec;
use core::any::TypeId;
use core::fmt;

use alloc::rc::Rc;
use waterui_backend_core::{AnyView, Environment, View};

use crate::contract::{NativeLeaf, RenderContext};

/// The handler signature every port implements: the erased view downcast to
/// the claimed type, the render context, the leaf it becomes.
pub(crate) type Handler = Box<dyn Fn(AnyView, &RenderContext<'_>) -> NativeLeaf>;

/// The dispatch table. Built once at startup by [`crate::registry`], then
/// immutable: children register through it, it is never written after
/// handlers could run.
pub(crate) struct Dispatcher {
    handlers: BTreeMap<TypeId, Handler>,
    /// The claimed type's name per registered type, kept for the `Debug`
    /// dump; release builds store none.
    #[cfg(debug_assertions)]
    names: Vec<&'static str>,
}

impl fmt::Debug for Dispatcher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut output = f.debug_struct("Dispatcher");
        output.field("handlers", &self.handlers.len());
        #[cfg(debug_assertions)]
        output.field("names", &self.names);
        output.finish()
    }
}

impl Dispatcher {
    /// An empty table; [`crate::registry`] fills it.
    pub(crate) fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
            #[cfg(debug_assertions)]
            names: Vec::new(),
        }
    }

    /// Claims `Native<C>`: a native payload the handler renders into a
    /// platform view.
    ///
    /// The handler receives the payload itself — the wrapper is downcast and
    /// unwrapped for it. Registering `C` here is how a component port owns a
    /// leaf: `TextField`'s `Native<ResolvedTextFieldConfig>`,
    /// `Native<Spacer>`, `Native<FixedContainer>`, and every other
    /// `raw_view!`/`configurable!` payload.
    pub(crate) fn register_native<C: waterui_core::NativeView + 'static>(
        &mut self,
        handler: impl Fn(C, &RenderContext<'_>) -> NativeLeaf + 'static,
    ) {
        let wrapped: Handler = Box::new(move |view, ctx| {
            let native = view
                .downcast::<waterui_backend_core::Native<C>>()
                .unwrap_or_else(|_| {
                    panic!(
                        "dispatcher claimed {} but the erased view was not one",
                        core::any::type_name::<C>()
                    )
                });
            handler(native.into_inner(), ctx)
        });
        self.handlers
            .insert(TypeId::of::<waterui_backend_core::Native<C>>(), wrapped);
        // The name the fallback's registry keys is the erased view's own:
        // `Native<C>`, not `C`.
        #[cfg(debug_assertions)]
        self.names
            .push(core::any::type_name::<waterui_backend_core::Native<C>>());
    }

    /// Claims `T` exactly as it appears in the view tree: a metadata wrapper
    /// (`Metadata<M>`, `IgnorableMetadata<M>`), or a composer the port takes
    /// before `body()` expands it.
    ///
    /// A `register_view` handler typically owns no platform view of its own:
    /// it applies `T`'s effect (an environment overlay, an attribute on the
    /// child's platform view) and returns the leaf its child rendered.
    #[cfg(any(
        feature = "accessibility_identifier",
        feature = "accessibility_metadata",
        feature = "anchored_overlay",
        feature = "border",
        feature = "clip_shape",
        feature = "context_menu",
        feature = "cursor",
        feature = "draggable",
        feature = "drop_destination",
        feature = "dynamic_range",
        feature = "focused",
        feature = "gesture",
        feature = "glass_background",
        feature = "hittable",
        feature = "ignore_safe_area",
        feature = "layout_priority",
        feature = "lifecycle_hook",
        feature = "material_background",
        feature = "multi_date_picker",
        feature = "navigation",
        feature = "offset",
        feature = "on_event",
        feature = "on_key_press",
        feature = "opacity",
        feature = "retain",
        feature = "rotation",
        feature = "scale",
        feature = "secure",
        feature = "shadow",
        feature = "with_env"
    ))]
    pub(crate) fn register_view<T: 'static>(
        &mut self,
        handler: impl Fn(T, &RenderContext<'_>) -> NativeLeaf + 'static,
    ) {
        let wrapped: Handler = Box::new(move |view, ctx| {
            let typed = view.downcast::<T>().unwrap_or_else(|_| {
                panic!(
                    "dispatcher claimed {} but the erased view was not one",
                    core::any::type_name::<T>()
                )
            });
            handler(*typed, ctx)
        });
        self.handlers.insert(TypeId::of::<T>(), wrapped);
        #[cfg(debug_assertions)]
        self.names.push(core::any::type_name::<T>());
    }

    /// The handler claiming `type_id`, if any.
    fn handler(&self, type_id: TypeId) -> Option<&Handler> {
        self.handlers.get(&type_id)
    }

    /// Renders `view` under `env` into the platform view it becomes.
    ///
    /// The walk: a registered handler claims the view; otherwise it expands
    /// through `body()`; a `Native<T>`/`Metadata<T>` that no handler claims
    /// crosses to the fallback. Returns `None` only when the fallback itself
    /// declines the view.
    pub(crate) fn render(
        self: &Rc<Self>,
        view: AnyView,
        env: &Environment,
        mtm: cocoa_ui::MainThreadMarker,
    ) -> Option<NativeLeaf> {
        let mut view = view;
        let ctx = RenderContext::new(env, self.clone(), mtm);
        loop {
            let type_id = view.type_id();
            if let Some(handler) = self.handler(type_id) {
                return Some(handler(view, &ctx));
            }
            view = AnyView::new(view.body(env));
        }
    }
}

/// Builds a dispatcher owned by this environment and its rendered leaves.
pub fn install(env: &mut Environment) {
    let mut dispatcher = Dispatcher::new();
    crate::registry::install(&mut dispatcher);
    env.insert(Rc::new(dispatcher));
}

pub(crate) fn dispatcher(env: &Environment) -> Rc<Dispatcher> {
    env.get::<Rc<Dispatcher>>()
        .expect("Apple dispatcher must be installed before rendering")
        .clone()
}

/// Stops filter-chain expansion at native leaves and metadata boundaries.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
pub(crate) fn is_native_boundary(view: &AnyView) -> bool {
    let name = view.name();
    name.starts_with("waterui_core::components::native::Native<")
        || name.starts_with("waterui_core::components::metadata::Metadata<")
}

/// Renders native content using the owning environment's dispatcher.
///
/// # Panics
/// Panics on an unhandled native component or off the main thread.
#[must_use]
pub fn render(view: AnyView, env: &Environment) -> NativeLeaf {
    let mtm = cocoa_ui::MainThreadMarker::new().expect("rendering runs on the main thread");
    dispatcher(env)
        .render(view, env, mtm)
        .expect("native view must render")
}
