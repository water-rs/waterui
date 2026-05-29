//! Cross-platform filter handler registry.
//!
//! `FilterHandlerRegistry` lets a backend intercept any concrete filter type
//! (`F: Effect`) emitted by `Filtered<V, F>::body()` and route it through a
//! platform-native compositor instead of falling through to the wgpu
//! [`AppliedFilter`](crate::filter_view::AppliedFilter) path.
//!
//! Architectural constraints baked into this module:
//!
//! - **Platform-agnostic.** Apple, Android, GTK, web, hydrolysis-m3, future
//!   compositors — every backend uses the same registration API. The
//!   registry stores a handler-per-`TypeId<F>` and never knows which
//!   platform it ultimately serves.
//! - **Filter-agnostic.** No special casing of Blur. Any `Effect`-shaped
//!   filter (single-pass color, separable spatial, multi-input, custom
//!   user filter) can be intercepted by registering a handler keyed by
//!   the concrete type.
//! - **Compositor optimisations live OUTSIDE `filtrate`.** The registry
//!   itself is in `waterui-graphics`; backend-specific handler bodies live
//!   in their respective backend crates and reach Rust over FFI. `filtrate`
//!   stays a pure GPU library that knows nothing about native compositors.
//! - **Always falls through cleanly.** When no handler matches, the
//!   default wgpu pipeline runs and the result is identical across every
//!   platform. Native paths are an *opt-in* optimisation, not a
//!   correctness requirement.
//!
//! # Usage
//!
//! ```ignore
//! use waterui_graphics::filter_registry::{FilterHandler, FilterHandlerRegistry};
//! use waterui_graphics::filter_view::Blur;
//! use waterui_core::{AnyView, Environment};
//!
//! struct AppleBlurHandler;
//! impl FilterHandler<Blur> for AppleBlurHandler {
//!     fn lower(&self, content: AnyView, filter: Blur, _env: &Environment) -> AnyView {
//!         // Construct a UIVisualEffectView wrapper here.
//!         # AnyView::new(content)
//!     }
//! }
//!
//! let mut registry = FilterHandlerRegistry::new();
//! registry.register::<Blur, _>(AppleBlurHandler);
//! ```

extern crate alloc;
extern crate std;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::any::{Any, TypeId};
use std::collections::HashMap;
use std::sync::RwLock;

use waterui_core::{AnyView, Environment, metadata::MetadataKey};

use crate::filter_view::AppliedFilter;
use filtrate::Effect;

/// Backend-supplied handler that lowers a specific filter type into a
/// platform-native view subtree.
///
/// The handler receives ownership of both the content view (already
/// type-erased) and the concrete filter; it is free to subscribe to any
/// reactive parameters the filter exposes (for example,
/// `Blur::radius_signal()`).
pub trait FilterHandler<F: Effect>: Send + Sync + 'static {
    /// Optional runtime predicate. Defaults to always-active. Backends
    /// override when they only support a sub-range of inputs (for
    /// example, "blur radius below 32 px maps to `UIVisualEffectView`,
    /// otherwise fall back to wgpu").
    fn matches(&self, _filter: &F, _env: &Environment) -> bool {
        true
    }

    /// Lower the filter into a platform-native view subtree. Called only
    /// when `matches` returned `true`.
    fn lower(&self, content: AnyView, filter: F, env: &Environment) -> AnyView;
}

/// Object-safe handler used internally to store heterogeneous handlers in
/// the registry's type-erased map.
trait ErasedFilterHandler: Send + Sync + 'static {
    /// Returns `true` if this handler wants to take over the filter.
    /// `filter` must wrap the same concrete type the handler was registered
    /// against; otherwise the implementation panics.
    fn matches_erased(&self, filter: &dyn Any, env: &Environment) -> bool;

    /// Lower the filter. `filter` must wrap the same concrete type the
    /// handler was registered against; otherwise the implementation panics.
    fn lower_erased(&self, content: AnyView, filter: Box<dyn Any>, env: &Environment) -> AnyView;
}

struct HandlerErasure<F: Effect, H: FilterHandler<F>> {
    handler: H,
    _marker: core::marker::PhantomData<fn(F)>,
}

impl<F: Effect, H: FilterHandler<F>> ErasedFilterHandler for HandlerErasure<F, H> {
    fn matches_erased(&self, filter: &dyn Any, env: &Environment) -> bool {
        let filter = filter
            .downcast_ref::<F>()
            .expect("FilterHandlerRegistry: handler invoked with wrong concrete filter type");
        self.handler.matches(filter, env)
    }

    fn lower_erased(&self, content: AnyView, filter: Box<dyn Any>, env: &Environment) -> AnyView {
        let filter: Box<F> = filter
            .downcast::<F>()
            .expect("FilterHandlerRegistry: handler invoked with wrong concrete filter type");
        self.handler.lower(content, *filter, env)
    }
}

/// Registry of native compositor handlers keyed by concrete filter type.
///
/// The registry is normally inserted into the [`Environment`] once during
/// app startup (typically by the backend's `install_filter_handlers`
/// function). Views that emit filtered subtrees consult the registry via
/// [`FilterHandlerRegistry::lower`] when their `body()` runs.
pub struct FilterHandlerRegistry {
    handlers: RwLock<HashMap<TypeId, Arc<dyn ErasedFilterHandler>>>,
}

impl Default for FilterHandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl core::fmt::Debug for FilterHandlerRegistry {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let count = self.handlers.read().map_or(0, |guard| guard.len());
        f.debug_struct("FilterHandlerRegistry")
            .field("registered_filters", &count)
            .finish()
    }
}

impl MetadataKey for FilterHandlerRegistry {}

impl FilterHandlerRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
        }
    }

    /// Register a handler for the concrete filter type `F`. The handler is
    /// invoked at view-tree expansion time when `Filtered<V, F>` resolves.
    ///
    /// Registering twice for the same `F` replaces the previous handler;
    /// the most recent registration wins.
    ///
    /// # Panics
    ///
    /// Panics if the registry write lock is poisoned.
    pub fn register<F: Effect, H: FilterHandler<F>>(&self, handler: H) {
        let erasure = HandlerErasure::<F, H> {
            handler,
            _marker: core::marker::PhantomData,
        };
        let mut guard = self
            .handlers
            .write()
            .expect("FilterHandlerRegistry: write lock poisoned");
        guard.insert(TypeId::of::<F>(), Arc::new(erasure));
    }

    /// Remove the handler registered for `F`, if any.
    ///
    /// # Panics
    ///
    /// Panics if the registry write lock is poisoned.
    pub fn unregister<F: Effect>(&self) -> bool {
        let mut guard = self
            .handlers
            .write()
            .expect("FilterHandlerRegistry: write lock poisoned");
        guard.remove(&TypeId::of::<F>()).is_some()
    }

    /// Number of registered handlers. Cheap, mostly useful for testing.
    pub fn len(&self) -> usize {
        self.handlers.read().map_or(0, |guard| guard.len())
    }

    /// Whether any handler is registered.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Try to lower the given filter via a registered native handler.
    ///
    /// # Errors
    ///
    /// Returns the original `(content, filter)` when no registered handler
    /// claims this filter.
    ///
    /// # Panics
    ///
    /// Panics if the registry read lock is poisoned.
    pub fn lower<F: Effect>(
        &self,
        content: AnyView,
        filter: F,
        env: &Environment,
    ) -> Result<AnyView, (AnyView, F)> {
        let handler = {
            let guard = self
                .handlers
                .read()
                .expect("FilterHandlerRegistry: read lock poisoned");
            guard.get(&TypeId::of::<F>()).cloned()
        };
        let Some(handler) = handler else {
            return Err((content, filter));
        };
        if !handler.matches_erased(&filter, env) {
            return Err((content, filter));
        }
        let lowered = handler.lower_erased(content, Box::new(filter), env);
        Ok(lowered)
    }
}

/// Default lowering used by [`FilteredView::body`]. Routes through the
/// registry first; falls back to the wgpu `AppliedFilter` metadata path
/// when no handler claims the filter.
pub(crate) fn lower_filtered<F: Effect>(content: AnyView, filter: F, env: &Environment) -> AnyView {
    if let Some(registry) = env.get::<FilterHandlerRegistry>() {
        match registry.lower(content, filter, env) {
            Ok(view) => return view,
            Err((content, filter)) => {
                return AnyView::new(waterui_core::Metadata::new(
                    content,
                    AppliedFilter::new(filter),
                ));
            }
        }
    }
    AnyView::new(waterui_core::Metadata::new(
        content,
        AppliedFilter::new(filter),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use waterui_core::{AnyView, Environment};

    /// A toy effect (not GPU-backed) for testing the registry's `TypeId`
    /// dispatch. We never actually run it.
    #[derive(Clone, Copy, Debug)]
    struct DummyEffect;

    impl Effect for DummyEffect {
        fn setup(
            &mut self,
            _ctx: &filtrate::EffectContext,
        ) -> impl core::future::Future<Output = filtrate::EffectSetupResult> {
            core::future::ready(Ok(()))
        }
        fn encode_render(
            &mut self,
            _input: &filtrate::EffectInput,
            _output: &filtrate::EffectOutput,
            _encoder: &mut wgpu::CommandEncoder,
        ) -> filtrate::EffectRenderResult {
            Ok(false)
        }
    }

    struct CountingHandler {
        calls: Arc<AtomicU32>,
    }

    impl FilterHandler<DummyEffect> for CountingHandler {
        fn lower(&self, content: AnyView, _filter: DummyEffect, _env: &Environment) -> AnyView {
            self.calls.fetch_add(1, Ordering::Relaxed);
            content
        }
    }

    #[test]
    fn registry_starts_empty() {
        let r = FilterHandlerRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn registry_registers_and_unregisters_by_typeid() {
        let r = FilterHandlerRegistry::new();
        r.register::<DummyEffect, _>(CountingHandler {
            calls: Arc::new(AtomicU32::new(0)),
        });
        assert_eq!(r.len(), 1);
        assert!(r.unregister::<DummyEffect>());
        assert!(r.is_empty());
        assert!(
            !r.unregister::<DummyEffect>(),
            "second unregister is a no-op"
        );
    }

    #[test]
    fn registry_dispatches_to_matching_handler() {
        let r = FilterHandlerRegistry::new();
        let calls = Arc::new(AtomicU32::new(0));
        r.register::<DummyEffect, _>(CountingHandler {
            calls: Arc::clone(&calls),
        });

        let env = Environment::new();
        let content = AnyView::new(());
        let result = r.lower(content, DummyEffect, &env);
        assert!(result.is_ok(), "registered handler should claim the filter");
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn registry_falls_through_when_no_handler_registered() {
        let r = FilterHandlerRegistry::new();
        let env = Environment::new();
        let content = AnyView::new(());
        let result = r.lower(content, DummyEffect, &env);
        assert!(
            result.is_err(),
            "missing handler should signal fall-through"
        );
    }
}
