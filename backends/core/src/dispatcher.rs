//! View dispatching infrastructure for `WaterUI` backends.
//!
//! The [`ViewDispatcher`] maps views to backend-specific handlers based on
//! their concrete types. The [`dispatch`](ViewDispatcher::dispatch) method
//! provides a zero-allocation generic path where views stay on the stack
//! and are passed via `&mut dyn Any` downcast.

use core::any::type_name;
use core::fmt;
use core::{
    any::{Any, TypeId},
    fmt::Debug,
};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;

use nami::with_local_binding_factory;
use waterui_core::LocalStateStore;
use waterui_core::{AnyView, Environment, LocalStateScope, View};

/// Handler that accepts a stack-allocated view via `&mut dyn Any` (`Option<V>` slot).
type RawHandlerFn<T, C, R> = Box<dyn Fn(&mut T, C, &mut dyn Any, &Environment) -> R>;

/// Handler that accepts a boxed `AnyView` (for when the view is already type-erased).
type BoxedHandlerFn<T, C, R> = Box<dyn Fn(&mut T, C, AnyView, &Environment) -> R>;

/// Holds both dispatch paths for a single registered view type.
struct HandlerEntry<T, C, R> {
    /// For zero-alloc dispatch: takes `&mut dyn Any` (`Option<V>` slot on stack).
    raw: RawHandlerFn<T, C, R>,
    /// For `AnyView` dispatch: takes `AnyView`, downcasts to `V`, calls handler.
    boxed: BoxedHandlerFn<T, C, R>,
}

/// A dispatcher that routes views to type-specific handlers.
///
/// Backends register handlers for concrete view types (e.g., `Native<TextConfig>`).
/// When dispatching a view, the dispatcher:
/// 1. Looks up a handler by the view's `TypeId`
/// 2. If found, invokes the handler
/// 3. If not found, calls `body()` on the view and recurses
///
/// # Type Parameters
///
/// - `T`: State type owned by the dispatcher
/// - `C`: Context type passed to handlers (e.g., parent widget)
/// - `R`: Return type from handlers (e.g., created widget)
///
/// # Example
///
/// ```ignore
/// let mut dispatcher: ViewDispatcher<(), GtkContext, gtk4::Widget> = ViewDispatcher::new();
///
/// dispatcher.register::<Native<TextConfig>>(|state, ctx, config, env| {
///     let label = gtk4::Label::new(Some(&config.content.get().to_plain()));
///     label.upcast()
/// });
///
/// let widget = dispatcher.dispatch(my_view, &env, ctx);
/// ```
pub struct ViewDispatcher<T, C, R> {
    state: T,
    handlers: HashMap<TypeId, HandlerEntry<T, C, R>>,
}

impl<T: Default, C, R> Default for ViewDispatcher<T, C, R> {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static DISPATCH_DEPTH: Cell<usize> = const { Cell::new(0) };
}

struct DispatchTraceGuard {
    enabled: bool,
}

impl DispatchTraceGuard {
    fn enter(enabled: bool, view_name: &str) -> Self {
        if enabled {
            DISPATCH_DEPTH.with(|depth| {
                let current = depth.get();
                tracing::debug!("[dispatch] {}{}", "  ".repeat(current), view_name);
                depth.set(current + 1);
            });
        }
        Self { enabled }
    }
}

impl Drop for DispatchTraceGuard {
    fn drop(&mut self) {
        if self.enabled {
            DISPATCH_DEPTH.with(|depth| {
                let current = depth.get();
                if current > 0 {
                    depth.set(current - 1);
                }
            });
        }
    }
}

fn with_local_bindings<R>(env: &Environment, f: impl FnOnce() -> R) -> R {
    let scope = env
        .get::<LocalStateScope>()
        .unwrap_or_else(|| {
            panic!("waterui-backend-core dispatch requires LocalStateScope for local bindings")
        })
        .clone();
    let store = env
        .get::<LocalStateStore>()
        .unwrap_or_else(|| {
            panic!("waterui-backend-core dispatch requires LocalStateStore for local bindings")
        })
        .clone();
    with_local_binding_factory(
        Rc::new(move |type_id, type_name, init| {
            store.get_or_init_dynamic(&scope, type_id, type_name, init)
        }),
        f,
    )
}

impl<T: Default, C, R> ViewDispatcher<T, C, R> {
    /// Creates a new dispatcher with default state.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: T::default(),
            handlers: HashMap::new(),
        }
    }
}

impl<T, C, R> Debug for ViewDispatcher<T, C, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ViewDispatcher<{}, {}, {}>(handlers: {})",
            type_name::<T>(),
            type_name::<C>(),
            type_name::<R>(),
            self.handlers.len()
        )
    }
}

impl<T, C, R> ViewDispatcher<T, C, R> {
    /// Creates a new dispatcher with the provided state.
    pub fn with_state(state: T) -> Self {
        Self {
            state,
            handlers: HashMap::new(),
        }
    }

    /// Returns a reference to the dispatcher's state.
    #[must_use]
    pub const fn state(&self) -> &T {
        &self.state
    }

    /// Returns a mutable reference to the dispatcher's state.
    pub const fn state_mut(&mut self) -> &mut T {
        &mut self.state
    }

    /// Registers a handler for a specific view type.
    pub fn register<V: View>(
        &mut self,
        handler: impl 'static + Clone + Fn(&mut T, C, V, &Environment) -> R,
    ) {
        let h_raw = handler.clone();
        let h_boxed = handler;

        let entry = HandlerEntry {
            raw: Box::new(move |state, ctx, slot: &mut dyn Any, env| {
                let v = slot
                    .downcast_mut::<Option<V>>()
                    .expect("type mismatch in raw dispatch")
                    .take()
                    .expect("view already taken from slot");
                h_raw(state, ctx, v, env)
            }),
            boxed: Box::new(move |state, ctx, view: AnyView, env| {
                let v = *view
                    .downcast::<V>()
                    .expect("type mismatch in boxed dispatch");
                h_boxed(state, ctx, v, env)
            }),
        };
        self.handlers.insert(TypeId::of::<V>(), entry);
    }

    /// Dispatches a view to its registered handler.
    ///
    /// For concrete types: zero-allocation — the view stays on the stack.
    /// For `AnyView`: the inner type is inspected and the boxed path is used.
    ///
    /// If no handler is found, `body()` is called and the result is dispatched
    /// recursively.
    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, context: C) -> R {
        let tid = TypeId::of::<V>();

        let debug_enabled = std::env::var_os("WATERUI_DISPATCH_DEBUG").is_some();
        let _trace_guard = DispatchTraceGuard::enter(debug_enabled, type_name::<V>());

        // AnyView is already heap-allocated — unwrap it and dispatch by inner TypeId.
        if tid == TypeId::of::<AnyView>() {
            // Extract the AnyView from the generic V via Any downcast.
            let mut slot = Some(view);
            let any_view = (&mut slot as &mut dyn Any)
                .downcast_mut::<Option<AnyView>>()
                .expect("AnyView downcast should succeed")
                .take()
                .expect("AnyView option should contain a value");

            return self.dispatch_boxed(any_view, env, context);
        }

        // Try handler (view stays on stack, downcast via &mut dyn Any)
        if let Some(entry) = self.handlers.get(&tid) {
            let mut slot: Option<V> = Some(view);
            return (entry.raw)(&mut self.state, context, &mut slot as &mut dyn Any, env);
        }

        // No handler found: expand body() and recurse — monomorphized, zero allocation
        let body_env = env
            .get::<LocalStateScope>()
            .map_or_else(|| env.clone(), |scope| env.extending(scope.reset()));
        let body_content_env = env
            .get::<LocalStateScope>()
            .map_or_else(|| env.clone(), |scope| env.extending(scope.child(0)));
        let body_eval_env = body_env.clone();
        self.dispatch_boxed(
            with_local_bindings(&body_env, move || AnyView::new(view.body(&body_eval_env))),
            &body_content_env,
            context,
        )
    }

    /// Internal: dispatches an already-boxed `AnyView` by its inner `TypeId`.
    fn dispatch_boxed(&mut self, view: AnyView, env: &Environment, context: C) -> R {
        let debug_enabled = std::env::var_os("WATERUI_DISPATCH_DEBUG").is_some();
        let _trace_guard = DispatchTraceGuard::enter(debug_enabled, view.name());
        let type_id = view.type_id();

        // AnyView::new() guarantees no nesting — no unwrap check needed.

        if let Some(entry) = self.handlers.get(&type_id) {
            (entry.boxed)(&mut self.state, context, view, env)
        } else {
            // body() returns impl View which AnyView::new wraps back into AnyView
            let body_env = env
                .get::<LocalStateScope>()
                .map_or_else(|| env.clone(), |scope| env.extending(scope.reset()));
            let body_content_env = env
                .get::<LocalStateScope>()
                .map_or_else(|| env.clone(), |scope| env.extending(scope.child(0)));
            let body_eval_env = body_env.clone();
            self.dispatch_boxed(
                with_local_bindings(&body_env, move || AnyView::new(view.body(&body_eval_env))),
                &body_content_env,
                context,
            )
        }
    }

    /// Returns the number of registered handlers.
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }
}
