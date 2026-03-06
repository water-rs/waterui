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
use nami::{Binding, Signal, SignalExt as _, signal::IntoComputed};
use waterui_core::IntoSignalF32;
pub use waterui_core::view::*;
use waterui_core::{
    AnyView, Environment, IgnorableMetadata, Retain,
    env::{With, use_env},
    layout::{
        HorizontalAlignment, HorizontalAlignmentGuide, VerticalAlignment,
        VerticalAlignmentGuide, ViewDimensions,
    },
    metadata::MetadataKey,
    plugin::Plugin,
};
use waterui_graphics::color::Color;
use waterui_graphics::filter_view::{
    Blur as GraphicsBlur, Brightness as GraphicsBrightness, Contrast as GraphicsContrast,
    FilterViewExt as GraphicsFilterViewExt, Filtered as GraphicsFiltered, GpuFilter,
    Grayscale as GraphicsGrayscale, HueRotation as GraphicsHueRotation, Invert as GraphicsInvert,
    Saturation as GraphicsSaturation, Sepia as GraphicsSepia, Sharpen as GraphicsSharpen,
    Vignette as GraphicsVignette,
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
    accessibility::{
        self, AccessibilityChildren, AccessibilityHidden, AccessibilityLabel, AccessibilityRole,
        AccessibilityState,
    },
    background::IntoBackground,
    border::Border,
    drag_drop::{DragData, Draggable, DropDestination},
    filter::Opacity,
    gesture::{Gesture, GestureObserver, LongPressGesture, TapGesture},
    interaction::Hittable,
    metadata::{context_menu::ContextMenu, secure::Secure},
    theme,
    view_ext::OnChange,
};
use crate::{
    component::{badge::Badge, focus::Focused},
    prelude::Shadow,
    shape::{ClipShape, Shape},
    style::{Anchor, Offset, Rotation, Scale},
};
#[cfg(feature = "std")]
use waterkit_haptic::{Haptic, Intensity};
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

    /// Adjusts the opacity (transparency) of this view.
    ///
    /// This produces a lightweight `Metadata<Opacity>` that maps directly to
    /// compositor-native operations (e.g. `CALayer.opacity` on Apple, `View.alpha`
    /// on Android, `push_layer()` with alpha on hydrolysis). No offscreen texture
    /// or GPU shader pass is involved.
    ///
    /// # Arguments
    /// * `amount` - The opacity value (0.0 = transparent, 1.0 = opaque). Can be reactive.
    fn opacity(self, amount: impl IntoSignalF32) -> Metadata<Opacity> {
        Metadata::new(self, Opacity::new(amount))
    }

    /// Applies a GPU filter to this view.
    ///
    /// This is the low-level entry for custom filters. Prefer convenience methods
    /// like `.blur()` when possible.
    fn filter<F: GpuFilter>(self, filter: F) -> GraphicsFiltered<Self, F> {
        GraphicsFilterViewExt::filter(self, filter)
    }

    /// Applies a blur filter.
    fn blur<T: IntoSignalF32>(self, radius: T) -> GraphicsFiltered<Self, GraphicsBlur> {
        GraphicsFilterViewExt::blur(self, radius)
    }

    /// Applies a brightness filter.
    fn brightness<T: IntoSignalF32>(self, amount: T) -> GraphicsFiltered<Self, GraphicsBrightness> {
        GraphicsFilterViewExt::brightness(self, amount)
    }

    /// Applies a contrast filter.
    fn contrast<T: IntoSignalF32>(self, amount: T) -> GraphicsFiltered<Self, GraphicsContrast> {
        GraphicsFilterViewExt::contrast(self, amount)
    }

    /// Applies a saturation filter.
    fn saturation<T: IntoSignalF32>(self, amount: T) -> GraphicsFiltered<Self, GraphicsSaturation> {
        GraphicsFilterViewExt::saturation(self, amount)
    }

    /// Applies a grayscale filter.
    fn grayscale<T: IntoSignalF32>(
        self,
        intensity: T,
    ) -> GraphicsFiltered<Self, GraphicsGrayscale> {
        GraphicsFilterViewExt::grayscale(self, intensity)
    }

    /// Applies a hue-rotation filter.
    fn hue_rotation<T: IntoSignalF32>(
        self,
        angle: T,
    ) -> GraphicsFiltered<Self, GraphicsHueRotation> {
        GraphicsFilterViewExt::hue_rotation(self, angle)
    }

    /// Applies an invert filter.
    fn invert(self) -> GraphicsFiltered<Self, GraphicsInvert> {
        GraphicsFilterViewExt::invert(self)
    }

    /// Applies a sepia filter.
    fn sepia<T: IntoSignalF32>(self, intensity: T) -> GraphicsFiltered<Self, GraphicsSepia> {
        GraphicsFilterViewExt::sepia(self, intensity)
    }

    /// Applies a sharpen filter.
    fn sharpen<T: IntoSignalF32>(self, amount: T) -> GraphicsFiltered<Self, GraphicsSharpen> {
        GraphicsFilterViewExt::sharpen(self, amount)
    }

    /// Applies a vignette filter.
    fn vignette<R: IntoSignalF32, S: IntoSignalF32>(
        self,
        radius: R,
        softness: S,
    ) -> GraphicsFiltered<Self, GraphicsVignette> {
        GraphicsFilterViewExt::vignette(self, radius, softness)
    }

    /// Sets the visibility of this view.
    ///
    /// # Arguments
    /// * `visible` - A reactive boolean indicating whether the view should be visible
    fn visable(self, visible: impl IntoComputed<bool>) -> impl View {
        self.opacity(
            visible
                .into_computed()
                .map(|visable| if visable { 1.0 } else { 0.0 }),
        )
    }

    /// Associates  a value with this view in the environment.
    fn with<T: 'static>(self, value: T) -> With<Self, T> {
        With::new(self, value)
    }

    /// Sets this view as the content of a navigation view with the specified title.
    ///
    /// # Arguments
    /// * `title` - The title for the navigation view (any View)
    fn title(self, title: impl View) -> NavigationView {
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
    /// The background view renders behind this view, with this view determining
    /// the layout size. This uses a layout-based approach where the background
    /// fills the bounds and the content renders on top.
    ///
    /// # Arguments
    /// * `background` - Any view or `Material` to render as the background
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Color background
    /// text!("Hello").background(Color::red());
    ///
    /// // Material background (platform backend best-effort)
    /// text!("Hello").background(Material::Regular);
    ///
    /// // Any view as background
    /// let my_gradient_view = stack::hstack((Color::red(), Color::blue()));
    /// text!("Hello").background(my_gradient_view);
    /// ```
    fn background<B: IntoBackground>(self, background: B) -> B::Output<Self> {
        background.apply_background(self)
    }

    /// Sets the foreground color for this view and all its descendants.
    ///
    /// This injects the color into the environment as the `Foreground` color token,
    /// which affects all text and icons in the subtree that don't have an explicit color set.
    ///
    /// # Arguments
    /// * `color` - The foreground color to apply
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // All text in this VStack will be red
    /// vstack((
    ///     text!("Hello"),
    ///     text!("World"),
    /// )).foreground(Color::red());
    /// ```
    fn foreground(self, color: impl Into<Color>) -> impl View {
        self.install(theme::ForegroundOverride::new(color))
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
    fn lifecycle(
        self,
        lifecycle: LifeCycle,
        handler: impl FnOnce() + 'static,
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
    fn on_disappear(self, handler: impl FnOnce() + 'static) -> Metadata<LifeCycleHook> {
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
    fn on_appear(self, handler: impl FnOnce() + 'static) -> Metadata<LifeCycleHook> {
        self.lifecycle(LifeCycle::Appear, handler)
    }

    /// Adds an event handler for the specified interaction event.
    ///
    /// You may want to use `ViewExt::on_hover_enter` or `ViewExt::on_hover_exit` for convenience.
    ///
    /// # Arguments
    /// * `event` - The event to listen for
    /// * `handler` - The action to execute when the event occurs (can be called multiple times)
    fn event(self, event: Event, handler: impl FnMut() + 'static) -> Metadata<OnEvent> {
        Metadata::new(self, OnEvent::new(event, handler))
    }

    /// Adds a handler that triggers when the cursor enters this view's bounds.
    ///
    /// This event can fire multiple times as the cursor moves in and out of the view.
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Arguments
    /// * `handler` - The action to execute when hover starts
    fn on_hover_enter(self, handler: impl FnMut() + 'static) -> Metadata<OnEvent> {
        self.event(Event::HoverEnter, handler)
    }

    /// Adds a handler that triggers when the cursor exits this view's bounds.
    ///
    /// This event can fire multiple times as the cursor moves in and out of the view.
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Arguments
    /// * `handler` - The action to execute when hover ends
    fn on_hover_exit(self, handler: impl FnMut() + 'static) -> Metadata<OnEvent> {
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

    /// Overrides a horizontal alignment guide for this view.
    fn horizontal_alignment_guide(
        self,
        alignment: HorizontalAlignment,
        compute: impl Fn(&ViewDimensions) -> f32 + 'static,
    ) -> IgnorableMetadata<HorizontalAlignmentGuide> {
        IgnorableMetadata::new(self, HorizontalAlignmentGuide::new(alignment, compute))
    }

    /// Overrides a vertical alignment guide for this view.
    fn vertical_alignment_guide(
        self,
        alignment: VerticalAlignment,
        compute: impl Fn(&ViewDimensions) -> f32 + 'static,
    ) -> IgnorableMetadata<VerticalAlignmentGuide> {
        IgnorableMetadata::new(self, VerticalAlignmentGuide::new(alignment, compute))
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

    /// Overrides whether this view is hidden from assistive technologies.
    fn a11y_hidden(self, hidden: bool) -> IgnorableMetadata<AccessibilityHidden> {
        IgnorableMetadata::new(self, accessibility::AccessibilityHidden::new(hidden))
    }

    /// Controls how descendants contribute semantics for this view.
    fn a11y_children(
        self,
        behavior: accessibility::AccessibilityChildren,
    ) -> IgnorableMetadata<AccessibilityChildren> {
        IgnorableMetadata::new(self, behavior)
    }

    /// Applies explicit accessibility state metadata.
    fn a11y_state(
        self,
        state: accessibility::AccessibilityState,
    ) -> IgnorableMetadata<AccessibilityState> {
        IgnorableMetadata::new(self, state)
    }

    /// Observes a gesture and executes an action when the gesture is recognized.
    ///
    /// # Arguments
    /// * `gesture` - The gesture to observe
    /// * `action` - The action to execute when the gesture is recognized
    fn gesture(
        self,
        gesture: impl Into<Gesture>,
        action: impl FnMut() + 'static,
    ) -> Metadata<GestureObserver> {
        Metadata::new(self, GestureObserver::new(gesture).action(action))
    }

    /// Attaches a pre-built gesture observer to this view.
    ///
    /// Use this method when you need the builder pattern for state capture.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::gesture::{GestureObserver, TapGesture};
    ///
    /// view.gesture_observer(
    ///     GestureObserver::new(TapGesture::repeat(2))
    ///         .with_state(&counter)
    ///         .action(|counter| counter.set(counter.get() + 1))
    /// )
    /// ```
    fn gesture_observer(self, observer: GestureObserver) -> Metadata<GestureObserver> {
        Metadata::new(self, observer)
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
    fn on_tap(self, action: impl FnMut() + 'static) -> Metadata<GestureObserver> {
        self.gesture(TapGesture::new(), action)
    }

    /// Adds a tap gesture recognizer to this view (SwiftUI naming style).
    ///
    /// Equivalent to [`ViewExt::on_tap`].
    fn on_tap_gesture(self, action: impl FnMut() + 'static) -> Metadata<GestureObserver> {
        self.on_tap(action)
    }

    /// Adds a tap gesture recognizer requiring an exact tap count.
    fn on_tap_gesture_count(
        self,
        count: u32,
        action: impl FnMut() + 'static,
    ) -> Metadata<GestureObserver> {
        self.gesture(TapGesture::repeat(count.max(1)), action)
    }

    /// Adds a long-press gesture recognizer to this view.
    ///
    /// `minimum_duration_ms` is expressed in milliseconds.
    fn on_long_press_gesture(
        self,
        minimum_duration_ms: u32,
        action: impl FnMut() + 'static,
    ) -> Metadata<GestureObserver> {
        self.gesture(LongPressGesture::new(minimum_duration_ms), action)
    }

    /// Adds a gesture intended to recognize alongside existing gestures.
    ///
    /// This follows SwiftUI naming and currently maps to `gesture(...)`.
    fn simultaneous_gesture(
        self,
        gesture: impl Into<Gesture>,
        action: impl FnMut() + 'static,
    ) -> Metadata<GestureObserver> {
        self.gesture(gesture, action)
    }

    /// Adds a gesture intended to have higher recognition precedence.
    ///
    /// This follows SwiftUI naming and currently maps to `gesture(...)`.
    fn high_priority_gesture(
        self,
        gesture: impl Into<Gesture>,
        action: impl FnMut() + 'static,
    ) -> Metadata<GestureObserver> {
        self.gesture(gesture, action)
    }

    /// Adds a tap gesture recognizer and triggers haptic impact feedback.
    #[cfg(feature = "std")]
    #[must_use]
    fn on_tap_haptic(
        self,
        intensity: Intensity,
        mut action: impl FnMut() + 'static,
    ) -> Metadata<GestureObserver> {
        self.gesture(TapGesture::new(), move || {
            spawn_local(async move {
                let _ = Haptic::impact(intensity).await;
            })
            .detach();
            action();
        })
    }

    /// Adds a tap gesture recognizer with medium haptic impact feedback.
    #[cfg(feature = "std")]
    #[must_use]
    fn on_tap_haptic_default(self, action: impl FnMut() + 'static) -> Metadata<GestureObserver> {
        self.on_tap_haptic(Intensity::MEDIUM, action)
    }

    /// Applies a shadow effect to this view.
    fn shadow(self, shadow: impl Into<Shadow>) -> Metadata<Shadow> {
        Metadata::new(self, shadow.into())
    }

    /// Applies a border around this view.
    ///
    /// Creates a border with the specified color and width on all edges
    /// with square corners.
    ///
    /// For rounded corners or edge-specific borders, use [`border_with`](ViewExt::border_with)
    /// with a configured [`Border`] instance.
    ///
    /// # Arguments
    /// * `color` - The border color
    /// * `width` - The border width in points
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Simple border on all edges
    /// text!("Hello").border(Color::red(), 2.0);
    /// ```
    fn border(self, color: impl Into<Color>, width: f32) -> Metadata<Border> {
        Metadata::new(self, Border::new(color, width))
    }

    /// Applies a border with full customization.
    ///
    /// Use this method when you need to configure all border properties at once.
    ///
    /// # Arguments
    /// * `border` - A fully configured `Border` instance
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::border::Border;
    ///
    /// let border = Border::new(Color::blue(), 2.0)
    ///     .corner_radius(12.0)
    ///     .edges(EdgeSet::HORIZONTAL);
    ///
    /// text!("Custom").border_with(border);
    /// ```
    fn border_with(self, border: Border) -> Metadata<Border> {
        Metadata::new(self, border)
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
    /// // Scale uniformly to 150%
    /// view.scale(1.5, 1.5);
    ///
    /// // Scale X only (stretch horizontally)
    /// view.scale(2.0, 1.0);
    ///
    /// // Animate scale
    /// let x = binding(1.0).animated();
    /// let y = binding(1.0).animated();
    /// view.scale(x, y);
    /// ```
    fn scale(self, x: impl IntoSignalF32, y: impl IntoSignalF32) -> Metadata<Scale> {
        Metadata::new(self, Scale::xy(x, y))
    }

    /// Applies a scale transform around a specific anchor point.
    ///
    /// # Arguments
    /// * `x` - The horizontal scale factor
    /// * `y` - The vertical scale factor
    /// * `anchor` - The anchor point for the scale (e.g., `Anchor::TOP_LEFT`)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::style::Anchor;
    ///
    /// // Scale from top-left corner
    /// view.scale_from(0.5, 0.5, Anchor::TOP_LEFT);
    /// ```
    fn scale_from(
        self,
        x: impl IntoSignalF32,
        y: impl IntoSignalF32,
        anchor: Anchor,
    ) -> Metadata<Scale> {
        Metadata::new(self, Scale::xy_from(x, y, anchor))
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
    /// let angle = binding(0.0).animated();
    /// view.rotation(angle);
    /// ```
    fn rotation(self, degrees: impl IntoSignalF32) -> Metadata<Rotation> {
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
    fn rotation_from(self, degrees: impl IntoSignalF32, anchor: Anchor) -> Metadata<Rotation> {
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
    /// let x = binding(0.0).animated();
    /// view.offset(x, 0.0);
    /// ```
    fn offset(self, x: impl IntoSignalF32, y: impl IntoSignalF32) -> Metadata<Offset> {
        Metadata::new(self, Offset::new(x, y))
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
    ///         MenuItem::new("Copy").action(|| println!("Copy")),
    ///         MenuItem::new("Paste").action(|| println!("Paste")),
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

    /// Makes this view draggable with the specified data.
    ///
    /// When the user drags this view (click-drag on macOS, long-press-drag on iOS/Android),
    /// the data will be transferred to any compatible drop destination.
    ///
    /// # Arguments
    /// * `data` - The data to transfer when dragging (can be reactive)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::drag_drop::DragData;
    ///
    /// text!("Drag me")
    ///     .draggable(DragData::text("Hello!"));
    /// ```
    fn draggable(self, data: impl IntoComputed<DragData>) -> Metadata<Draggable> {
        Metadata::new(self, Draggable::new(data))
    }

    /// Makes this view a drop destination for dragged content.
    ///
    /// For simple cases without state, pass a handler directly. For stateful
    /// handlers, use `.with_state(&x).drop_destination(|state, data| ...)`.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    /// use waterui::drag_drop::DragData;
    ///
    /// // Simple usage without state
    /// text!("Drop here")
    ///     .drop_destination(|data: DragData| {
    ///         println!("Received: {:?}", data);
    ///     });
    ///
    /// // With state - use .with_state() first
    /// text!("Drop here")
    ///     .with_state(&items)
    ///     .with_state(&count)
    ///     .drop_destination(|(items, count), data| {
    ///         items.update(|v| v.push(data.as_str().to_string()));
    ///         count.set(count.get() + 1);
    ///     });
    /// ```
    fn drop_destination(
        self,
        on_drop: impl FnMut(DragData) + 'static,
    ) -> Metadata<DropDestination> {
        Metadata::new(self, DropDestination::new(on_drop))
    }

    /// Controls whether this view responds to hit testing (touch/click events).
    ///
    /// When `enabled` is false, touch events pass through the view to views behind it.
    /// This modifier does NOT affect the visual appearance of the view.
    ///
    /// # Arguments
    /// * `enabled` - Whether hit testing is enabled (can be reactive)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Make a view transparent to touch events
    /// text("Click through me")
    ///     .hittable(false);
    ///
    /// // Reactive hit testing control
    /// let can_interact = binding(true);
    /// Button::new("Click me", || {})
    ///     .hittable(can_interact);
    /// ```
    fn hittable(self, enabled: impl IntoComputed<bool>) -> Metadata<Hittable> {
        Metadata::new(self, Hittable::new(enabled))
    }

    /// Disables this view - grays it out and blocks all interactions.
    ///
    /// This is a convenience modifier that composes `opacity(0.5)` + `hittable(false)`.
    /// When disabled, the view becomes semi-transparent and ignores all touch events.
    ///
    /// # Arguments
    /// * `is_disabled` - Whether the view is disabled (can be reactive)
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// use waterui::prelude::*;
    ///
    /// // Disable a button
    /// Button::new("Submit", || {})
    ///     .disabled(true);
    ///
    /// // Reactive disable based on form validity
    /// let is_submitting = binding(false);
    /// vstack((
    ///     TextField::new("Name", name),
    ///     Button::new("Submit", || {}),
    /// )).disabled(is_submitting);
    /// ```
    fn disabled(self, is_disabled: impl IntoComputed<bool>) -> impl View {
        let is_disabled = is_disabled.into_computed();

        // Compose: opacity + hit testing
        // opacity: 0.5 when disabled, 1.0 when enabled
        // hittable: false when disabled, true when enabled
        let opacity_value = is_disabled.clone().map(|d| if d { 0.5 } else { 1.0 });
        let hittable_value = is_disabled.map(|d| !d);

        self.opacity(opacity_value).hittable(hittable_value)
    }

    /// Starts building a view with captured state for event handlers.
    ///
    /// The state value is cloned when this method is called. Chain multiple
    /// `with_state` calls to capture multiple values - they accumulate as
    /// nested tuples: `(first, second)`, then `((first, second), third)`.
    ///
    /// After capturing state, use event handlers like `on_hover_enter`,
    /// `on_tap`, etc. which will receive the accumulated state.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// text("Hover Me!")
    ///     .with_state(&hover_count)
    ///     .with_state(&is_hovered)
    ///     .on_hover_enter(|(count, hovered)| {
    ///         *count.get_mut() += 1;
    ///         hovered.set(true);
    ///     })
    ///     .on_hover_exit(|(_, hovered)| {
    ///         hovered.set(false);
    ///     })
    /// ```
    fn with_state<T: Clone + 'static>(self, state: &T) -> StatefulView<Self, T> {
        StatefulView {
            view: self,
            state: state.clone(),
        }
    }
}

impl<V: View + Sized> ViewExt for V {}

// ============================================================================
// StatefulView - View wrapper with accumulated state for event handlers
// ============================================================================

/// A view wrapper that accumulates state for event handlers.
///
/// Created by calling [`ViewExt::with_state`]. Chain multiple `with_state`
/// calls to accumulate state as nested tuples, then use event handlers
/// like `on_hover_enter` which receive the accumulated state.
///
/// # State Accumulation
///
/// Each `with_state` call wraps the previous state in a tuple:
/// - First call: `S`
/// - Second call: `(S, T)`
/// - Third call: `((S, T), U)`
///
/// # Example
///
/// ```rust,ignore
/// text("Hover Me!")
///     .padding()
///     .background(color)
///     // State accumulation starts here
///     .with_state(&hover_count)
///     .with_state(&is_hovered)
///     // Handlers receive accumulated state
///     .on_hover_enter(|(count, hovered)| {
///         *count.get_mut() += 1;
///         hovered.set(true);
///     })
///     .on_hover_exit(|(_, hovered)| {
///         hovered.set(false);
///     })
/// ```
#[derive(Debug, Clone)]
pub struct StatefulView<V, S> {
    view: V,
    state: S,
}

impl<V: View, S: 'static> View for StatefulView<V, S> {
    fn body(self, _env: &Environment) -> impl View {
        self.view
    }
}

impl<V: View, S: Clone + 'static> StatefulView<V, S> {
    /// Adds another state value to the builder.
    ///
    /// The state accumulates as nested tuples: `(previous, new)`.
    #[must_use]
    pub fn with_state<T: Clone + 'static>(self, state: &T) -> StatefulView<V, (S, T)> {
        StatefulView {
            view: self.view,
            state: (self.state, state.clone()),
        }
    }

    /// Adds a handler that triggers when the cursor enters this view's bounds.
    ///
    /// The handler receives the accumulated state.
    #[must_use]
    pub fn on_hover_enter(
        self,
        mut handler: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<OnEvent>, S> {
        let state = self.state.clone();
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                OnEvent::new(Event::HoverEnter, move || {
                    handler(state_for_handler.clone());
                }),
            ),
            state,
        }
    }

    /// Adds a handler that triggers when the cursor exits this view's bounds.
    ///
    /// The handler receives the accumulated state.
    #[must_use]
    pub fn on_hover_exit(
        self,
        mut handler: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<OnEvent>, S> {
        let state = self.state.clone();
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                OnEvent::new(Event::HoverExit, move || handler(state_for_handler.clone())),
            ),
            state,
        }
    }

    /// Adds a handler that triggers when the view appears.
    ///
    /// The handler receives the accumulated state (moved, not cloned).
    #[must_use]
    pub fn on_appear(
        self,
        handler: impl FnOnce(S) + 'static,
    ) -> StatefulView<Metadata<LifeCycleHook>, ()> {
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                LifeCycleHook::new(LifeCycle::Appear, move || handler(state_for_handler)),
            ),
            state: (),
        }
    }

    /// Adds a handler that triggers when the view disappears.
    ///
    /// The handler receives the accumulated state (moved, not cloned).
    #[must_use]
    pub fn on_disappear(
        self,
        handler: impl FnOnce(S) + 'static,
    ) -> StatefulView<Metadata<LifeCycleHook>, ()> {
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                LifeCycleHook::new(LifeCycle::Disappear, move || handler(state_for_handler)),
            ),
            state: (),
        }
    }

    /// Adds a tap gesture recognizer that receives the accumulated state.
    #[must_use]
    pub fn on_tap(
        self,
        mut action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        let state = self.state.clone();
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                GestureObserver::new(TapGesture::new())
                    .action(move || action(state_for_handler.clone())),
            ),
            state,
        }
    }

    /// Adds a tap gesture recognizer that receives the accumulated state (SwiftUI naming style).
    #[must_use]
    pub fn on_tap_gesture(
        self,
        action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        self.on_tap(action)
    }

    /// Adds a tap gesture recognizer requiring an exact tap count.
    #[must_use]
    pub fn on_tap_gesture_count(
        self,
        count: u32,
        mut action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        let state = self.state.clone();
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                GestureObserver::new(TapGesture::repeat(count.max(1)))
                    .action(move || action(state_for_handler.clone())),
            ),
            state,
        }
    }

    /// Adds a long-press gesture recognizer that receives the accumulated state.
    #[must_use]
    pub fn on_long_press_gesture(
        self,
        minimum_duration_ms: u32,
        mut action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        let state = self.state.clone();
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                GestureObserver::new(LongPressGesture::new(minimum_duration_ms))
                    .action(move || action(state_for_handler.clone())),
            ),
            state,
        }
    }

    /// Observes a gesture with the accumulated state.
    #[must_use]
    pub fn gesture(
        self,
        gesture: impl Into<Gesture>,
        mut action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        let state = self.state.clone();
        let state_for_handler = self.state;
        StatefulView {
            view: Metadata::new(
                self.view,
                GestureObserver::new(gesture).action(move || action(state_for_handler.clone())),
            ),
            state,
        }
    }

    /// Adds a gesture intended to recognize alongside existing gestures.
    #[must_use]
    pub fn simultaneous_gesture(
        self,
        gesture: impl Into<Gesture>,
        action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        self.gesture(gesture, action)
    }

    /// Adds a gesture intended to have higher recognition precedence.
    #[must_use]
    pub fn high_priority_gesture(
        self,
        gesture: impl Into<Gesture>,
        action: impl FnMut(S) + 'static,
    ) -> StatefulView<Metadata<GestureObserver>, S> {
        self.gesture(gesture, action)
    }

    /// Makes this view a drop destination with the accumulated state.
    ///
    /// The handler receives both the accumulated state and the dropped data.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// view
    ///     .with_state(&collected)
    ///     .with_state(&bounce)
    ///     .drop_destination(|(collected, bounce), data| {
    ///         collected.update(|v| v.push(data.as_str().to_string()));
    ///         bounce.set(1.2);
    ///     })
    ///     .drop_hover(&is_hovering)
    /// ```
    #[must_use]
    pub fn drop_destination(
        self,
        mut handler: impl FnMut(S, DragData) + 'static,
    ) -> Metadata<DropDestination> {
        let state = self.state;
        Metadata::new(
            self.view,
            DropDestination::new(move |data| handler(state.clone(), data)),
        )
    }
}
