//! Render utilities for handling view dispatching.

use core::{any::TypeId, fmt::Debug};
use std::collections::HashMap;

use waterui_core::{AnyView, Environment, View};

/// Type alias for the handler function to reduce complexity.
type HandlerFn<T, C, R> = Box<dyn Fn(&mut T, C, AnyView, &Environment) -> R>;

/// A dispatcher that can register and dispatch views based on their types.
pub struct ViewDispatcher<T, C, R> {
    state: T,
    map: HashMap<TypeId, HandlerFn<T, C, R>>,
}

impl<T: Default, C, R> Default for ViewDispatcher<T, C, R> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T, C, R> Debug for ViewDispatcher<T, C, R> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "ViewDispatcher<{}, {}>(..)",
            core::any::type_name::<T>(),
            core::any::type_name::<R>()
        )
    }
}

impl<T, C, R> ViewDispatcher<T, C, R> {
    /// Creates a new [`ViewDispatcher`] with the given state.
    pub fn new(state: T) -> Self {
        Self {
            state,
            map: HashMap::new(),
        }
    }

    /// Registers a handler for a specific view type.
    ///
    /// # Panics
    ///
    /// Panics if the view cannot be downcast to the expected type.
    pub fn register<V: View>(
        &mut self,
        handler: impl 'static + Fn(&mut T, C, V, &Environment) -> R,
    ) {
        self.map.insert(
            TypeId::of::<V>(),
            Box::new({
                move |state, context, view: AnyView, env| {
                    let v = view.downcast::<V>().unwrap();
                    handler(state, context, *v, env)
                }
            }),
        );
    }

    /// Dispatches a view to the appropriate handler.
    pub fn dispatch<V: View>(&mut self, view: V, env: &Environment, context: C) -> R {
        self.dispatch_any(AnyView::new(view), env, context)
    }

    /// Dispatches a view to the appropriate handler.
    pub fn dispatch_any(&mut self, view: AnyView, env: &Environment, context: C) -> R {
        let type_id = view.type_id();

        let view = match view.downcast::<AnyView>() {
            Ok(any) => return self.dispatch(*any, env, context),
            Err(view) => view,
        };

        if let Some(handler) = self.map.get(&type_id) {
            handler(&mut self.state, context, view, env)
        } else {
            self.dispatch(view.body(env), env, context)
        }
    }
}
