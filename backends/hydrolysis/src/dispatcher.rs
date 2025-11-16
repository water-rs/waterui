//! View dispatching helpers shared by every renderer pipeline.
//!
//! The dispatcher maps [`AnyView`](waterui_core::AnyView) instances to backend-specific handlers.
//! Hydrolysis uses it while parsing the public view tree into render nodes.

use core::{any::TypeId, fmt::Debug};
use std::collections::HashMap;

use waterui_core::{AnyView, Environment, View};

/// Type alias for the handler function to reduce complexity.
type HandlerFn<T, C, R> = Box<dyn Fn(&mut T, C, AnyView, &Environment) -> R>;

/// A dispatcher that can register and dispatch views based on their types.
#[derive(Default)]
pub struct ViewDispatcher<T, C, R> {
    state: T,
    map: HashMap<TypeId, HandlerFn<T, C, R>>,
}

impl<T: Default, C, R> ViewDispatcher<T, C, R> {
    /// Creates a new [`ViewDispatcher`] with the given default state.
    pub fn new() -> Self {
        Self {
            state: T::default(),
            map: HashMap::new(),
        }
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
    /// Creates a new [`ViewDispatcher`] with the provided state.
    pub fn with_state(state: T) -> Self {
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
                    let v = view.downcast::<V>().expect("failed to downcast view");
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
