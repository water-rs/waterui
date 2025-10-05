//! This module provides extension traits and builder patterns for creating and configuring views.
//!
//! # Overview
//!
//! The module implements:
//! - `ConfigViewExt`: Extends configurable views with common modifier patterns
//! - `ViewBuilder`: A trait for objects that can build views from an environment
//! - `ViewExt`: Extends all views with common styling and configuration methods
//!
//! These extensions help create a fluent API for constructing user interfaces.

use waterui_color::Color;
pub use waterui_core::view::*;
use waterui_core::{
    AnyView, Environment,
    env::With,
    handler::{Handler, HandlerFn, IntoHandler},
};

use alloc::boxed::Box;
use nami::{Binding, signal::IntoComputed};
use waterui_layout::{frame::Frame, padding::{EdgeInsets, Padding}, stack::Alignment};
use waterui_navigation::NavigationView;

use crate::background::{Background, ForegroundColor};
use crate::component::{Metadata, Text, badge::Badge, focu::Focused};
use waterui_core::id::TaggedView;

/// A trait for types that can build views from an environment.
pub trait ViewBuilder: 'static {
    /// Creates a view using the provided environment.
    ///
    /// # Arguments
    /// * `env` - The environment to use for building the view
    fn view(&self, env: &Environment) -> impl View;
}

impl<F, V> ViewBuilder for F
where
    F: Fn(Environment) -> V + 'static,
    V: View,
{
    fn view(&self, env: &Environment) -> impl View {
        (self)(env.clone())
    }
}

impl ViewBuilder for () {
    fn view(&self, _env: &Environment) -> impl View {}
}

impl<H, P, V> ViewBuilder for IntoHandler<H, P, V>
where
    H: HandlerFn<P, V>,
    P: 'static,
    V: View,
{
    fn view(&self, env: &Environment) -> impl View {
        self.handle(env)
    }
}

/// A boxed view builder that can store any view-building function.
pub struct AnyViewBuilder(Box<dyn Fn(Environment) -> AnyView>);

impl_debug!(AnyViewBuilder);

impl AnyViewBuilder {
    /// Creates a new boxed view builder from any view builder implementation.
    ///
    /// # Arguments
    /// * `builder` - The builder to box
    pub fn new(builder: impl ViewBuilder + 'static) -> Self {
        Self(Box::new(move |env| builder.view(&env).anyview()))
    }
}

impl ViewBuilder for AnyViewBuilder {
    fn view(&self, env: &Environment) -> impl View {
        (self.0)(env.clone())
    }
}

/// Extension trait for views, adding common styling and configuration methods.
pub trait ViewExt: View + Sized {
    /// Attaches metadata to a view.
    ///
    /// # Arguments
    /// * `metadata` - The metadata to attach
    fn metadata<T>(self, metadata: T) -> Metadata<T> {
        Metadata::new(self, metadata)
    }

    /// Associates a value with this view in the environment.
    ///
    /// # Arguments
    /// * `value` - The value to associate with this view
    fn with<T: 'static>(self, value: T) -> With<Self, T> {
        With::new(self, value)
    }

    /// Sets this view as the content of a navigation view with the specified title.
    ///
    /// # Arguments
    /// * `title` - The title for the navigation view
    fn title(self, title: impl Into<Text>) -> NavigationView {
        NavigationView::new(title, self)
    }

    /// Marks this view as focused when the binding matches the specified value.
    ///
    /// # Arguments
    /// * `value` - Binding to the focused value
    /// * `equals` - The value to compare against for focus
    fn focused<T: 'static + Eq + Clone>(
        self,
        value: &Binding<Option<T>>,
        equals: T,
    ) -> Metadata<Focused> {
        Metadata::new(self, Focused::new(value, equals))
    }

    /// Converts this view to an `AnyView` type-erased container.
    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }

    /// Sets the background of this view.
    ///
    /// # Arguments
    /// * `background` - The background to apply
    fn background(self, background: impl Into<Background>) -> Metadata<Background> {
        Metadata::new(self, background.into())
    }

    /// Sets the foreground color of this view.
    ///
    /// # Arguments
    /// * `color` - The foreground color to apply
    fn foreground(self, color: impl IntoComputed<Color>) -> Metadata<ForegroundColor> {
        Metadata::new(self, ForegroundColor::new(color))
    }

    /// Adds a badge to this view.
    ///
    /// # Arguments
    /// * `value` - The numeric value to display in the badge
    fn badge(self, value: impl IntoComputed<i32>) -> Badge {
        Badge::new(value, self)
    }

    fn width(self, width: f32) -> Frame {
        Frame::new(self).width(width)
    }

    fn height(self, height: f32) -> Frame {
        Frame::new(self).height(height)
    }

    fn min_width(self, width: f32) -> Frame {
        Frame::new(self).min_width(width)
    }

    fn max_width(self, width: f32) -> Frame {
        Frame::new(self).max_width(width)
    }

    fn min_height(self, height: f32) -> Frame {
        Frame::new(self).min_height(height)
    }

    fn max_height(self, height: f32) -> Frame {
        Frame::new(self).max_height(height)
    }

    fn size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).width(width).height(height)
    }

    fn min_size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).min_width(width).min_height(height)
    }

    fn max_size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).max_width(width).max_height(height)
    }

    fn alignment(self, alignment: Alignment) -> Frame {
        Frame::new(self).alignment(alignment)
    }

    /// Adds padding to this view with custom edge insets.
    ///
    /// # Arguments
    /// * `edge` - The edge insets to apply as padding
    fn padding_with(self, edge: EdgeInsets) -> Padding {
        Padding::new(edge, self)
    }

    /// Adds default padding to this view.
    fn padding(self) -> Padding {
        Padding::new(EdgeInsets::default(), self)
    }

    /// Tags this view with a custom tag for identification.
    ///
    /// # Arguments
    /// * `tag` - The tag to associate with this view
    fn tag<T>(self, tag: T) -> TaggedView<T, Self> {
        TaggedView::new(tag, self)
    }
}

impl<V: View + Sized> ViewExt for V {}
