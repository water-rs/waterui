//! View dispatching infrastructure for `WaterUI` backends.
//!
//! The [`ViewDispatcher`] maps [`AnyView`](waterui_core::AnyView) instances to
//! backend-specific handlers based on their concrete types.

use core::{any::TypeId, fmt::Debug};
use std::cell::Cell;
use std::collections::HashMap;

use waterui_core::{AnyView, Environment, View};

/// Type alias for the handler function signature.
type HandlerFn<T, C, R> = Box<dyn Fn(&mut T, C, AnyView, &Environment) -> R>;

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
#[derive(Default)]
pub struct ViewDispatcher<T, C, R> {
    state: T,
    handlers: HashMap<TypeId, HandlerFn<T, C, R>>,
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
                eprintln!("[gtk-dispatch] {}{}", "  ".repeat(current), view_name);
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
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ViewDispatcher<{}, {}, {}>(handlers: {})",
            core::any::type_name::<T>(),
            core::any::type_name::<C>(),
            core::any::type_name::<R>(),
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
    ///
    /// The handler receives:
    /// - `&mut T`: Mutable reference to the dispatcher's state
    /// - `C`: Context from the dispatch call
    /// - `V`: The concrete view type (after downcasting)
    /// - `&Environment`: The `WaterUI` environment
    ///
    /// # Panics
    ///
    /// The handler will panic if the view cannot be downcast (should never happen
    /// if the dispatcher is used correctly).
    pub fn register<V: View>(
        &mut self,
        handler: impl 'static + Fn(&mut T, C, V, &Environment) -> R,
    ) {
        self.handlers.insert(
            TypeId::of::<V>(),
            Box::new(move |state, context, view: AnyView, env| {
                let v = view.downcast::<V>().expect("failed to downcast view");
                handler(state, context, *v, env)
            }),
        );
    }

    /// Dispatches a view to its registered handler.
    ///
    /// This is a convenience method that wraps the view in [`AnyView`].
    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, context: C) -> R {
        self.dispatch_any(AnyView::new(view), env, context)
    }

    /// Dispatches an [`AnyView`] to its registered handler.
    ///
    /// If no handler is registered for the view's concrete type, the dispatcher
    /// calls `body()` on the view and tries again with the result.
    pub fn dispatch_any(&mut self, view: AnyView, env: &Environment, context: C) -> R {
        let debug_enabled = std::env::var_os("WATERUI_DISPATCH_DEBUG").is_some();
        let view_name = view.name();
        let _trace_guard = DispatchTraceGuard::enter(debug_enabled, view_name);
        let type_id = view.type_id();

        // Handle nested AnyView (unwrap and recurse)
        let view = match view.downcast::<AnyView>() {
            Ok(inner) => return self.dispatch_any(*inner, env, context),
            Err(view) => view,
        };

        // Look up handler by type
        if let Some(handler) = self.handlers.get(&type_id) {
            handler(&mut self.state, context, view, env)
        } else {
            // No handler found - expand body and recurse
            let body = view.body(env);
            self.dispatch(body, env, context)
        }
    }

    /// Returns the number of registered handlers.
    #[must_use]
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }
}
