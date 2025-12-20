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

use alloc::vec::Vec;
use executor_core::spawn_local;
use nami::{Binding, Computed, Signal, signal::IntoComputed};
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
    filter,
    gesture::{Gesture, GestureObserver, TapGesture},
    metadata::{context_menu::ContextMenu, secure::Secure},
    view_ext::OnChange,
};
use crate::{
    component::{Text, badge::Badge, focus::Focused},
    prelude::Shadow,
    shape::{ClipShape, Shape},
    style::{Anchor, Offset, Rotation, Scale, Transform},
};
use waterui_core::Metadata;
use waterui_core::event::{Event, LifeCycle, LifeCycleHook, OnEvent};
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

    /// Spawns an asynchronous task tied to the lifecycle of this view.
    ///
    /// The task will be cancelled when the view is dropped.
    ///
    /// # Arguments
    /// * `task` - The asynchronous task to run
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
    /// text!("Hello").background(Color::red());
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
    /// text("Hello").overlay(Color::red().with_opacity(0.5));
    /// ```
    fn overlay<V>(self, overlay: V) -> Overlay<Self, V> {
        Overlay::new(self, overlay)
    }

    /// Adds a lifecycle hook for the specified lifecycle event.
    ///
    /// You may want to use `ViewExt::on_appear` or `ViewExt::on_disappear` for convenience.
    ///
    /// # Arguments
    /// * `lifecycle` - The lifecycle event to listen for
    /// * `handler` - The action to execute when the event occurs (called once)
    fn lifecycle<H: 'static>(
        self,
        lifecycle: LifeCycle,
        handler: impl HandlerFnOnce<H, ()> + 'static,
    ) -> Metadata<LifeCycleHook> {
        Metadata::new(self, LifeCycleHook::new(lifecycle, handler))
    }

    /// Adds a handler that triggers when the view disappears.
    ///
    /// Warning: This handler will be called when the view is removed from the view hierarchy,
    /// not when the view is hidden. Also, removed from the view hierarchy does not mean the view is destroyed,
    /// if you want to release resources when the view is destroyed, consider to use [`ViewExt::retain`] to keep the view alive.
    ///
    /// # Arguments
    /// * `handler` - The action to execute when the view disappears
    fn on_disappear<H: 'static>(
        self,
        handler: impl HandlerFnOnce<H, ()> + 'static,
    ) -> Metadata<LifeCycleHook> {
        self.lifecycle(LifeCycle::Disappear, handler)
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
    /// let count:Binding<i32> = binding(0);
    /// text("Hello").on_appear(|| println!("Hello, World!"));
    /// ```
    ///
    /// # Arguments
    /// * `handler` - The action to execute when the view appears
    fn on_appear<H: 'static>(
        self,
        handler: impl HandlerFnOnce<H, ()> + 'static,
    ) -> Metadata<LifeCycleHook> {
        self.lifecycle(LifeCycle::Appear, handler)
    }

    /// Adds an event handler for the specified interaction event.
    ///
    /// You may want to use `ViewExt::on_hover_enter` or `ViewExt::on_hover_exit` for convenience.
    ///
    /// # Arguments
    /// * `event` - The event to listen for
    /// * `handler` - The action to execute when the event occurs (can be called multiple times)
    fn event<H: 'static>(
        self,
        event: Event,
        handler: impl HandlerFn<H, ()> + 'static,
    ) -> Metadata<OnEvent> {
        Metadata::new(self, OnEvent::new(event, handler))
    }

    /// Adds a handler that triggers when the cursor enters this view's bounds.
    ///
    /// This event can fire multiple times as the cursor moves in and out of the view.
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Arguments
    /// * `handler` - The action to execute when hover starts
    fn on_hover_enter<H: 'static>(
        self,
        handler: impl HandlerFn<H, ()> + 'static,
    ) -> Metadata<OnEvent> {
        self.event(Event::HoverEnter, handler)
    }

    /// Adds a handler that triggers when the cursor exits this view's bounds.
    ///
    /// This event can fire multiple times as the cursor moves in and out of the view.
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Arguments
    /// * `handler` - The action to execute when hover ends
    fn on_hover_exit<H: 'static>(
        self,
        handler: impl HandlerFn<H, ()> + 'static,
    ) -> Metadata<OnEvent> {
        self.event(Event::HoverExit, handler)
    }

    /// Sets the cursor style when hovering over this view.
    ///
    /// The cursor style is scoped to the view's bounds - when the cursor exits
    /// the view, the cursor automatically reverts to the parent view's cursor
    /// or the system default.
    ///
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::cursor::CursorStyle;
    ///
    /// text!("Click me").cursor(CursorStyle::PointingHand)
    /// ```
    ///
    /// # Arguments
    /// * `style` - The cursor style to display (can be reactive)
    fn cursor(
        self,
        style: impl IntoComputed<crate::cursor::CursorStyle>,
    ) -> Metadata<crate::cursor::Cursor> {
        Metadata::new(self, crate::cursor::Cursor::new(style))
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

    /// Applies a 2D transform to this view.
    ///
    /// Transforms are purely visual and do not affect layout calculations.
    /// They are applied after layout, making them ideal for animations.
    ///
    /// # Arguments
    /// * `transform` - The transform to apply (scale, rotation, translation)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::style::Transform;
    ///
    /// // Scale and rotate a colored box
    /// Color::red()
    ///     .width(100.0)
    ///     .height(100.0)
    ///     .transform(Transform::scale(1.5).with_rotation(45.0));
    ///
    /// // Animate a transform
    /// let scale = binding(1.0).animated();
    /// Color::blue()
    ///     .width(80.0)
    ///     .height(80.0)
    ///     .transform(Transform::scale(scale));
    /// ```
    fn transform(self, transform: Transform) -> Metadata<Transform> {
        Metadata::new(self, transform)
    }

    /// Applies a uniform scale transform to this view around its center.
    ///
    /// Scales are purely visual and do not affect layout calculations.
    ///
    /// # Arguments
    /// * `factor` - The scale factor (1.0 = no scale, 0.5 = half size, 2.0 = double size)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Scale a view to 150%
    /// Color::red()
    ///     .width(100.0)
    ///     .height(100.0)
    ///     .scale(1.5);
    ///
    /// // Animate scale
    /// let factor = binding(1.0_f32).animated();
    /// Color::blue()
    ///     .width(80.0)
    ///     .height(80.0)
    ///     .scale(factor);
    /// ```
    fn scale(self, factor: impl IntoComputed<f32>) -> Metadata<Scale> {
        Metadata::new(self, Scale::uniform(factor))
    }

    /// Applies a uniform scale transform to this view around a specific anchor point.
    ///
    /// # Arguments
    /// * `factor` - The scale factor
    /// * `anchor` - The anchor point for the scale (e.g., `Anchor::TOP_LEFT`)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::style::Anchor;
    ///
    /// // Scale from top-left corner
    /// view.scale_from(0.5, Anchor::TOP_LEFT);
    /// ```
    fn scale_from(self, factor: impl IntoComputed<f32>, anchor: Anchor) -> Metadata<Scale> {
        Metadata::new(self, Scale::uniform_from(factor, anchor))
    }

    /// Applies a rotation transform to this view around its center.
    ///
    /// Rotations are purely visual and do not affect layout calculations.
    ///
    /// # Arguments
    /// * `degrees` - The rotation angle in degrees (positive = clockwise)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Rotate 45 degrees
    /// view.rotation(45.0);
    ///
    /// // Animate rotation
    /// let angle = binding(0.0_f32).animated();
    /// view.rotation(angle);
    /// ```
    fn rotation(self, degrees: impl IntoComputed<f32>) -> Metadata<Rotation> {
        Metadata::new(self, Rotation::degrees(degrees))
    }

    /// Applies a rotation transform to this view around a specific anchor point.
    ///
    /// # Arguments
    /// * `degrees` - The rotation angle in degrees
    /// * `anchor` - The anchor point for the rotation
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::style::Anchor;
    ///
    /// // Rotate around top-left corner
    /// view.rotation_from(45.0, Anchor::TOP_LEFT);
    /// ```
    fn rotation_from(self, degrees: impl IntoComputed<f32>, anchor: Anchor) -> Metadata<Rotation> {
        Metadata::new(self, Rotation::degrees_from(degrees, anchor))
    }

    /// Applies an offset (translation) transform to this view.
    ///
    /// Offsets are purely visual and do not affect layout calculations.
    ///
    /// # Arguments
    /// * `x` - The offset along the X axis in points
    /// * `y` - The offset along the Y axis in points
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Move view by (10, 20) points
    /// view.offset(10.0, 20.0);
    ///
    /// // Animate offset
    /// let x = binding(0.0_f32).animated();
    /// view.offset(x, 0.0);
    /// ```
    fn offset(self, x: impl IntoComputed<f32>, y: impl IntoComputed<f32>) -> Metadata<Offset> {
        Metadata::new(self, Offset::new(x, y))
    }

    /// Applies a blur filter to this view.
    ///
    /// Blur is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `radius` - The blur radius in points (0 = no blur)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Apply 10pt blur
    /// view.blur(10.0);
    ///
    /// // Animate blur
    /// let radius = binding(0.0_f32).animated();
    /// view.blur(radius);
    /// ```
    fn blur(self, radius: impl IntoComputed<f32>) -> Metadata<filter::Blur> {
        Metadata::new(self, filter::Blur::new(radius))
    }

    /// Applies a brightness adjustment to this view.
    ///
    /// Brightness is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `amount` - Brightness adjustment (0 = no change, negative = darker, positive = brighter)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Darken view
    /// view.brightness(-0.3);
    ///
    /// // Animate brightness
    /// let amount = binding(0.0_f32).animated();
    /// view.brightness(amount);
    /// ```
    fn brightness(self, amount: impl IntoComputed<f32>) -> Metadata<filter::Brightness> {
        Metadata::new(self, filter::Brightness::new(amount))
    }

    /// Applies a saturation adjustment to this view.
    ///
    /// Saturation is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `amount` - Saturation amount (0 = grayscale, 1 = normal, >1 = oversaturated)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Desaturate to 50%
    /// view.saturation(0.5);
    ///
    /// // Animate saturation
    /// let amount = binding(1.0_f32).animated();
    /// view.saturation(amount);
    /// ```
    fn saturation(self, amount: impl IntoComputed<f32>) -> Metadata<filter::Saturation> {
        Metadata::new(self, filter::Saturation::new(amount))
    }

    /// Applies a contrast adjustment to this view.
    ///
    /// Contrast is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `amount` - Contrast amount (1 = normal, <1 = less contrast, >1 = more contrast)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Increase contrast
    /// view.contrast(1.5);
    ///
    /// // Animate contrast
    /// let amount = binding(1.0_f32).animated();
    /// view.contrast(amount);
    /// ```
    fn contrast(self, amount: impl IntoComputed<f32>) -> Metadata<filter::Contrast> {
        Metadata::new(self, filter::Contrast::new(amount))
    }

    /// Applies a hue rotation to this view.
    ///
    /// Hue rotation is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `degrees` - The angle of hue rotation in degrees (0-360)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Rotate hue by 180 degrees (invert colors)
    /// view.hue_rotation(180.0);
    ///
    /// // Animate hue rotation
    /// let angle = binding(0.0_f32).animated();
    /// view.hue_rotation(angle);
    /// ```
    fn hue_rotation(self, degrees: impl IntoComputed<f32>) -> Metadata<filter::HueRotation> {
        Metadata::new(self, filter::HueRotation::new(degrees))
    }

    /// Applies a grayscale filter to this view.
    ///
    /// Grayscale is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `intensity` - Grayscale intensity (0 = full color, 1 = full grayscale)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Convert to full grayscale
    /// view.grayscale(1.0);
    ///
    /// // Animate grayscale
    /// let intensity = binding(0.0_f32).animated();
    /// view.grayscale(intensity);
    /// ```
    fn grayscale(self, intensity: impl IntoComputed<f32>) -> Metadata<filter::Grayscale> {
        Metadata::new(self, filter::Grayscale::new(intensity))
    }

    /// Applies an opacity adjustment to this view.
    ///
    /// Opacity is purely visual and does not affect layout calculations.
    ///
    /// # Arguments
    /// * `value` - Opacity value (0 = transparent, 1 = opaque)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Make view 50% transparent
    /// view.opacity(0.5);
    ///
    /// // Animate opacity
    /// let value = binding(1.0_f32).animated();
    /// view.opacity(value);
    /// ```
    fn opacity(self, value: impl IntoComputed<f32>) -> Metadata<filter::Opacity> {
        Metadata::new(self, filter::Opacity::new(value))
    }

    /// Clips this view to the specified shape.
    ///
    /// The shape defines a mask - only the portion of the view inside the shape
    /// will be visible. Coordinates in the shape are normalized (0.0-1.0) and
    /// scale with the view's bounds.
    ///
    /// # Arguments
    /// * `shape` - The shape to clip to (e.g., `Circle`, `RoundedRectangle`, custom `Path`)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::shape::*;
    ///
    /// // Clip image to a circle
    /// image("avatar.jpg").clip(Circle);
    ///
    /// // Clip to rounded rectangle
    /// card.clip(RoundedRectangle::new(0.1));
    ///
    /// // Custom triangle shape
    /// let triangle = Path::new()
    ///     .move_to(0.5, 0.0)
    ///     .line_to(1.0, 1.0)
    ///     .line_to(0.0, 1.0)
    ///     .close();
    /// Color::red().size(100.0, 100.0).clip(triangle);
    /// ```
    fn clip(self, shape: impl Shape) -> Metadata<ClipShape> {
        Metadata::new(self, ClipShape::new(shape))
    }

    /// Attaches a context menu to this view.
    ///
    /// The context menu appears when the user:
    /// - Long-presses on iOS/Android
    /// - Right-clicks on macOS
    ///
    /// # Arguments
    /// * `items` - The menu items to display
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// text!("Right-click me")
    ///     .context_menu(vec![
    ///         MenuItem::new("Copy", || println!("Copy")),
    ///         MenuItem::new("Paste", || println!("Paste")),
    ///     ]);
    /// ```
    fn context_menu(
        self,
        items: impl IntoComputed<Vec<crate::metadata::context_menu::MenuItem>>,
    ) -> Metadata<ContextMenu> {
        Metadata::new(self, ContextMenu::new(items))
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
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Extend background to fill entire screen
    /// Color::red()
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
    /// fn view() -> impl View{
    ///     let count:Binding<i32> = binding(0);
    ///     let guard = count.clone().watch(|v| println!("Count: {}", v.into_value()));
    ///     text("Hello").retain(guard)
    /// }
    /// ```
    fn retain<T: 'static>(self, value: T) -> Metadata<Retain> {
        Metadata::new(self, Retain::new(value))
    }
}

impl<V: View + Sized> ViewExt for V {}
