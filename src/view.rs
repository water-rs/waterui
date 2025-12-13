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

use executor_core::spawn_local;
use nami::{Binding, Signal, signal::IntoComputed};
use waterui_color::Color;
pub use waterui_core::view::*;
use waterui_core::{
    AnyView, Environment, IgnorableMetadata, Retain,
    env::{With, use_env},
    handler::{HandlerFn, HandlerFnOnce},
    metadata::MetadataKey,
    plugin::Plugin,
};

use waterui_layout::{
    EdgeSet, IgnoreSafeArea, Overlay,
    frame::Frame,
    padding::{EdgeInsets, Padding},
    stack::Alignment,
};
use waterui_navigation::NavigationView;
use waterui_str::Str;

use crate::{
    accessibility::{self, AccessibilityLabel, AccessibilityRole},
    background::{Background, ForegroundColor},
    gesture::{Gesture, GestureObserver, TapGesture},
    metadata::secure::Secure,
    view_ext::OnChange,
};
use crate::{
    component::{Text, badge::Badge, focu::Focused},
    prelude::Shadow,
};
use waterui_core::Metadata;
use waterui_core::event::{Event, OnEvent};
use waterui_core::id::TaggedView;
/// Extension trait for views, adding common styling and configuration methods.
pub trait ViewExt: View + Sized {
    /// Attaches metadata to a view.
    ///
    /// # Arguments
    /// * `metadata` - The metadata to attach
    fn metadata<T: MetadataKey>(self, metadata: T) -> Metadata<T> {
        Metadata::new(self, metadata)
    }

    /// Associates  a value with this view in the environment.
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

    /// Monitors a signal for changes and triggers a handler when the signal's value changes.
    ///
    /// Compare to manual watching, this method automatically manages the watcher lifecycle.
    fn on_change<C, F>(self, source: &C, handler: F) -> OnChange<Self, C::Guard>
    where
        C: Signal,
        C::Output: PartialEq + Clone,
        F: Fn(C::Output) + 'static,
    {
        OnChange::<Self, C::Guard>::new(self, source, handler)
    }

    ///
    fn task<Fut>(self, task: Fut) -> Metadata<Retain>
    where
        Fut: std::future::Future<Output = ()> + 'static,
    {
        let local_task = spawn_local(task);
        self.retain(local_task)
    }

    /// Converts this view to an `AnyView` type-erased container.
    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }

    /// Sets the background of this view.
    ///
    /// # Arguments
    /// * `background` - The background to apply
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// text!("Hello").background(Color::RED);
    /// ```
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

    /// Adds an overlay to this view.
    ///
    /// Unlike `ZStack`, `Overlay` will not affect the size of the base view.
    ///
    /// # Arguments
    /// * `overlay` - The overlay view to add
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// text("Hello").overlay(Color::red().opacity(0.5));
    /// ```
    fn overlay<V>(self, overlay: V) -> Overlay<Self, V> {
        Overlay::new(self, overlay)
    }

    /// Adds an event handler for the specified event.
    ///
    /// You may would like use `ViewExt::on_appear` or `ViewExt::on_disappear` for convenience.
    ///
    /// # Arguments
    /// * `event` - The event to listen for
    /// * `handler` - The action to execute when the event occurs
    fn event<H: 'static>(
        self,
        event: Event,
        handler: impl HandlerFnOnce<H, ()> + 'static,
    ) -> Metadata<OnEvent> {
        Metadata::new(self, OnEvent::new(event, handler))
    }

    /// Adds a handler that triggers when the view disappears.
    ///
    /// Warning: This handler will be called when the view is removed from the view hierarchy,
    /// not when the view is hidden. Also, removed from the view hierarchy does not mean the view is destroyed,
    /// if you want to release resources when the view is destroyed, consider to use [`ViewExt::retain`] to keep the view alive.
    ////
    /// # Arguments
    /// * `handler` - The action to execute when the view disappears
    fn on_disappear<H: 'static>(
        self,
        handler: impl HandlerFnOnce<H, ()> + 'static,
    ) -> Metadata<OnEvent> {
        self.event(Event::Disappear, handler)
    }

    /// Adds a handler that triggers when the view appears.
    ///
    /// In `WaterUI`, a struct that implements `View` trait is a descriptor of a view,
    /// `View` has a `body` method which would be called when the view is rendered.
    /// However, even if `body` is called, the view is not guaranteed to be visible yet.
    /// For instance, a lazy view may resolve bunch of views by calling `body` method,
    /// but delay the actual rendering of the view until it is needed.
    ///
    /// So, if you want to execute some code when the view is visible, you should use this method
    /// to add a handler that triggers when the view appears.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::reactive::binding;
    ///
    /// let count = binding(0);
    /// text("Hello").on_appear(|| println!("Hello, World!"));
    /// ```
    //// # Arguments
    /// * `handler` - The action to execute when the view appears
    fn on_appear<H: 'static>(
        self,
        handler: impl HandlerFnOnce<H, ()> + 'static,
    ) -> Metadata<OnEvent> {
        self.event(Event::Appear, handler)
    }

    /// Adds a badge to this view.
    ///
    /// # Arguments
    /// * `value` - The numeric value to display in the badge
    fn badge(self, value: impl IntoComputed<i32>) -> Badge {
        Badge::new(value, self)
    }

    /// Fixes this view's width to the provided value.
    fn width(self, width: f32) -> Frame {
        Frame::new(self).width(width)
    }

    /// Fixes this view's height to the provided value.
    fn height(self, height: f32) -> Frame {
        Frame::new(self).height(height)
    }

    /// Applies a minimum width constraint.
    fn min_width(self, width: f32) -> Frame {
        Frame::new(self).min_width(width)
    }

    /// Applies a maximum width constraint.
    fn max_width(self, width: f32) -> Frame {
        Frame::new(self).max_width(width)
    }

    /// Applies a minimum height constraint.
    fn min_height(self, height: f32) -> Frame {
        Frame::new(self).min_height(height)
    }

    /// Applies a maximum height constraint.
    fn max_height(self, height: f32) -> Frame {
        Frame::new(self).max_height(height)
    }

    /// Fixes both width and height simultaneously.
    fn size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).width(width).height(height)
    }

    /// Applies minimum constraints on both axes.
    fn min_size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).min_width(width).min_height(height)
    }

    /// Applies maximum constraints on both axes.
    fn max_size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).max_width(width).max_height(height)
    }

    /// Aligns this view within its frame using the provided alignment.
    fn alignment(self, alignment: Alignment) -> Frame {
        Frame::new(self).alignment(alignment)
    }

    /// Adds padding to this view with custom edge insets.
    ///
    /// # Arguments
    /// * `edge` - The edge insets to apply as padding
    fn padding_with(self, edge: impl Into<EdgeInsets>) -> Padding {
        Padding::new(edge.into(), self)
    }

    /// Adds default padding to this view.
    ///
    /// By default, the padding is 14.0 points.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// text!("Hello").padding();
    /// ```
    fn padding(self) -> Padding {
        Padding::new(EdgeInsets::all(14.0), self)
    }

    /// Marks this view as secure.
    ///
    /// User would be forbidden to take a screenshot of the view.
    ///
    /// # Arguments
    /// * `secure` - The secure metadata to apply
    fn secure(self) -> Metadata<Secure> {
        Metadata::new(self, Secure::new())
    }

    /// Tags this view with a custom tag for identification.
    ///
    /// # Arguments
    /// * `tag` - The tag to associate with this view
    fn tag<T>(self, tag: T) -> TaggedView<T, Self> {
        TaggedView::new(tag, self)
    }

    /// Sets the accessibility label for this view.
    ///
    /// # Arguments
    /// * `label` - The accessibility label to apply
    fn a11y_label(self, label: impl Into<Str>) -> IgnorableMetadata<AccessibilityLabel> {
        IgnorableMetadata::new(self, accessibility::AccessibilityLabel::new(label))
    }

    /// Sets the accessibility role for this view.
    ///
    /// # Arguments
    /// * `role` - The accessibility role to apply
    fn a11y_role(
        self,
        role: accessibility::AccessibilityRole,
    ) -> IgnorableMetadata<AccessibilityRole> {
        IgnorableMetadata::new(self, role)
    }

    /// Observes a gesture and executes an action when the gesture is recognized.
    ///
    /// # Arguments
    /// * `gesture` - The gesture to observe
    /// * `action` - The action to execute when the gesture is recognized
    fn gesture<P: 'static>(
        self,
        gesture: impl Into<Gesture>,
        action: impl HandlerFn<P, ()> + 'static,
    ) -> Metadata<GestureObserver> {
        Metadata::new(self, GestureObserver::new(gesture, action))
    }

    /// Adds a tap gesture recognizer to this view that triggers the specified action.
    ///
    /// # Arguments
    /// * `action` - The action to execute when the tap gesture is recognized
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// text!("Click me").on_tap(|| println!("Clicked!"));
    /// ```
    fn on_tap<P: 'static>(
        self,
        action: impl HandlerFn<P, ()> + 'static,
    ) -> Metadata<GestureObserver> {
        self.gesture(TapGesture::new(), action)
    }

    /// Applies a shadow effect to this view.
    fn shadow(self, shadow: impl Into<Shadow>) -> Metadata<Shadow> {
        Metadata::new(self, shadow.into())
    }

    /// Extends this view's bounds to ignore safe area insets on the specified edges.
    ///
    /// This allows backgrounds, images, and other visual elements to extend edge-to-edge
    /// while content remains in the safe area. The native renderer will expand the
    /// view's frame to include the unsafe regions on the specified edges.
    ///
    /// # Arguments
    /// * `edges` - The edges on which to ignore safe area insets
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Extend background to fill entire screen
    /// Color::blue()
    ///     .ignore_safe_area(EdgeSet::ALL);
    ///
    /// // Only extend to top (under status bar)
    /// header_view
    ///     .ignore_safe_area(EdgeSet::TOP);
    /// ```
    fn ignore_safe_area(self, edges: EdgeSet) -> Metadata<IgnoreSafeArea> {
        Metadata::new(self, IgnoreSafeArea::new(edges))
    }

    /// Installs a plugin into the environment.
    fn install(self, plugin: impl Plugin) -> impl View {
        use_env(move |mut env: Environment| {
            plugin.install(&mut env);
            Metadata::new(self, env)
        })
    }

    /// Retains a value for the lifetime of this view.
    ///
    /// This is useful for keeping watcher guards, subscriptions, or other values
    /// alive as long as the view exists. The retained value is dropped when the
    /// view is dropped.
    ///
    /// # Arguments
    /// * `value` - The value to retain (e.g., watcher guard, subscription)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::reactive::binding;
    ///
    /// let count = binding(0);
    /// let guard = count.clone().watch(|v| println!("Count: {}", v.into_value()));
    /// text("Hello").retain(guard)
    /// ```
    fn retain<T: 'static>(self, value: T) -> Metadata<Retain> {
        Metadata::new(self, Retain::new(value))
    }
}

impl<V: View + Sized> ViewExt for V {}
