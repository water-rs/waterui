//! # View Module
//!
//! This module provides the core abstractions for building user interfaces.
//!
//! The primary types include:
//! - `View`: The fundamental trait for UI components
//! - `IntoView`: A trait for converting values into views
//! - `TupleViews`: A trait for working with collections of views
//! - `ConfigurableView`: A trait for views that can be configured
//! - `Modifier`: A type for modifying configurable views
//!
//! These abstractions support a declarative and composable approach to UI building, allowing
//! for flexible combinations of views and transformations.

use crate::backend::{Backend, ChildSlot};
use crate::{AnyView, Environment, components::Metadata};
use alloc::{boxed::Box, vec::Vec};
use core::any::type_name;
use core::fmt;

/// View represents a part of the user interface.
///
/// You can create your custom view by implementing this trait. You just need to implement fit.
///
/// Users can also create a View using a function that returns another View. This allows for more
/// flexible and composable UI designs.
///
/// # Example
///
/// ```rust
/// use waterui_core::View;
///
/// fn greeting() -> impl View {
///     "Hello, World!" // &'static str implements View
/// }
///
#[must_use]
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a WaterUI view",
    label = "expected a view",
    note = "Any `'static` type implementing `View::body` is a view, as is a function returning `impl View` or a bare `&'static str`, `String`, or `Str`. A `ForEach` is a collection of views, not a view — hand it to a container such as `Lazy::for_each` or `List::for_each`, and erase differing view types with `.anyview()`."
)]
pub trait View: 'static {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    fn body(self, _env: &Environment) -> impl View;

    /// Turns this view into `backend`'s node.
    ///
    /// A composite expands its `body` in place, so a tree of composites
    /// resolves through monomorphized code down to the native payloads and
    /// metadata the backend realizes. Native and metadata wrappers override
    /// it to reach the backend; nothing else does.
    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output
    where
        Self: Sized,
    {
        self.body(env).dispatch(backend, env, cx)
    }
}

impl<F: 'static + FnOnce() -> V, V: View> View for F {
    fn body(self, _env: &Environment) -> impl View {
        self()
    }
}

impl<V: View, E: View> View for Result<V, E> {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Ok(view) => AnyView::new(view),
            Err(view) => AnyView::new(view),
        }
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        match self {
            Ok(view) => view.dispatch(backend, env, cx),
            Err(view) => view.dispatch(backend, env, cx),
        }
    }
}

impl<V: View> View for Option<V> {
    fn body(self, _env: &Environment) -> impl View {
        self.map_or_else(|| AnyView::new(()), AnyView::new)
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        match self {
            Some(view) => view.dispatch(backend, env, cx),
            None => ().dispatch(backend, env, cx),
        }
    }
}

/// A trait for converting values into views.
///
/// This trait allows different types to be converted into View implementations,
/// enabling more flexible composition of UI elements.
pub trait IntoView {
    /// The resulting View type after conversion.
    type Output: View;

    /// Converts the implementing type into a View.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment containing context for the view conversion.
    ///
    /// # Returns
    ///
    /// A View implementation that can be used in the UI.
    fn into_view(self, env: &Environment) -> Self::Output;
}

impl<V: View> IntoView for V {
    type Output = V;
    fn into_view(self, _env: &Environment) -> Self::Output {
        self
    }
}

/// The children of a container: a tuple, an array or a `Vec` of views.
///
/// A container keeps its children's own types until it reaches a backend,
/// which receives each child in its [`Backend::Slot`]: a typed child stays
/// statically dispatched, and only an erased backend sees [`AnyView`]s.
pub trait ViewSequence: 'static {
    /// How many views the sequence holds.
    fn len(&self) -> usize;

    /// Whether the sequence holds no view.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Appends each view, in order, to `slots` as a slot of type `S`.
    fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>);

    /// The views, in order, each in a slot of type `S`.
    fn into_slots<S: ChildSlot>(self) -> Vec<S>
    where
        Self: Sized,
    {
        let mut slots = Vec::with_capacity(self.len());
        self.push_slots(&mut slots);
        slots
    }
}

impl<V: View> ViewSequence for Vec<V> {
    fn len(&self) -> usize {
        self.len()
    }

    fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>) {
        slots.extend(self.into_iter().map(S::from_view));
    }
}

impl<V: View, const N: usize> ViewSequence for [V; N] {
    fn len(&self) -> usize {
        N
    }

    fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>) {
        slots.extend(self.into_iter().map(S::from_view));
    }
}

/// A trait for views that can be configured with additional parameters.
///
/// This trait extends the basic `View` trait to support views that can be
/// customized with a configuration object, allowing for more flexible and
/// reusable UI components.
pub trait ConfigurableView: View {
    /// The configuration type associated with this view.
    ///
    /// This type defines the structure of configuration data that can be
    /// applied to the view.
    type Config: ViewConfiguration;

    /// Returns the configuration for this view.
    ///
    /// This method extracts the configuration data from the view, which can
    /// then be modified and applied to create customized versions of the view.
    ///
    /// # Returns
    ///
    /// The configuration object for this view.
    fn config(self) -> Self::Config;
}

/// A trait for types that can be used to configure views.
///
/// View configurations are used by hooks to modify how views are rendered.
pub trait ViewConfiguration: 'static {
    // Note: the result would ignore any hook in the environment, to avoid infinite recursion.
    /// The view type that this configuration produces.
    type View: View;
    /// Renders this configuration into a view.
    fn render(self) -> Self::View;
}

// Note: Hook could change the behavior of the view dynamically based on the environment
// only view implemented `ViewConfiguration` can be hooked.
// A struct implemented `View` can be not concrete, but `ViewConfiguration` providing
// `config()` method, which would return a concrete type.
// By add `Hook<Config>` into `Environment`, a
/// A function type for view hooks.
type HookFn<C> = Box<dyn Fn(&Environment, C) -> AnyView>;

/// A hook that can intercept and modify view configurations.
///
/// Hooks are used to apply global transformations to views based on their configuration.
pub struct Hook<C>(HookFn<C>);

impl<C> fmt::Debug for Hook<C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Modifier<{}>(..)", type_name::<C>())
    }
}

impl<V, C, F> From<F> for Hook<C>
where
    C: ViewConfiguration,
    V: View,
    F: Fn(&Environment, C) -> V + 'static,
{
    fn from(value: F) -> Self {
        Self(Box::new(move |env, config| {
            let mut env = env.clone();
            env.remove::<Self>(); // avoid infinite recursion
            AnyView::new(Metadata::new(value(&env, config), env))
        }))
    }
}

impl<C> Hook<C>
where
    C: ViewConfiguration,
{
    /// Creates a new hook from a function.
    ///
    /// The function will be called with the environment and configuration
    /// whenever a matching view configuration is encountered.
    pub fn new<V, F>(f: F) -> Self
    where
        V: View,
        F: Fn(&Environment, C) -> V + 'static,
    {
        Self::from(f)
    }

    /// Applies this hook to a configuration, producing a view.
    pub fn apply(&self, env: &Environment, config: C) -> AnyView {
        (self.0)(env, config)
    }
}

impl<C: ViewConfiguration> Hook<C> {}

macro_rules! impl_tuple_views {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:View,)*> ViewSequence for ($($ty,)*){
            fn len(&self) -> usize {
                let views: &[&str] = &[$(stringify!($ty)),*];
                views.len()
            }

            fn push_slots<S: ChildSlot>(self, slots: &mut Vec<S>) {
                // The trailing comma matters: `let (T) = self` is a parenthesized
                // binding, not a one-element tuple pattern, so without it a
                // single-child container would hold the tuple itself instead
                // of the view inside it.
                let ($($ty,)*) = self;
                $(slots.push(S::from_view($ty));)*
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

impl<V: View> View for (V,) {
    fn body(self, _env: &Environment) -> impl View {
        self.0
    }
}

#[cfg(feature = "nightly")]
impl View for ! {
    fn body(self, _env: &Environment) -> impl View {}
}
