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

pub trait IntoView {
    type Output: View;
    fn into_view(self, env: &Environment) -> Self::Output;
}

impl<V: View> IntoView for V {
    type Output = V;
    fn into_view(self, _env: &Environment) -> Self::Output {
        self
    }
}

pub trait TupleViews {
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

pub trait ConfigurableView: View {
    type Config: 'static;
    fn config(self) -> Self::Config;
}

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
    pub fn new<V2, F>(f: F) -> Self
    where
        V: ConfigurableView,
        V2: View,
        F: Fn(Environment, V::Config) -> V2 + 'static,
    {
        Self::from(f)
    }
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
