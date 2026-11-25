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

use crate::animation::{FrameRate, FrameRateVote};
use crate::sheet::Sheet;
#[cfg(feature = "snackbar")]
use crate::snackbar::SnackbarHost;
use core::future::Future;
use nami::{
    Binding, Computed, Signal, SignalExt as _,
    signal::{IntoComputed, IntoSignal},
};
use waterui_core::IntoSignalF32;
pub use waterui_core::view::*;
use waterui_core::{
    Environment, Metadata, Retain,
    env::{Installed, With},
    extract::State,
    handler::{EventHandler, Handler, HandlerOnce},
    layout::{HorizontalAlignment, VerticalAlignment, ViewDimensions},
    metadata::MetadataKey,
    plugin::Plugin,
};
use waterui_graphics::color::Color;

/// All view-level filter modifiers (`.blur()`, `.brightness()`, ...) come
/// from [`waterui_graphics::filter_view::FilterViewExt`]. This re-export
/// makes them part of the `WaterUI` prelude alongside [`ViewExt`], so a single
/// `use waterui::prelude::*;` is enough.
#[cfg(feature = "effects")]
pub use waterui_graphics::filter_view::FilterViewExt;

use suiteki::Str;
#[cfg(feature = "capture")]
use waterui_capture::{CaptureHandle, CaptureScope};
use waterui_layout::{
    AspectRatio, ContentMode, HorizontalAlignmentGuide, IgnoreSafeArea, LayoutPriority, Overlay,
    VerticalAlignmentGuide,
    frame::Frame,
    padding::{EdgeInsets, Padding},
    stack::Alignment,
};
#[cfg(feature = "navigation")]
use waterui_navigation::NavigationView;

#[cfg(feature = "list")]
use crate::component::list::{ListMinRowHeight, ListRowDeletable, ListRowInsets};
#[cfg(feature = "haptics")]
use crate::haptic::Feedback;
use crate::{
    accessibility::{
        self, AccessibilityChildren, AccessibilityGeometry, AccessibilityHidden,
        AccessibilityIdentifier, AccessibilityLabel, AccessibilityRole, AccessibilityState,
        AccessibilityValue,
    },
    background::IntoBackground,
    border::Border,
    drag_drop::{Draggable, DropDestination, Transferable},
    filter::Opacity,
    gesture::{GestureObserver, GestureType, LongPressGesture, TapGesture},
    interaction::Hittable,
    metadata::secure::{DynamicRange, DynamicRangeMetadata, Secure},
    theme,
};
use crate::{
    component::{
        badge::{Badge, BadgeValue},
        focus::Focused,
    },
    floating::Floating,
    prelude::Shadow,
    shape::{ClipShape, Shape},
    style::{Anchor, FloatingStyle, Offset, Rotation, Scale},
};
use waterui_controls::button::{ButtonStyleHook, IntoButtonStyle};
#[cfg(feature = "toggle")]
use waterui_controls::toggle::{IntoToggleStyle, ToggleStyleHook};
use waterui_core::id::TaggedView;
use waterui_interaction::event::{
    Event, HoverEvent, LifeCycle, LifeCycleHook, OnChange, OnEvent, TaskHook,
};
use waterui_interaction::interaction::{Disabled, InteractionReport, InteractionState, Selected};
use waterui_interaction::key::{KeyHandling, KeyPress, OnKeyPress};
use waterui_structural_transition::{Transition, Transitioned};

/// The inset a bare [`ViewExt::padding`] applies.
///
/// It is named because two places need the same number: the modifier, and the
/// TypeScript catalog's `padding={true}`, which is the same spelling from the
/// other side of the seam. A second literal would let the two drift.
pub const DEFAULT_PADDING: f32 = 14.0;

/// Extension trait for views, adding common styling and configuration methods.
pub trait ViewExt: View + Sized {
    /// Attaches metadata to a view.
    ///
    /// # Arguments
    /// * `metadata` - The metadata to attach
    fn metadata<T: MetadataKey>(self, metadata: T) -> Metadata<T, Self> {
        Metadata::new(self, metadata)
    }

    /// Selects the [`DynamicRange`] for this subtree.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// let wide_gamut = text!("Sunset").dynamic_range(DynamicRange::High);
    /// let thumbnail = text!("Avatar").dynamic_range(DynamicRange::Standard);
    /// ```
    fn dynamic_range(
        self,
        range: impl IntoComputed<DynamicRange>,
    ) -> Metadata<DynamicRangeMetadata, Self> {
        Metadata::new(self, DynamicRangeMetadata::new(range))
    }

    /// Animates structural insertion and removal of this view.
    ///
    /// Value updates do not trigger a transition. Applied to a `Dynamic`, this
    /// decorates its replacing content; applied to a collection item, it follows
    /// that item's membership. Removal immediately ends semantic membership,
    /// while the backend retains a non-interactive visual and a fixed-size slot.
    fn transition(self, transition: impl Transition) -> Transitioned<Self> {
        Transitioned::new(self, transition)
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
    fn opacity(self, amount: impl IntoSignalF32) -> Metadata<Opacity, Self> {
        Metadata::new(self, Opacity::new(amount))
    }

    /// Constrains this view to a width-to-height ratio.
    ///
    /// The view shrinks to fit inside whatever it is offered, leaving slack on
    /// the longer axis — `SwiftUI`'s `.aspectRatio(_, contentMode: .fit)`. Use
    /// [`AspectRatio::new`] for the filling mode.
    ///
    /// # Arguments
    /// * `ratio` - Width divided by height. Must be positive and finite.
    fn aspect_ratio(self, ratio: impl IntoSignalF32 + 'static) -> impl View {
        AspectRatio::new(self, ratio, ContentMode::Fit)
    }

    /// Sets how strongly this view holds on to space when its stack runs short.
    ///
    /// A stack compresses its lowest-priority children first, so raising one
    /// child above its siblings keeps it at its ideal size while they give way.
    /// The default is `0`.
    ///
    /// # Arguments
    /// * `priority` - Higher keeps more space; ties share the shortfall.
    fn layout_priority(self, priority: i32) -> Metadata<LayoutPriority, Self> {
        Metadata::new(self, LayoutPriority::new(priority))
    }

    /// Joins the window's refresh-rate vote while this view is mounted and visible.
    ///
    /// Multiple active votes combine field-wise by maximum (`combine`), so an app
    /// vote can raise the window's refresh rate without lowering running animations
    /// below their classified floor, and requests no frames on its own.
    fn frame_rate(
        self,
        rate: impl IntoComputed<Option<FrameRate>>,
    ) -> Metadata<FrameRateVote, Self> {
        Metadata::new(self, FrameRateVote::new(rate))
    }

    /// Sets the visibility of this view.
    ///
    /// # Arguments
    /// * `visible` - A reactive boolean indicating whether the view should be visible
    fn visible(self, visible: impl IntoComputed<bool>) -> impl View {
        let visible = visible.into_computed();
        // A visible scope says nothing about hiding: like the opacity it
        // multiplies, `.visible(true)` never reveals what an enclosing
        // `.visible(false)` hid, so it leaves the flag to the enclosing scope.
        let accessibility_state = visible.map(|visible| {
            if visible {
                AccessibilityState::new()
            } else {
                AccessibilityState::new().hidden(true)
            }
        });
        let opacity_value = visible.map(|visible| if visible { 1.0 } else { 0.0 });
        let hittable_value = visible;

        self.a11y_state(accessibility_state)
            .opacity(opacity_value)
            .hittable(hittable_value)
    }

    /// Associates  a value with this view in the environment.
    fn with<T: 'static>(self, value: T) -> With<Self, T> {
        With::new(self, value)
    }

    /// Offers every native payload in this view to `hooks` before the
    /// backend realizes it: the realization a subtree chooses for a
    /// component, such as a browser engine for `WebView`.
    ///
    /// `hooks` is one [`Hook`](hook::Hook) or a tuple of them; an inner
    /// scope is offered a payload before an outer one.
    fn hooks<H: hook::HookList>(self, hooks: H) -> hook::Hooks<Self, H> {
        hook::Hooks::new(self, hooks)
    }

    /// Styles all [`Button`](crate::component::button::Button) views in this subtree
    /// that did not specify an explicit `.style(...)`.
    ///
    /// Accepts a built-in [`ButtonStyle`](crate::component::ButtonStyle), a
    /// [`CustomButtonStyle`](crate::component::CustomButtonStyle), or a
    /// [`PrimitiveButtonStyle`](crate::component::PrimitiveButtonStyle).
    fn button_style<S, Marker>(
        self,
        style: S,
    ) -> hook::Hooks<Self, ButtonStyleHook<<S as IntoButtonStyle<Marker>>::Style>>
    where
        S: IntoButtonStyle<Marker>,
    {
        hook::Hooks::new(self, ButtonStyleHook::new(style.into_button_style()))
    }

    /// Styles all [`Toggle`](crate::component::toggle::Toggle) views in this subtree
    /// that did not specify an explicit `.style(...)`.
    ///
    /// Accepts a built-in [`ToggleStyle`](crate::component::ToggleStyle) or a
    /// [`CustomToggleStyle`](crate::component::CustomToggleStyle).
    #[cfg(feature = "toggle")]
    fn toggle_style<S, Marker>(
        self,
        style: S,
    ) -> hook::Hooks<Self, ToggleStyleHook<<S as IntoToggleStyle<Marker>>::Style>>
    where
        S: IntoToggleStyle<Marker>,
    {
        hook::Hooks::new(self, ToggleStyleHook::new(style.into_toggle_style()))
    }

    /// Sets this view as the content of a navigation view with the specified title.
    ///
    /// # Arguments
    /// * `title` - The semantic title for the navigation view
    #[cfg(feature = "navigation")]
    fn navigation_title(self, title: impl crate::text::IntoText) -> NavigationView<Self> {
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
    ) -> Metadata<Focused, Self> {
        Metadata::new(self, Focused::new(value, equals))
    }

    /// Runs `handler` whenever `source`'s value changes, for as long as this
    /// view is mounted.
    ///
    /// The handler is an [`EventHandler`]: the new value comes first, then any
    /// extractor arguments — `State<T>`, `Environment`, … — resolved from the
    /// environment this view resolves in, so a [`ViewExt::state`] install
    /// reaches it on either side of `.on_change(..)`. A `Binding` the handler
    /// writes is injected with [`ViewExt::state`] and read back as a
    /// `State<Binding<T>>` parameter rather than captured, so the handler can
    /// be a named function and the data flow stays visible at the call site.
    ///
    /// Compared to watching the signal by hand, the watcher's lifetime is
    /// tied to the view and consecutive equal values are collapsed.
    ///
    /// ```
    /// use waterui::prelude::*;
    ///
    /// let selection = Binding::new(0_i32);
    /// let last_seen = Binding::new(0_i32);
    ///
    /// let view = text!("{selection}")
    ///     .on_change(&selection, |value: i32, State(last_seen): State<Binding<i32>>| {
    ///         last_seen.set(value);
    ///     })
    ///     .state(&last_seen);
    /// ```
    fn on_change<C, H, A>(self, source: &C, handler: H) -> OnChange<Self>
    where
        C: Signal,
        C::Output: PartialEq + Clone,
        H: EventHandler<C::Output, A>,
    {
        OnChange::new(self, source, handler)
    }

    /// Attaches an asynchronous task tied to the mounted lifecycle of this view.
    ///
    /// The handler runs once when the view appears, extracting any parameters
    /// from the view's environment (such as `State<T>` or `#[state]` types),
    /// and the returned future is cancelled when the view disappears. A view
    /// constructed and dropped without ever mounting never invokes `task`.
    ///
    /// # Arguments
    /// * `task` - A one-shot handler returning the future to run while mounted
    fn task<H, Args, Fut>(self, task: H) -> Metadata<TaskHook, Self>
    where
        H: HandlerOnce<Args, Fut>,
        Fut: Future<Output = ()> + 'static,
    {
        Metadata::new(self, TaskHook::new(task))
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
    /// // Material background: a frosted treatment of the content behind the view
    /// text!("Hello").background(Material::Regular);
    ///
    /// // Any view as background
    /// let my_gradient_view = stack::hstack((Color::red(), Color::blue()));
    /// text!("Hello").background(my_gradient_view);
    /// ```
    fn background<B: IntoBackground<M>, M>(self, background: B) -> B::Output<Self> {
        background.apply_background(self)
    }

    /// Groups the backdrop materials in this view's subtree.
    ///
    /// The materials inside share one capture of what lies behind the
    /// group, and the style may merge them into one shape where they come
    /// close, as a toolbar's glass controls do. The style decides whether
    /// and how they merge; the nearest enclosing group wins. See
    /// [`MaterialGroup`](crate::background::MaterialGroup) for the contract.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// hstack((
    ///     text!("Back").padding().background(Glass::regular()),
    ///     text!("Share").padding().background(Glass::regular()),
    /// ))
    /// .material_group();
    /// ```
    fn material_group(self) -> Metadata<crate::background::MaterialGroup, Self> {
        Metadata::new(self, crate::background::MaterialGroup)
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
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // All text in this VStack will be red
    /// vstack((
    ///     text!("Hello"),
    ///     text!("World"),
    /// )).foreground(Color::red());
    /// ```
    fn foreground(self, color: impl IntoSignal<Color> + 'static) -> impl View {
        self.install(theme::ForegroundOverride::new(color))
    }

    /// Renders this view and its subtree in the theme's muted foreground —
    /// `self.foreground(MutedForeground)`, the color captions, secondary
    /// labels and inactive icons share.
    ///
    /// `.foreground(Foreground)` restores the primary foreground below a
    /// muted subtree.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// vstack((
    ///     text!("Sync"),
    ///     text!("Last run: never").caption().muted(),
    /// ));
    /// ```
    fn muted(self) -> impl View {
        self.foreground(theme::color::MutedForeground)
    }

    /// Sets the tint the controls in this view's subtree draw with.
    ///
    /// A control's tint is the nearest enclosing `tint`, and the theme's
    /// `Accent` token when none is set. Buttons, switches, sliders and other
    /// controls draw their emphasis with it. The colour may be a signal; a
    /// change updates the controls in place.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// vstack((
    ///     button("Delete").action(|| {}).tint(Color::red()),
    ///     button("Save").action(|| {}),
    /// ));
    /// ```
    fn tint(self, color: impl IntoSignal<Color> + 'static) -> impl View {
        self.with(theme::Tint::new(color))
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
    fn overlay<V: View>(self, overlay: V) -> Overlay<Self, V> {
        Overlay::new(self, overlay)
    }

    /// Adds a lifecycle hook for the specified lifecycle event.
    ///
    /// You may want to use `ViewExt::on_appear` or `ViewExt::on_disappear` for convenience.
    ///
    /// # Arguments
    /// * `lifecycle` - The lifecycle event to listen for
    /// * `handler` - The action to execute when the event occurs (called once)
    fn lifecycle<H, Args>(self, lifecycle: LifeCycle, handler: H) -> Metadata<LifeCycleHook, Self>
    where
        H: HandlerOnce<Args, ()>,
    {
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
    fn on_disappear<H, Args>(self, handler: H) -> Metadata<LifeCycleHook, Self>
    where
        H: HandlerOnce<Args, ()>,
    {
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
    /// text("Hello").on_appear(|| {});
    /// ```
    ///
    /// # Arguments
    /// * `handler` - The action to execute when the view appears
    fn on_appear<H, Args>(self, handler: H) -> Metadata<LifeCycleHook, Self>
    where
        H: HandlerOnce<Args, ()>,
    {
        self.lifecycle(LifeCycle::Appear, handler)
    }

    /// Adds an event handler for the specified interaction event.
    ///
    /// You may want to use `ViewExt::on_hover_enter`, `ViewExt::on_hover_move`,
    /// or `ViewExt::on_hover_exit` for convenience.
    fn event<H, Args>(self, event: Event, handler: H) -> Metadata<OnEvent, Self>
    where
        H: Handler<Args, ()>,
    {
        Metadata::new(self, OnEvent::new(event, handler))
    }

    /// Adds a handler that triggers when the cursor enters this view's bounds.
    ///
    /// This event can fire multiple times as the cursor moves in and out of the view.
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Arguments
    /// * `handler` - The action to execute when hover starts
    fn on_hover_enter<H, Args>(self, handler: H) -> Metadata<OnEvent, Self>
    where
        H: Handler<Args, ()>,
    {
        Metadata::new(self, OnEvent::hover_enter(handler))
    }

    /// Adds a handler that triggers when the cursor moves inside this view's bounds,
    /// delivering [`HoverEvent`] as the handler's first argument.
    fn on_hover_move<H, Args>(self, handler: H) -> Metadata<OnEvent, Self>
    where
        H: EventHandler<HoverEvent, Args>,
    {
        Metadata::new(self, OnEvent::hover_move(handler))
    }

    /// Adds a handler that triggers when the cursor exits this view's bounds.
    ///
    /// This event can fire multiple times as the cursor moves in and out of the view.
    /// Only affects platforms with cursor support (macOS, iPadOS with trackpad, Android API 24+).
    ///
    /// # Arguments
    /// * `handler` - The action to execute when hover ends
    fn on_hover_exit<H, Args>(self, handler: H) -> Metadata<OnEvent, Self>
    where
        H: Handler<Args, ()>,
    {
        Metadata::new(self, OnEvent::hover_exit(handler))
    }

    /// Handles key presses that the focused view inside this one leaves
    /// unconsumed.
    ///
    /// The keyboard goes to the focused view first. A key it does not use —
    /// Escape, or an arrow in a single-line text field — bubbles to its
    /// ancestors, nearest first, until a handler returns
    /// [`KeyHandling::Handled`]. The handler receives [`KeyPress`] as its first
    /// argument.
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::key::{KeyHandling, KeyPress, NamedKey, Key};
    ///
    /// let open = binding::<bool>(true);
    /// let query = binding::<Str>(Str::default());
    /// field("Search", &query).on_key_press(
    ///     |press: KeyPress, State(open): State<Binding<bool>>| {
    ///         if press.key() == &Key::Named(NamedKey::Escape) {
    ///             open.set(false);
    ///             KeyHandling::Handled
    ///         } else {
    ///             KeyHandling::Ignored
    ///         }
    ///     },
    /// ).state(&open);
    /// ```
    ///
    /// Omitting [`KeyPress`] as the first parameter fails to compile:
    ///
    /// ```compile_fail
    /// use waterui::key::KeyHandling;
    /// use waterui::prelude::*;
    ///
    /// text!("Key").on_key_press(|| KeyHandling::Ignored);
    /// ```
    fn on_key_press<H, Args>(self, handler: H) -> Metadata<OnKeyPress, Self>
    where
        H: EventHandler<KeyPress, Args, KeyHandling>,
    {
        Metadata::new(self, OnKeyPress::new(handler))
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
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::cursor::CursorStyle;
    ///
    /// let clickable = text!("Click me").cursor(CursorStyle::PointingHand);
    /// ```
    ///
    /// # Arguments
    /// * `style` - The cursor style to display (can be reactive)
    fn cursor(
        self,
        style: impl IntoComputed<crate::cursor::CursorStyle>,
    ) -> Metadata<crate::cursor::Cursor, Self> {
        Metadata::new(self, crate::cursor::Cursor::new(style))
    }

    /// Adds a badge to this view.
    ///
    /// # Arguments
    /// * `value` - The badge indicator to display (`u32`, [`BadgeValue`](crate::component::badge::BadgeValue), or reactive signal; `0` hides)
    fn badge(self, value: impl IntoComputed<BadgeValue>) -> Badge<Self> {
        Badge::new(value, self)
    }

    /// Fixes this view's width to the provided value or signal.
    fn width(self, width: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).width(width)
    }

    /// Fixes this view's height to the provided value or signal.
    fn height(self, height: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).height(height)
    }

    /// Sets the width this view asks for when its parent proposes nothing.
    fn ideal_width(self, width: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).ideal_width(width)
    }

    /// Sets the height this view asks for when its parent proposes nothing.
    fn ideal_height(self, height: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).ideal_height(height)
    }

    /// Applies a minimum width constraint.
    fn min_width(self, width: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).min_width(width)
    }

    /// Applies a maximum width constraint.
    fn max_width(self, width: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).max_width(width)
    }

    /// Applies a minimum height constraint.
    fn min_height(self, height: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).min_height(height)
    }

    /// Applies a maximum height constraint.
    fn max_height(self, height: impl IntoSignalF32 + 'static) -> Frame<Self> {
        Frame::new(self).max_height(height)
    }

    /// Fixes both width and height simultaneously.
    fn size(
        self,
        width: impl IntoSignalF32 + 'static,
        height: impl IntoSignalF32 + 'static,
    ) -> Frame<Self> {
        Frame::new(self).width(width).height(height)
    }

    /// Applies minimum constraints on both axes.
    fn min_size(
        self,
        width: impl IntoSignalF32 + 'static,
        height: impl IntoSignalF32 + 'static,
    ) -> Frame<Self> {
        Frame::new(self).min_width(width).min_height(height)
    }

    /// Applies maximum constraints on both axes.
    fn max_size(
        self,
        width: impl IntoSignalF32 + 'static,
        height: impl IntoSignalF32 + 'static,
    ) -> Frame<Self> {
        Frame::new(self).max_width(width).max_height(height)
    }

    /// Aligns this view within its frame using the provided alignment or token.
    fn alignment(self, alignment: impl Into<Alignment>) -> Frame<Self> {
        Frame::new(self).alignment(alignment)
    }

    /// Overrides a horizontal alignment guide for this view.
    fn horizontal_alignment_guide<F>(
        self,
        alignment: HorizontalAlignment,
        compute: F,
    ) -> HorizontalAlignmentGuide<Self, F>
    where
        F: Fn(&ViewDimensions) -> f32 + 'static,
    {
        HorizontalAlignmentGuide::new(self, alignment, compute)
    }

    /// Overrides a vertical alignment guide for this view.
    fn vertical_alignment_guide<F>(
        self,
        alignment: VerticalAlignment,
        compute: F,
    ) -> VerticalAlignmentGuide<Self, F>
    where
        F: Fn(&ViewDimensions) -> f32 + 'static,
    {
        VerticalAlignmentGuide::new(self, alignment, compute)
    }

    /// Adds padding to this view with custom edge insets.
    ///
    /// `EdgeInsets` converts from a single `f32` (every edge), a
    /// `(vertical, horizontal)` pair and a `[top, leading, bottom, trailing]`
    /// array, so the common shapes need no constructor:
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// text!("a").padding_with(16.0);
    /// text!("b").padding_with((8.0, 16.0));
    /// text!("c").padding_with([4.0, 16.0, 12.0, 16.0]);
    /// ```
    ///
    /// # Arguments
    /// * `edge` - The edge insets to apply as padding
    fn padding_with(self, edge: impl IntoComputed<EdgeInsets>) -> Padding<Self> {
        Padding::new(edge, self)
    }

    /// Adds padding on the leading and trailing edges only.
    fn padding_horizontal(self, value: impl IntoSignalF32) -> Padding<Self> {
        Padding::new(
            value
                .into_signal_f32()
                .map(|value| EdgeInsets::symmetric(0.0, value)),
            self,
        )
    }

    /// Adds padding on the top and bottom edges only.
    fn padding_vertical(self, value: impl IntoSignalF32) -> Padding<Self> {
        Padding::new(
            value
                .into_signal_f32()
                .map(|value| EdgeInsets::symmetric(value, 0.0)),
            self,
        )
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
    fn padding(self) -> Padding<Self> {
        Padding::new(EdgeInsets::all(DEFAULT_PADDING), self)
    }

    /// Marks this view as secure.
    ///
    /// User would be forbidden to take a screenshot of the view.
    ///
    /// # Arguments
    /// * `secure` - The secure metadata to apply
    fn secure(self) -> Metadata<Secure, Self> {
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
    /// The label is reactive: pass a signal and a label derived from app state
    /// (`"3 unread messages"`) stays current without rebuilding the subtree.
    ///
    /// # Arguments
    /// * `label` - The accessibility label to apply, constant or reactive
    fn a11y_label(self, label: impl IntoComputed<Str>) -> Metadata<AccessibilityLabel, Self> {
        Metadata::new(self, accessibility::AccessibilityLabel::new(label))
    }

    /// Sets the accessibility value for this view.
    ///
    /// The value is the node's own semantic content, announced by assistive
    /// technologies after the label — a formula's spoken mathematics, a chart's
    /// summary, a document's description. It exists beside the label, so
    /// applying a human-readable `.a11y_label(...)` no longer discards the
    /// content a component publishes about itself.
    ///
    /// The value is reactive: pass a signal and it stays current without
    /// rebuilding the subtree.
    ///
    /// # Arguments
    /// * `value` - The accessibility value to apply, constant or reactive
    fn a11y_value(self, value: impl IntoComputed<Str>) -> Metadata<AccessibilityValue, Self> {
        Metadata::new(self, accessibility::AccessibilityValue::new(value))
    }

    /// Sets the accessibility role for this view.
    ///
    /// # Arguments
    /// * `role` - The accessibility role to apply
    fn a11y_role(
        self,
        role: accessibility::AccessibilityRole,
    ) -> Metadata<AccessibilityRole, Self> {
        Metadata::new(self, role)
    }

    /// Sets a stable automation identifier for locating this view in UI tests.
    ///
    /// Identifiers are invisible to end users and assistive technologies; they
    /// exist for `waterui-testing` selectors and native automation frameworks
    /// (`XCUITest` `accessibilityIdentifier`, Android `UiAutomator`).
    fn a11y_id(self, identifier: impl Into<Str>) -> Metadata<AccessibilityIdentifier, Self> {
        Metadata::new(
            self,
            accessibility::AccessibilityIdentifier::new(identifier),
        )
    }

    /// Overrides whether this view is hidden from assistive technologies.
    ///
    /// Accepts a constant `bool` or any reactive boolean signal.
    fn a11y_hidden(self, hidden: impl IntoComputed<bool>) -> Metadata<AccessibilityHidden, Self> {
        Metadata::new(self, accessibility::AccessibilityHidden::new(hidden))
    }

    /// Chooses which rectangle this view's accessibility element reports.
    ///
    /// An element reports the view's own answer, positioned in its assigned
    /// frame, by default. `AccessibilityGeometry::Allocation` reports the
    /// whole assigned frame — a canvas whose drawing fills it, or a gap a
    /// stack allocated to a spacer — and `AccessibilityGeometry::Region`
    /// an explicit rectangle in the view's own coordinates. Hit testing,
    /// focus and drawing are unaffected.
    fn a11y_geometry(
        self,
        geometry: AccessibilityGeometry,
    ) -> Metadata<AccessibilityGeometry, Self> {
        Metadata::new(self, geometry)
    }

    /// Controls how descendants contribute semantics for this view.
    fn a11y_children(
        self,
        behavior: accessibility::AccessibilityChildren,
    ) -> Metadata<AccessibilityChildren, Self> {
        Metadata::new(self, behavior)
    }

    /// Applies static or reactive accessibility state metadata.
    fn a11y_state(
        self,
        state: impl IntoComputed<accessibility::AccessibilityState>,
    ) -> Metadata<accessibility::AccessibilityStateSignal, Self> {
        Metadata::new(self, accessibility::AccessibilityStateSignal::new(state))
    }

    /// Temporary Step-1 compatibility alias until split crates migrate to
    /// [`ViewExt::a11y_state`].
    #[doc(hidden)]
    fn a11y_state_signal(
        self,
        state: impl IntoComputed<accessibility::AccessibilityState>,
    ) -> Metadata<accessibility::AccessibilityStateSignal, Self> {
        self.a11y_state(state)
    }

    /// Observes a gesture and executes an action whose handler consumes
    /// `G::Event` as its first argument, followed by environment extractors.
    ///
    /// The action resolves its extractors from the environment of the view it
    /// is attached to, so a value it extracts with `State<T>` must be installed
    /// with [`ViewExt::state`] on an ancestor: `view.gesture(..).state(&value)`,
    /// not `view.state(&value).gesture(..)`.
    ///
    /// # Arguments
    /// * `gesture` - The gesture to observe
    /// * `action` - The action to execute when the gesture is recognized
    ///
    /// Passing a handler expecting another gesture's event type fails to
    /// compile:
    ///
    /// ```compile_fail
    /// use waterui::gesture::{DragEvent, TapGesture};
    /// use waterui::prelude::*;
    ///
    /// text!("Tap").gesture(TapGesture::new(), |_: DragEvent| {});
    /// ```
    fn gesture<G, H, Args>(self, gesture: G, action: H) -> Metadata<GestureObserver, Self>
    where
        G: GestureType,
        H: EventHandler<G::Event, Args>,
    {
        Metadata::new(self, GestureObserver::from_event(gesture, action))
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
    /// text!("Click me").on_tap(|| {});
    /// ```
    fn on_tap<H, Args>(self, action: H) -> Metadata<GestureObserver, Self>
    where
        H: Handler<Args, ()>,
    {
        Metadata::new(
            self,
            GestureObserver::without_event(TapGesture::new(), action),
        )
    }

    /// Adds a long-press gesture recognizer to this view using the platform's
    /// default long-press timeout.
    fn on_long_press<H, Args>(self, action: H) -> Metadata<GestureObserver, Self>
    where
        H: Handler<Args, ()>,
    {
        Metadata::new(
            self,
            GestureObserver::without_event(LongPressGesture::new(), action),
        )
    }

    /// Plays haptic `feedback` whenever `trigger`'s value changes.
    ///
    /// Nothing runs on initial mount or when consecutive updates produce an
    /// equal value. Use [`Haptic`](crate::haptic::Haptic) inside gesture
    /// handlers when an imperative trigger is needed.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// let saved_count = Binding::new(0_i32);
    /// text!("Saved").sensory_feedback(Feedback::Success, &saved_count);
    /// ```
    #[cfg(feature = "haptics")]
    fn sensory_feedback<C>(self, feedback: Feedback, trigger: &C) -> OnChange<Self>
    where
        C: Signal,
        C::Output: PartialEq + Clone,
    {
        self.on_change(trigger, move |_value: C::Output| {
            feedback.trigger();
        })
    }

    /// Applies a shadow effect to this view.
    fn shadow(self, shadow: impl Into<Shadow>) -> Metadata<Shadow, Self> {
        Metadata::new(self, shadow.into())
    }

    /// Promotes this view to a themed floating surface with elevation.
    ///
    /// Floating presentation is an attribute: applying it to a semantic button
    /// preserves the button's identity while changing its container, state-layer
    /// geometry, and elevation.
    fn floating(self) -> Floating<Self> {
        Floating::new(self)
    }

    /// Promotes this view with explicit floating-surface theme tokens.
    fn floating_with(self, style: FloatingStyle) -> Floating<Self> {
        Floating::with_style(self, style)
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
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Simple border on all edges
    /// text!("Hello").border(Color::red(), 2.0);
    /// ```
    fn border(self, color: impl IntoSignal<Color> + 'static, width: f32) -> Metadata<Border, Self> {
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
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::border::Border;
    ///
    /// let border = Border::new(Color::blue(), 2.0)
    ///     .corner_radius(12.0)
    ///     .edges(EdgeSet::HORIZONTAL);
    ///
    /// text!("Custom").border_with(border);
    /// ```
    fn border_with(self, border: Border) -> Metadata<Border, Self> {
        Metadata::new(self, border)
    }

    /// Applies a uniform scale transform to this view around its center.
    ///
    /// Scales are purely visual and do not affect layout calculations.
    ///
    /// # Arguments
    /// * `factor` - The uniform scale factor (1.0 = no scale, 0.5 = half size, 2.0 = double size)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Scale uniformly to 150%
    /// let bigger = text!("Hello").scale(1.5);
    ///
    /// // Animate scale
    /// let s = binding::<f32>(1.0_f32).animated();
    /// let animated = text!("Hello").scale(s);
    /// ```
    fn scale(self, factor: impl IntoSignalF32) -> Metadata<Scale, Self> {
        Metadata::new(self, Scale::uniform(factor))
    }

    /// Applies a per-axis scale transform to this view around its center.
    ///
    /// Scales are purely visual and do not affect layout calculations.
    ///
    /// # Arguments
    /// * `x` - The horizontal scale factor (1.0 = original width, can be reactive)
    /// * `y` - The vertical scale factor (1.0 = original height, can be reactive)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Scale X only (stretch horizontally)
    /// let wider = text!("Hello").scale_xy(2.0, 1.0);
    /// ```
    fn scale_xy(self, x: impl IntoSignalF32, y: impl IntoSignalF32) -> Metadata<Scale, Self> {
        Metadata::new(self, Scale::xy(x, y))
    }

    /// Applies a uniform scale transform around a specific anchor point.
    ///
    /// # Arguments
    /// * `factor` - The uniform scale factor (1.0 = original size, can be reactive)
    /// * `anchor` - The anchor point for the scale (e.g., `Anchor::TOP_LEFT`)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::style::Anchor;
    ///
    /// // Scale from top-left corner
    /// let shrunk = text!("Hello").scale_from(0.5, Anchor::TOP_LEFT);
    /// ```
    fn scale_from(self, factor: impl IntoSignalF32, anchor: Anchor) -> Metadata<Scale, Self> {
        Metadata::new(self, Scale::uniform_from(factor, anchor))
    }

    /// Applies a per-axis scale transform around a specific anchor point.
    ///
    /// # Arguments
    /// * `x` - The horizontal scale factor
    /// * `y` - The vertical scale factor
    /// * `anchor` - The anchor point for the scale (e.g., `Anchor::TOP_LEFT`)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::style::Anchor;
    ///
    /// // Stretch horizontally from top-left corner
    /// let wider = text!("Hello").scale_xy_from(2.0, 1.0, Anchor::TOP_LEFT);
    /// ```
    fn scale_xy_from(
        self,
        x: impl IntoSignalF32,
        y: impl IntoSignalF32,
        anchor: Anchor,
    ) -> Metadata<Scale, Self> {
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
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Rotate 45 degrees
    /// let tilted = text!("Hello").rotation(45.0);
    ///
    /// // Animate rotation
    /// let angle = binding::<f32>(0.0_f32).animated();
    /// let spinning = text!("Hello").rotation(angle);
    /// ```
    fn rotation(self, degrees: impl IntoSignalF32) -> Metadata<Rotation, Self> {
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
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::style::Anchor;
    ///
    /// // Rotate around top-left corner
    /// let tilted = text!("Hello").rotation_from(45.0, Anchor::TOP_LEFT);
    /// ```
    fn rotation_from(
        self,
        degrees: impl IntoSignalF32,
        anchor: Anchor,
    ) -> Metadata<Rotation, Self> {
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
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Move view by (10, 20) points
    /// let nudged = text!("Hello").offset(10.0, 20.0);
    ///
    /// // Animate offset
    /// let x = binding::<f32>(0.0_f32).animated();
    /// let sliding = text!("Hello").offset(x, 0.0);
    /// ```
    fn offset(self, x: impl IntoSignalF32, y: impl IntoSignalF32) -> Metadata<Offset, Self> {
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
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::shape::*;
    ///
    /// // Clip a view to a circle
    /// let avatar = Color::red().clip(Circle);
    ///
    /// // Clip to rounded rectangle
    /// let card = text!("Card").clip(RoundedRectangle::new(8.0));
    ///
    /// // Custom triangle shape
    /// let triangle = Path::new()
    ///     .move_to(0.5, 0.0)
    ///     .line_to(1.0, 1.0)
    ///     .line_to(0.0, 1.0)
    ///     .close();
    /// Color::red().size(100.0, 100.0).clip(triangle);
    /// ```
    fn clip(self, shape: impl Shape) -> Metadata<ClipShape, Self> {
        Metadata::new(self, ClipShape::new(shape))
    }

    /// Attaches a context menu to this view.
    ///
    /// The context menu appears when the user:
    /// - Long-presses on iOS/Android, or holds a touch or pen press elsewhere
    /// - Secondary-clicks with a pointer
    ///
    /// Pass the menu items directly, or a
    /// [`ContextMenu`](crate::metadata::context_menu::ContextMenu) to also
    /// choose the lifted preview and anchor an interactive accessory to it.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// text!("Right-click me")
    ///     .context_menu(vec![
    ///         "Copy".action(|| {}),
    ///         "Paste".action(|| {}),
    ///     ]);
    /// ```
    #[cfg(feature = "menu")]
    fn context_menu<M: crate::metadata::context_menu::IntoContextMenu>(
        self,
        menu: M,
    ) -> crate::metadata::context_menu::ContextMenuView<Self, M::Preview, M::Accessory> {
        crate::metadata::context_menu::ContextMenuView::new(self, menu.into_context_menu())
    }

    /// Presents `overlay` next to this view, above all other content in the
    /// window, while its binding is `true`.
    ///
    /// The backend places it against the preferred edge of this view, flips
    /// it to the opposite edge when there is no room, and keeps it inside the
    /// window, as the overlay's
    /// [`AnchorPlacement`](crate::metadata::anchored_overlay::AnchorPlacement)
    /// declares.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::metadata::anchored_overlay::{AnchorEdge, AnchoredOverlay};
    /// use waterui::prelude::*;
    ///
    /// let open = binding(false);
    /// let trigger = button("Options")
    ///     .action({
    ///         let open = open.clone();
    ///         move || open.set(true)
    ///     })
    ///     .anchored_overlay(AnchoredOverlay::new(&open, move || text!("Popover")).edge(AnchorEdge::Bottom));
    /// ```
    fn anchored_overlay(
        self,
        overlay: crate::metadata::anchored_overlay::AnchoredOverlay,
    ) -> Metadata<crate::metadata::anchored_overlay::AnchoredOverlay, Self> {
        Metadata::new(self, overlay)
    }

    /// Presents `dialog` window-modal while its `is_presented` binding is
    /// `true`.
    ///
    /// The dialog renders full-window wherever it is attached — the content
    /// beneath is inert to pointer, keyboard and accessibility while it is up.
    /// Choosing an action runs that action's handler and writes
    /// `is_presented = false`; Escape, the Android back gesture and the scrim
    /// tap run the `Cancel` action's handler the same way, and do nothing when
    /// no `Cancel` action was declared. One dialog presents per window at a
    /// time; a second one waits in presentation order.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::dialog::{Dialog, DialogAction};
    /// use waterui::prelude::*;
    ///
    /// let confirm = binding(false);
    /// button("Discard")
    ///     .action({
    ///         let confirm = confirm.clone();
    ///         move || confirm.set(true)
    ///     })
    ///     .dialog(
    ///         Dialog::new(&confirm, "Discard changes?")
    ///             .action(DialogAction::destructive("Discard", || {}))
    ///             .action(DialogAction::cancel("Keep", || {})),
    ///     );
    /// ```
    #[cfg(feature = "dialog")]
    fn dialog(self, dialog: crate::dialog::Dialog) -> Metadata<crate::dialog::Dialog, Self> {
        Metadata::new(self, dialog)
    }

    /// Presents `sheet` window-modal while its `is_presented` binding is `true`.
    ///
    /// While presented, everything beneath the sheet is inert to pointer,
    /// keyboard, and assistive technology; focus moves into the sheet and
    /// returns to the previously focused element on dismissal. When
    /// [`Sheet::is_dismissible`](crate::sheet::Sheet::is_dismissible) is `true`,
    /// Escape, the platform back action, a scrim tap, or interactive swipe-down
    /// invokes [`Sheet::on_dismiss`](crate::sheet::Sheet::on_dismiss) and writes
    /// `is_presented = false`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::sheet::{Sheet, SheetDetent, SheetDismiss};
    ///
    /// let open = binding(false);
    /// button("Filter")
    ///     .action({
    ///         let open = open.clone();
    ///         move || open.set(true)
    ///     })
    ///     .sheet(
    ///         Sheet::new(&open, || {
    ///             button("Done").action(|sheet: SheetDismiss| sheet.dismiss())
    ///         })
    ///         .detents([SheetDetent::Medium, SheetDetent::Large]),
    ///     );
    /// ```
    fn sheet(self, sheet: Sheet) -> Metadata<Sheet, Self> {
        Metadata::new(self, sheet)
    }

    /// Hosts a [`SnackbarManager`](crate::snackbar::SnackbarManager) and its
    /// overlay for this view subtree.
    ///
    /// Descendant handlers can extract `SnackbarManager` bare
    /// (`|manager: SnackbarManager| ...`). Each built instance creates its own
    /// manager and overlay stack, keeping multiple windows and scenes isolated.
    /// When `flow-markdown` is enabled, markdown code blocks inside this
    /// subtree also show copy feedback through the hosted manager.
    #[cfg(feature = "snackbar")]
    fn snackbar_host(self) -> SnackbarHost<Self> {
        SnackbarHost::new(self)
    }

    /// Hosts a [`SnackbarManager`](crate::snackbar::SnackbarManager) accepting
    /// snackbars with leading icon views of type `I`, alongside plain
    /// icon-less snackbars.
    ///
    /// Descendant handlers can extract `SnackbarManager<I>` bare.
    #[cfg(feature = "snackbar")]
    fn snackbar_host_with_icons<I: View + 'static>(self) -> SnackbarHost<Self, I> {
        SnackbarHost::new(self)
    }

    /// Sets the minimum row height of every list inside this view, in points.
    ///
    /// Replaces the theme's one-line row height as the floor rows are measured
    /// against; `0.0` sizes each row to its content plus its insets. Combine
    /// with [`ViewExt::list_row_insets`] to make individual rows compact.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::layout::padding::EdgeInsets;
    /// use waterui::prelude::*;
    ///
    /// let log = List::content((
    ///     || text!("Alice joined the group").list_row_insets(EdgeInsets::all(4.0)),
    ///     || text!("Hello"),
    /// ))
    /// .list_min_row_height(0.0);
    /// ```
    #[cfg(feature = "list")]
    fn list_min_row_height(self, height: f32) -> With<Self, ListMinRowHeight> {
        self.with(ListMinRowHeight(height))
    }

    /// Sets whether this list row can be deleted using a reactive signal.
    ///
    /// When `false`, swipe-to-delete and delete buttons are disabled for this
    /// row. Rows are deletable by default when the enclosing list provides
    /// `on_delete`.
    #[cfg(feature = "list")]
    fn list_row_deletable(
        self,
        deletable: impl IntoComputed<bool>,
    ) -> With<Self, ListRowDeletable> {
        self.with(ListRowDeletable(deletable.into_computed()))
    }

    /// Replaces the theme's row insets for this list row.
    ///
    /// Together with [`ViewExt::list_min_row_height`], this lets one row size
    /// to its content (a one-line event in a chat log, a compact header) while
    /// the rest keep the list's row metrics.
    #[cfg(feature = "list")]
    fn list_row_insets(self, insets: EdgeInsets) -> With<Self, ListRowInsets> {
        self.with(ListRowInsets(insets))
    }

    /// Extends this view's bounds to ignore safe area insets on the specified
    /// regions and edges.
    ///
    /// This allows backgrounds, images, and other visual elements to extend
    /// edge-to-edge while content remains in the safe area. The native renderer
    /// lays the view out clear of every region it does not ignore.
    ///
    /// # Arguments
    /// * `ignore` - The regions and edges to ignore: an
    ///   [`EdgeSet`](waterui_layout::EdgeSet) ignores every region on its
    ///   edges, and [`SafeAreaRegions::on`](waterui_layout::SafeAreaRegions::on)
    ///   names one region.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Extend background to fill entire screen
    /// let backdrop = Color::red()
    ///     .ignore_safe_area(EdgeSet::ALL);
    ///
    /// // Only extend to top (under status bar)
    /// let header = text!("Title")
    ///     .ignore_safe_area(EdgeSet::TOP);
    ///
    /// // Lay an editor under the keyboard but still above the container inset
    /// let editor = text!("editor")
    ///     .ignore_safe_area(SafeAreaRegions::KEYBOARD.on(EdgeSet::BOTTOM));
    /// ```
    fn ignore_safe_area(self, ignore: impl Into<IgnoreSafeArea>) -> Metadata<IgnoreSafeArea, Self> {
        Metadata::new(self, ignore.into())
    }

    /// Installs a plugin into the environment.
    fn install(self, plugin: impl Plugin) -> impl View {
        Installed::new(self, plugin)
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
    ///     let guard = count.clone().watch(|v| {
    ///         let _ = v.into_value();
    ///     });
    ///     text("Hello").retain(guard)
    /// }
    /// ```
    fn retain<T: 'static>(self, value: T) -> Metadata<Retain, Self> {
        Metadata::new(self, Retain::new(value))
    }

    /// Makes this view draggable, carrying `payload`.
    ///
    /// When the user drags this view (click-drag on macOS, long-press-drag on iOS/Android),
    /// the payload travels to drop destinations that accept its type. [`Str`],
    /// [`Url`](crate::Url) and [`Files`](crate::drag_drop::Files) also reach other
    /// applications; an application's own [`Transferable`] types stay in the process.
    ///
    /// # Arguments
    /// * `payload` - The value to transfer: a plain [`Transferable`] value, or a `Binding` /
    ///   `Computed` of one, read when the drag begins
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::Str;
    ///
    /// text!("Drag me")
    ///     .draggable(Str::from("Hello!"));
    /// ```
    fn draggable<S>(self, payload: S) -> Metadata<Draggable, Self>
    where
        S: waterui_core::Signal + Clone + 'static,
        S::Output: Transferable,
    {
        Metadata::new(self, Draggable::new(payload))
    }

    /// Makes this view a drop destination for dragged values.
    ///
    /// The handler's first argument is the dropped value, and its type is the type this
    /// destination accepts: a drag carrying any other type is neither highlighted nor
    /// delivered. The remaining arguments are extractors, so local state injected with
    /// [`ViewExt::state`] composes.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::Str;
    ///
    /// // Accepts plain text only.
    /// text!("Drop here")
    ///     .drop_destination(|text: Str| {
    ///         let _ = text;
    ///     });
    ///
    /// // With injected state
    /// let items = binding::<Vec<String>>(Vec::new());
    /// let count = binding::<i32>(0);
    /// text!("Drop here")
    ///     .state(&items)
    ///     .state(&count)
    ///     .drop_destination(
    ///         |text: Str,
    ///          State(items): State<Binding<Vec<String>>>,
    ///          State(count): State<Binding<i32>>| {
    ///             items.get_mut().push(text.to_string());
    ///             *count.get_mut() += 1;
    ///         },
    ///     );
    /// ```
    fn drop_destination<T, H, Args>(self, on_drop: H) -> Metadata<DropDestination, Self>
    where
        T: Transferable,
        H: EventHandler<T, Args>,
    {
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
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Make a view transparent to touch events
    /// let passthrough = text!("Click through me")
    ///     .hittable(false);
    ///
    /// // Reactive hit testing control
    /// let can_interact = binding::<bool>(true);
    /// button("Click me").action(|| {})
    ///     .hittable(can_interact);
    /// ```
    fn hittable(self, enabled: impl IntoComputed<bool>) -> Metadata<Hittable, Self> {
        Metadata::new(self, Hittable::new(enabled))
    }

    /// Disables every interactive control in this view subtree.
    ///
    /// Installs a [`waterui_interaction::interaction::Disabled`] scope into the
    /// subtree's environment, so controls (toggles, buttons, sliders, …)
    /// render their platform-correct disabled appearance and stop responding
    /// to input. Nested `.disabled(...)` scopes OR-combine: a control stays
    /// disabled while any enclosing scope is disabled. The subtree also stops
    /// hit-testing entirely and its accessibility nodes report the disabled
    /// state to assistive technologies.
    ///
    /// # Arguments
    /// * `is_disabled` - Whether the subtree is disabled (can be reactive)
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// // Disable a button
    /// button("Submit").action(|| {})
    ///     .disabled(true);
    ///
    /// // Reactive disable based on form validity
    /// let is_submitting = binding::<bool>(false);
    /// let name = binding::<Str>(Str::default());
    /// vstack((
    ///     TextField::new("Name", &name),
    ///     button("Submit").action(|| {}),
    /// )).disabled(is_submitting);
    /// ```
    fn disabled(self, is_disabled: impl IntoComputed<bool>) -> impl View {
        let is_disabled = is_disabled.into_computed();

        // Disabled scopes OR-combine (`Disabled::install`), so an enabled
        // scope leaves the flag to the enclosing scope rather than claim it.
        let accessibility_state = is_disabled.map(|disabled| {
            if disabled {
                AccessibilityState::new().disabled(true)
            } else {
                AccessibilityState::new()
            }
        });
        let hittable_value = is_disabled.map(|d| !d);

        self.a11y_state(accessibility_state)
            .hittable(hittable_value)
            .install(DisabledScope(is_disabled))
    }

    /// Reports the state of the interactive control this modifies — or the
    /// outermost one inside it — into `state`, whenever it changes.
    ///
    /// Use it to style chrome the control does not draw itself:
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::interaction::InteractionState;
    ///
    /// let state = binding(InteractionState::empty());
    /// let outline = state.map(|s: InteractionState| if s.contains(InteractionState::FOCUSED) { 3.0 } else { 1.0 });
    /// let chip = button("Filter").action(|| {}).interaction_state(&state);
    /// ```
    fn interaction_state(self, state: &Binding<InteractionState>) -> impl View + use<Self> {
        self.install(InteractionReport(state.clone()))
    }

    /// Marks the interactive control this modifies as selected.
    ///
    /// A selected control is in [`InteractionState::SELECTED`], so its
    /// style's selected values apply — a selected list item's container, a
    /// selected navigation destination's indicator — and assistive technology
    /// announces it as selected. It applies to the control it modifies, not to
    /// controls nested inside that control.
    ///
    /// [`InteractionState::SELECTED`]: waterui_interaction::interaction::InteractionState::SELECTED
    fn selected(self, is_selected: impl IntoComputed<bool>) -> Metadata<Selected, Self> {
        Metadata::new(self, Selected(is_selected.into_computed()))
    }

    /// Injects cloneable state into this view subtree's environment.
    ///
    /// Actions and event handlers can later extract the injected value using
    /// [`waterui_core::extract::State`] in their handler parameters — or bare,
    /// as `value: T`, when `T` is an owned type marked `#[state]`.
    ///
    /// A handler sees the environment its modified view resolves in, so a
    /// `.state` install reaches the handler wherever it sits in the same
    /// modifier chain — `.state(&v).on_hover_exit(h)` and
    /// `.on_hover_exit(h).state(&v)` deliver the same value. State injected
    /// inside the view's own body is not part of the chain and stays invisible
    /// to the handler.
    ///
    /// # Example
    ///
    /// ```rust
    /// use waterui::prelude::*;
    ///
    /// let hover_count = binding::<i32>(0);
    /// let is_hovered = binding::<bool>(false);
    ///
    /// // The installs may sit before the handlers in the chain…
    /// let hoverable = text!("Hover Me!")
    ///     .state(&hover_count)
    ///     .state(&is_hovered)
    ///     .on_hover_enter(
    ///         |State(count): State<Binding<i32>>, State(hovered): State<Binding<bool>>| {
    ///             *count.get_mut() += 1;
    ///             hovered.set(true);
    ///         },
    ///     )
    ///     .on_hover_exit(|State(hovered): State<Binding<bool>>| {
    ///         hovered.set(false);
    ///     });
    ///
    /// // …or after them — both spellings resolve to the same environment.
    /// let equivalent = text!("Hover Me!")
    ///     .on_hover_enter(|State(count): State<Binding<i32>>| {
    ///         *count.get_mut() += 1;
    ///     })
    ///     .state(&hover_count);
    /// ```
    fn state<T: Clone + 'static>(self, state: &T) -> With<Self, State<T>> {
        With::new(self, State(state.clone()))
    }

    /// Wires `handle` to the active backend's offscreen renderer when this view
    /// dispatches, without altering the view's layout or appearance.
    ///
    /// Views of type `V` rendered through `handle` retain their concrete type
    /// across backend dispatch instead of erasing to [`AnyView`](waterui_core::AnyView).
    #[cfg(feature = "capture")]
    fn capture<V: View>(self, handle: &CaptureHandle<V>) -> CaptureScope<V, Self> {
        handle.attach(self)
    }
}

impl<V: View + Sized> ViewExt for V {}

/// The scope [`ViewExt::disabled`] installs: OR-combined with any disabled
/// scope inherited from an enclosing `.disabled(...)`.
struct DisabledScope(Computed<bool>);

impl Plugin for DisabledScope {
    fn install(self, env: &mut Environment) {
        Disabled::install(env, self.0);
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::{AnyView, Binding, View};
    use waterui_interaction::interaction::InteractionState;

    use super::ViewExt;

    /// `interaction_state` clones the binding it reports into, so the view it
    /// returns must not borrow it: the binding is usually a local the body
    /// also reads.
    #[test]
    fn interaction_state_does_not_borrow_the_binding() {
        fn reported(content: AnyView) -> impl View {
            let state = Binding::new(InteractionState::empty());
            content.interaction_state(&state)
        }

        let _ = reported;
    }

    #[cfg(feature = "haptics")]
    #[test]
    fn sensory_feedback_does_not_borrow_trigger() {
        use nami::Binding;

        use crate::haptic::Feedback;

        fn triggered(content: AnyView) -> impl View {
            let count = Binding::new(0_i32);
            content.sensory_feedback(Feedback::Success, &count)
        }

        let _ = triggered;
    }

    #[cfg(feature = "navigation")]
    #[test]
    fn text_supports_navigation_title_without_shadowing() {
        use crate::{navigation::NavigationView, text::text};

        let page = text("About").navigation_title("About");
        let _body: NavigationView<_> = page;
    }

    #[test]
    fn scale_uniform_and_xy_variants() {
        use nami::Signal as _;

        use crate::style::Anchor;

        let uniform = AnyView::new(()).scale(1.5);
        assert!((uniform.value.x().snapshot() - 1.5).abs() < f32::EPSILON);
        assert!((uniform.value.y().snapshot() - 1.5).abs() < f32::EPSILON);
        assert_eq!(uniform.value.anchor(), Anchor::CENTER);

        let xy = AnyView::new(()).scale_xy(2.0, 0.5);
        assert!((xy.value.x().snapshot() - 2.0).abs() < f32::EPSILON);
        assert!((xy.value.y().snapshot() - 0.5).abs() < f32::EPSILON);
        assert_eq!(xy.value.anchor(), Anchor::CENTER);

        let anchored = AnyView::new(()).scale_from(0.75, Anchor::TOP_LEFT);
        assert!((anchored.value.x().snapshot() - 0.75).abs() < f32::EPSILON);
        assert!((anchored.value.y().snapshot() - 0.75).abs() < f32::EPSILON);
        assert_eq!(anchored.value.anchor(), Anchor::TOP_LEFT);

        let anchored_xy = AnyView::new(()).scale_xy_from(1.25, 0.25, Anchor::BOTTOM_RIGHT);
        assert!((anchored_xy.value.x().snapshot() - 1.25).abs() < f32::EPSILON);
        assert!((anchored_xy.value.y().snapshot() - 0.25).abs() < f32::EPSILON);
        assert_eq!(anchored_xy.value.anchor(), Anchor::BOTTOM_RIGHT);
    }

    #[test]
    fn dynamic_range_returns_concrete_metadata_and_tracks_signal() {
        use nami::Signal as _;
        use waterui_core::Metadata;

        use crate::metadata::secure::{DynamicRange, DynamicRangeMetadata};

        let constant: Metadata<DynamicRangeMetadata, AnyView> =
            AnyView::new(()).dynamic_range(DynamicRange::Standard);
        assert_eq!(constant.value.range().snapshot(), DynamicRange::Standard);

        let range = Binding::new(DynamicRange::Standard);
        let reactive: Metadata<DynamicRangeMetadata, AnyView> =
            AnyView::new(()).dynamic_range(range.clone());
        assert_eq!(reactive.value.range().snapshot(), DynamicRange::Standard);
        range.set(DynamicRange::High);
        assert_eq!(reactive.value.range().snapshot(), DynamicRange::High);
    }

    #[test]
    fn text_font_size_does_not_shadow_view_ext_size() {
        use waterui_layout::frame::Frame;
        use waterui_text::{Text, text};

        let _framed: Frame<Text> = text("Hi").font_size(17.0).size(120.0, 40.0);
    }

    #[test]
    fn sizing_shortcuts_accept_signals_and_update_layout_without_rebuilding() {
        use alloc::rc::Rc;
        use alloc::vec::Vec;
        use core::cell::Cell;

        use nami::SignalExt as _;
        use waterui_core::{AnimationExt as _, Environment, Native};
        use waterui_layout::{
            LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView,
            TopLeading, ViewDimensions, container::FixedContainer,
        };

        struct FlexibleChild;

        impl SubView for FlexibleChild {
            fn resolve_extent(
                &self,
                bounds: Rect,
                proposal: ProposalSize,
            ) -> Result<ResolvedExtent, LayoutError> {
                ResolvedExtent::centred(bounds, self.measure(proposal).size)
            }

            fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
                ViewDimensions::new(Size::new(
                    proposal.width.unwrap_or(0.0),
                    proposal.height.unwrap_or(0.0),
                ))
            }

            fn stretch_axis(&self) -> StretchAxis {
                StretchAxis::Both
            }

            fn priority(&self) -> i32 {
                0
            }
        }

        let expanded = Binding::new(false);
        let animated_height = expanded.select(240.0_f32, 0.0_f32).animated();
        let width_signal = Binding::new(120.0_f32);

        let _ = AnyView::new(()).width(width_signal.clone());
        let _ = AnyView::new(()).height(animated_height.clone());
        let _ = AnyView::new(()).ideal_width(width_signal.clone());
        let _ = AnyView::new(()).ideal_height(animated_height.clone());
        let _ = AnyView::new(()).min_width(width_signal.clone());
        let _ = AnyView::new(()).max_width(width_signal.clone());
        let _ = AnyView::new(()).min_height(animated_height.clone());
        let _ = AnyView::new(()).max_height(animated_height.clone());
        let _ = AnyView::new(()).size(width_signal.clone(), animated_height.clone());
        let _ = AnyView::new(()).min_size(width_signal.clone(), animated_height.clone());
        let _ = AnyView::new(()).max_size(width_signal, animated_height.clone());
        let _ = AnyView::new(()).alignment(TopLeading);

        let env = Environment::new();
        let expanded_view = AnyView::new(())
            .height(animated_height)
            .body(&env)
            .body(&env);
        let native = AnyView::new(expanded_view)
            .downcast::<Native<FixedContainer<Vec<AnyView>>>>()
            .expect("Frame expands to Native<FixedContainer>");
        let container = native.into_inner();
        let (layout, _) = container.as_parts();

        let invalidations = Rc::new(Cell::new(0usize));
        let invalidations_for_cb = Rc::clone(&invalidations);
        let _guards = layout.watch_invalidation(Rc::new(move || {
            invalidations_for_cb.set(invalidations_for_cb.get() + 1);
        }));

        let proposal = ProposalSize::new(100.0, 400.0);
        assert!((layout.size_that_fits(proposal, &[]).height - 0.0).abs() < f32::EPSILON);

        expanded.set(true);
        assert!(invalidations.get() > 0);
        assert!((layout.size_that_fits(proposal, &[]).height - 240.0).abs() < f32::EPSILON);

        let ideal_w = Binding::new(90.0_f32);
        let ideal_h = Binding::new(45.0_f32);
        let ideal_view = AnyView::new(())
            .ideal_width(ideal_w.clone())
            .ideal_height(ideal_h.clone())
            .body(&env)
            .body(&env);
        let ideal_native = AnyView::new(ideal_view)
            .downcast::<Native<FixedContainer<Vec<AnyView>>>>()
            .expect("ideal Frame expands to Native<FixedContainer>");
        let ideal_container = ideal_native.into_inner();
        let (ideal_layout, _) = ideal_container.as_parts();
        let child = FlexibleChild;
        let children: [&dyn SubView; 1] = [&child];
        let initial_ideal = ideal_layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
        assert!((initial_ideal.width - 90.0).abs() < f32::EPSILON);
        assert!((initial_ideal.height - 45.0).abs() < f32::EPSILON);

        ideal_w.set(160.0);
        ideal_h.set(80.0);
        let updated_ideal = ideal_layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);
        assert!((updated_ideal.width - 160.0).abs() < f32::EPSILON);
        assert!((updated_ideal.height - 80.0).abs() < f32::EPSILON);
    }
}
