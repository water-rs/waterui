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

use crate::{components::Metadata, AnyView, Environment};
use alloc::{boxed::Box, vec::Vec};
use waterui_reactive::compute::Unique;

/// View represents a part of the user interface.
///
/// You can create your custom view by implementing this trait. You just need to implement fit.
///
/// Users can also create a View using a function that returns another View. This allows for more
/// flexible and composable UI designs.
///
/// # Example
///
/// ```
/// fn greeting() -> impl View {
///     Text::new("Hello, World!")
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

impl<T: View> View for Unique<T> {
    fn body(self, _env: &Environment) -> impl View {
        self.0
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
        match self {
            Some(view) => AnyView::new(view),
            None => AnyView::new(()),
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
    type Config: 'static;

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

/// A wrapper for functions that modify configurable views.
///
/// `Modifier` provides a way to transform views with specific configurations,
/// enabling a consistent approach to view customization. Modifiers can be
/// reused across different instances of the same view type.
pub struct Modifier<V: ConfigurableView>(Box<dyn Fn(Environment, V::Config) -> AnyView>);

impl<V, V2, F> From<F> for Modifier<V>
where
    V: ConfigurableView,
    V2: View,
    F: Fn(Environment, V::Config) -> V2 + 'static,
{
    fn from(value: F) -> Self {
        Self(Box::new(move |mut env, config| {
            env.remove::<Self>();
            AnyView::new(Metadata::new(value(env.clone(), config), env))
        }))
    }
}

impl<V: ConfigurableView> Modifier<V> {
    /// Creates a new modifier that transforms a configurable view.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes an environment and a configuration, and returns a view.
    ///
    /// # Returns
    ///
    /// A new `Modifier` instance that can be applied to views of type `V`.
    pub fn new<V2, F>(f: F) -> Self
    where
        V: ConfigurableView,
        V2: View,
        F: Fn(Environment, V::Config) -> V2 + 'static,
    {
        Self::from(f)
    }

    /// Applies this modifier to a view configuration with the given environment.
    ///
    /// # Arguments
    ///
    /// * `env` - The environment context to use when applying the modification.
    /// * `config` - The configuration of the view to modify.
    ///
    /// # Returns
    ///
    /// An `AnyView` containing the modified view.
    pub fn modify(&self, env: Environment, config: V::Config) -> AnyView {
        (self.0)(env, config)
    }
}

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
