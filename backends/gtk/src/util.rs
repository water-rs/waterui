//! Shared utilities for the GTK backend.

use std::rc::Rc;

use glib::object::{ObjectExt, ObjectType};
use gtk4::prelude::{IsA, WidgetExt};
use gtk4::{CssProvider, Widget, gdk};
use nami::{
    Signal,
    watcher::{BoxWatcherGuard, Context},
};
use waterui::accessibility::{
    AccessibilityChildren, AccessibilityHidden, AccessibilityLabel, AccessibilityRole,
    AccessibilityState, AccessibilityStateSignal,
};
use waterui::background::{Background, MaterialBackground};
use waterui::border::Border;
use waterui::component::focus::Focused;
use waterui::cursor::Cursor;
use waterui::drag_drop::{Draggable, DropDestination};
use waterui::filter::Opacity;
use waterui::gesture::GestureObserver;
use waterui::interaction::Hittable;
use waterui::metadata::context_menu::ResolvedContextMenu;
use waterui::metadata::secure::{HighDynamicRange, Secure, StandardDynamicRange};
use waterui::navigation::{NavigationTransitionDestination, NavigationTransitionSource};
use waterui::style::{Offset, Rotation, Scale, Shadow};
use waterui_core::event::{LifeCycleHook, OnEvent};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, IgnorableMetadata, Metadata, Retain};
use waterui_graphics::AppliedFilter;
use waterui_graphics::color::ResolvedColor;
use waterui_layout::safe_area::IgnoreSafeArea;
use waterui_shape::ClipShape;

/// Installs a signal subscription before taking its initial snapshot.
pub fn subscribe_then_get<S>(
    signal: &S,
    watcher: impl Fn(Context<S::Output>) + 'static,
) -> (S::Output, S::Guard)
where
    S: Signal,
{
    let guard = signal.watch(watcher);
    (signal.get(), guard)
}

/// Widget-data key under which every reactive watcher guard for a widget is
/// accumulated. Only [`store_watcher_guard`] and [`store_watcher_guards`]
/// touch this key, and it always holds a `Vec<BoxWatcherGuard>`.
const WATCHER_GUARDS_DATA_KEY: &str = "waterui_watcher_guards";

/// Stores a watcher guard on a widget to prevent it from being dropped.
///
/// Guards accumulate: storing a new guard keeps every previously stored one
/// alive, so independent subscriptions attached to the same widget (opacity,
/// cursor, hit-testing, ...) do not cancel each other.
pub fn store_watcher_guard(widget: &impl ObjectExt, guard: BoxWatcherGuard) {
    store_watcher_guards(widget, vec![guard]);
}

/// Stores multiple watcher guards on a widget.
///
/// Use this when a component has multiple reactive subscriptions that need
/// to be kept alive with the widget.
pub fn store_watcher_guards(widget: &impl ObjectExt, guards: Vec<BoxWatcherGuard>) {
    // SAFETY: `WATCHER_GUARDS_DATA_KEY` is private to this function pair and
    // always stores a `Vec<BoxWatcherGuard>`, so the retrieved type matches the
    // stored type; widget data is only touched on the GTK main thread, and the
    // stolen vector is owned here with no outstanding reference to it.
    let mut all = unsafe { widget.steal_data::<Vec<BoxWatcherGuard>>(WATCHER_GUARDS_DATA_KEY) }
        .unwrap_or_default();
    all.extend(guards);
    // SAFETY: same key/type contract as above; `set_data` takes ownership and
    // drops the accumulated guards when the widget is destroyed.
    unsafe { widget.set_data(WATCHER_GUARDS_DATA_KEY, all) };
}

/// A CSS provider that styles exactly one widget.
///
/// GTK 4.10 removed per-widget style contexts, so a provider can only be
/// registered display-wide. To keep a rule from bleeding into every other widget
/// sharing the same semantic class, the handle mints a CSS class that is unique
/// to the widget instance and emits rules for that class alone. The handle keeps
/// itself alive as widget data, so the display loses the provider exactly when
/// the widget goes away.
#[derive(Debug, Clone)]
pub struct ScopedCss(Rc<ScopedCssProvider>);

#[derive(Debug)]
struct ScopedCssProvider {
    display: gdk::Display,
    provider: CssProvider,
    class: String,
}

impl Drop for ScopedCssProvider {
    fn drop(&mut self) {
        gtk4::style_context_remove_provider_for_display(&self.display, &self.provider);
    }
}

impl ScopedCss {
    /// Registers a provider scoped to `widget` and tags the widget with both
    /// `class` and the instance-unique class that scopes the provider.
    ///
    /// The declaration block starts out empty; call
    /// [`ScopedCss::set_declarations`] to style the widget.
    pub fn attach(widget: &impl IsA<Widget>, class: &str, priority: u32) -> Self {
        let display = widget.display();
        let provider = CssProvider::new();
        gtk4::style_context_add_provider_for_display(&display, &provider, priority);

        // The widget address is unique among live widgets, and this handle dies
        // with the widget, so the scoped class can never collide.
        let scoped_class = format!("{class}-{:x}", widget.as_ref().as_ptr() as usize);
        widget.add_css_class(class);
        widget.add_css_class(&scoped_class);

        let handle = Self(Rc::new(ScopedCssProvider {
            display,
            provider,
            class: scoped_class.clone(),
        }));
        // SAFETY: the scoped class is unique to this widget instance, so the
        // key is written exactly once and never read back; the clone is stored
        // only so destroying the widget drops the handle (removing the
        // display-wide provider), and widget data lives on the GTK main thread.
        unsafe { widget.set_data(&scoped_class, handle.clone()) };
        handle
    }

    /// Applies `declarations` (a CSS declaration block without braces) to the
    /// scoped widget, replacing whatever was applied before.
    pub fn set_declarations(&self, declarations: &str) {
        self.0
            .provider
            .load_from_string(&format!(".{} {{ {declarations} }}", self.0.class));
    }

    /// Drops every declaration previously applied to the scoped widget.
    pub fn clear(&self) {
        self.0.provider.load_from_string("");
    }
}

/// Converts a resolved color to clamped sRGBA byte channels.
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels are clamped to the target range before the cast"
)]
pub fn resolved_color_to_rgba8(color: ResolvedColor) -> (u8, u8, u8, f32) {
    let srgb = color.to_srgb_with_headroom();
    let red = (srgb.red.clamp(0.0, 1.0) * 255.0) as u8;
    let green = (srgb.green.clamp(0.0, 1.0) * 255.0) as u8;
    let blue = (srgb.blue.clamp(0.0, 1.0) * 255.0) as u8;
    let alpha = color.opacity.clamp(0.0, 1.0);
    (red, green, blue, alpha)
}

/// Converts a resolved color to clamped SDR sRGBA float channels in `[0.0, 1.0]`.
///
/// # Panics
///
/// Panics if any channel of the resolved color is not finite.
#[must_use]
pub fn resolved_color_to_srgba_f64(color: ResolvedColor) -> (f64, f64, f64, f64) {
    let srgb = color.to_srgb_with_headroom();
    assert!(
        srgb.red.is_finite(),
        "resolved color red channel must be finite"
    );
    assert!(
        srgb.green.is_finite(),
        "resolved color green channel must be finite"
    );
    assert!(
        srgb.blue.is_finite(),
        "resolved color blue channel must be finite"
    );
    assert!(
        color.opacity.is_finite(),
        "resolved color opacity channel must be finite"
    );
    (
        f64::from(srgb.red.clamp(0.0, 1.0)),
        f64::from(srgb.green.clamp(0.0, 1.0)),
        f64::from(srgb.blue.clamp(0.0, 1.0)),
        f64::from(color.opacity.clamp(0.0, 1.0)),
    )
}

/// Converts a resolved color to `#RRGGBB` format.
#[must_use]
pub fn resolved_color_to_hex(color: ResolvedColor) -> String {
    let (red, green, blue, _) = resolved_color_to_rgba8(color);
    format!("#{red:02X}{green:02X}{blue:02X}")
}

/// Converts a resolved color to CSS `rgba(r, g, b, a)` format.
#[must_use]
pub fn resolved_color_to_css_rgba(color: ResolvedColor) -> String {
    let (red, green, blue, alpha) = resolved_color_to_rgba8(color);
    format!("rgba({red}, {green}, {blue}, {alpha})")
}

fn passthrough_content(view: &AnyView) -> Option<&AnyView> {
    macro_rules! passthrough_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<Metadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    macro_rules! passthrough_ignorable_metadata_content {
        ($($ty:ty),+ $(,)?) => {
            $(
                if let Some(metadata) = view.downcast_ref::<IgnorableMetadata<$ty>>() {
                    return Some(&metadata.content);
                }
            )+
        };
    }

    passthrough_metadata_content!(
        Environment,
        Retain,
        Opacity,
        AppliedFilter,
        Scale,
        Rotation,
        Offset,
        ClipShape,
        Border,
        Shadow,
        Focused,
        Hittable,
        GestureObserver,
        LifeCycleHook,
        OnEvent,
        Secure,
        StandardDynamicRange,
        HighDynamicRange,
        Cursor,
        IgnoreSafeArea,
        ResolvedContextMenu,
        Draggable,
        DropDestination,
        Background,
        NavigationTransitionSource,
        NavigationTransitionDestination
    );
    passthrough_ignorable_metadata_content!(
        MaterialBackground,
        AccessibilityLabel,
        AccessibilityRole,
        AccessibilityHidden,
        AccessibilityChildren,
        AccessibilityState,
        AccessibilityStateSignal
    );

    None
}

/// Returns the stretch axis for layout, recursively unwrapping metadata wrappers.
#[must_use]
pub fn effective_stretch_axis(view: &AnyView) -> StretchAxis {
    if let Some(content) = passthrough_content(view) {
        return effective_stretch_axis(content);
    }
    view.stretch_axis()
}
