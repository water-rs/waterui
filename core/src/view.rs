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

use crate::{AnyView, Environment, components::Metadata};
use alloc::{boxed::Box, vec::Vec};

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
pub trait View: 'static {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    fn body(self, _env: &Environment) -> impl View;
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
}

impl<V: View> View for Option<V> {
    fn body(self, _env: &Environment) -> impl View {
        self.map_or_else(|| AnyView::new(()), |view| AnyView::new(view))
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

/// A trait for converting collections and tuples of views into a vector of `AnyView`s.
///
/// This trait provides a uniform way to handle multiple views, allowing them
/// to be converted into a homogeneous collection that can be processed consistently.
pub trait TupleViews {
    /// Converts the implementing type into a vector of `AnyView` objects.
    ///
    /// # Returns
    ///
    /// A `Vec<AnyView>` containing each view from the original collection.
    fn into_views(self) -> Vec<AnyView>;
}

impl<V: View> TupleViews for Vec<V> {
    fn into_views(self) -> Vec<AnyView> {
        self.into_iter()
            .map(|content| AnyView::new(content))
            .collect()
    }
}

impl<V: View, const N: usize> TupleViews for [V; N] {
    fn into_views(self) -> Vec<AnyView> {
        self.into_iter()
            .map(|content| AnyView::new(content))
            .collect()
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

pub trait ViewConfiguration: 'static {
    // Note: the result would ignore any hook in the environment, to avoid infinite recursion.
    type View: View;
    fn render(self) -> Self::View;
}

// Note: Hook could change the behavior of the view dynamically based on the environment
// only view implemented `ViewConfiguration` can be hooked.
// A struct implemented `View` can be not concrete, but `ViewConfiguration` providing
// `config()` method, which would return a concrete type.
// By add `Hook<Config>` into `Environment`, a
pub struct Hook<C>(Box<dyn Fn(&Environment, C) -> AnyView>);

impl<C> core::fmt::Debug for Hook<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Modifier<{}>(..)", core::any::type_name::<C>())
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
    pub fn new<V, F>(f: F) -> Self
    where
        V: View,
        F: Fn(&Environment, C) -> V + 'static,
    {
        Self::from(f)
    }

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
        impl <$($ty:View,)*>TupleViews for ($($ty,)*){
            fn into_views(self) -> Vec<AnyView> {
                let ($($ty),*)=self;
                alloc::vec![$(AnyView::new($ty)),*]
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

impl View for ! {
    fn body(self, _env: &Environment) -> impl View {}
}
