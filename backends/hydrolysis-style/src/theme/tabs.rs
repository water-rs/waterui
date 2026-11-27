//! The tabs look of the theme contract (water-rs/waterui#1434): what a theme
//! returns for a tab container ([`TabsLook`]).
//!
//! A tab bar is a surface, a divider on its inner edge and one item per tab.
//! Each item is a [`Chrome`] of its own, which draws the item's interaction
//! feedback, and may carry a highlight, a [`Custom`] part over the item's
//! [`TabItem`] that shows the item's selection. Hydrolysis measures, places,
//! hit-tests and records the bar from this one look, so the geometry it
//! measures is the geometry it draws, and a selection change, a hover or a
//! press moves live operands and records nothing.

use std::rc::Rc;

use kurbo::{Rect, Vec2};
use nami::Computed;
use waterui_graphics::color::Color;
use waterui_navigation::tab::TabStyle;
use waterui_text::font::Font;

use super::Chrome;
use super::look::{Custom, CustomPart, DrivenSet, Motion};

/// What Hydrolysis asks a theme's [`tabs`](super::WidgetTheme::tabs) look
/// for: everything structural about one tab container.
///
/// A spec change requests the look again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabsSpec {
    placement: TabStyle,
}

impl TabsSpec {
    /// The spec of a tab container presented as `placement`.
    #[must_use]
    pub const fn new(placement: TabStyle) -> Self {
        Self { placement }
    }

    /// Whether the items sit in a bar or a sidebar strip; `Automatic` and
    /// `TabBar` are both a bar.
    #[must_use]
    pub const fn placement(&self) -> TabStyle {
        self.placement
    }
}

/// How a tab bar lays out each item's icon and label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each tab item arrangement variant by variant; a new tab item arrangement must fail their build instead of falling into a wildcard arm"
)]
pub enum TabItemLayout {
    /// The icon above the label, the indicator behind the icon: the compact
    /// bar of a narrow window.
    Vertical,
    /// The icon beside the label, the indicator behind both: the bar of a
    /// medium-width window, where items have room to spread.
    Horizontal,
}

/// How a bottom tab bar sizes its width inside the container.
#[derive(Debug, Clone, Copy, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "bottom tab bar sizing is either edge-to-edge full width or content-fitted compact pill"
)]
pub enum TabBarWidth {
    /// Spans the full container width (edge-to-edge docked bar).
    Full,
    /// Sizes the bar to fit its tab items, centered horizontally inside the
    /// container and bounded by `max_width` and available container width.
    Compact {
        /// Minimum width reserved for each tab item slot.
        item_min_width: f32,
        /// Optional maximum width of the bar in logical points.
        max_width: Option<f32>,
        /// Outer horizontal padding inside the bar before the first item and
        /// after the last item.
        horizontal_padding: f32,
    },
}

/// The geometry of one tab item, in the coordinate space of the bar that
/// holds it: what a style needs to draw the selection indicator wherever and
/// in whatever shape it wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabItemGeometry {
    /// The item's chrome bounds: the whole button for vertical items, the
    /// icon and title grown by the button inset for horizontal ones.
    pub bounds: Rect,
    /// The icon's rect, `None` for a title-only item.
    pub icon: Option<Rect>,
    /// The title's rect.
    pub title: Rect,
    /// How the item arranges its icon and title.
    pub layout: TabItemLayout,
    /// Whether the item sits in a bar or a sidebar strip; `Automatic` and
    /// `TabBar` are both a bar.
    pub placement: TabStyle,
}

/// Layout metrics for tab containers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabsMetrics {
    /// Tab bar height.
    pub bar_height: f32,
    /// Minimum width for one tab button.
    pub button_min_width: f32,
    /// Horizontal label inset inside one tab button.
    pub button_horizontal_inset: f32,
    /// Gap between a tab item's icon and its label: vertical space when the
    /// icon sits above the label, horizontal space when it sits beside it.
    pub icon_label_spacing: f32,
    /// Logical point size for tab item icons, installed into each item's
    /// icon environment via `IconSize`.
    pub icon_size: f32,
}

impl TabsMetrics {
    /// Create tab layout metrics.
    #[must_use]
    pub const fn new(
        bar_height: f32,
        button_min_width: f32,
        button_horizontal_inset: f32,
        icon_label_spacing: f32,
    ) -> Self {
        Self {
            bar_height,
            button_min_width,
            button_horizontal_inset,
            icon_label_spacing,
            // Matches `waterui_icon::DEFAULT_ICON_SIZE` while downstream style
            // packages migrate to explicit `icon_size`.
            icon_size: 24.0,
        }
    }
}

/// One tab item as the look's parts see it: the value a highlight follows.
///
/// The item's geometry and position are fixed while the item is laid out
/// where it is; `selected` is live, and a change of it moves the operands of
/// the two items it touches under the look's [`Motion::value`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabItem {
    /// The item's index, counted from the leading one.
    pub index: usize,
    /// How many items the bar has.
    pub count: usize,
    /// Whether this is the selected item.
    pub selected: bool,
    /// Where the item's parts lie.
    pub geometry: TabItemGeometry,
}

/// The whole tab bar's live selection value: what a bar-level selection
/// indicator follows.
#[derive(Debug, Clone, PartialEq)]
pub struct TabsValue {
    /// The index of the selected item, counted from the leading one; `None`
    /// while the selection names no item.
    pub selected: Option<usize>,
    /// Where the selection indicator sits in item indices from the leading
    /// one: `Some(selected as f64)` at rest, between two items while a drag
    /// moves it, and `None` when no item is selected.
    pub position: Option<f64>,
    /// Each item's geometry, leading first.
    pub items: Rc<[TabItemGeometry]>,
}

impl TabsValue {
    /// Interpolated item bounds at [`Self::position`], linearly interpolating
    /// between adjacent items' [`TabItemGeometry::bounds`] while `position`
    /// is fractional, or `None` when no item is selected or the bar is empty.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "tab indices are small non-negative integers far below 2^53"
    )]
    pub fn indicator_bounds(&self) -> Option<Rect> {
        let pos = self.position?;
        if self.items.is_empty() || !pos.is_finite() {
            return None;
        }
        let last = self.items.len() - 1;
        let clamped = pos.clamp(0.0, last as f64);
        let lo = clamped.floor() as usize;
        let hi = (lo + 1).min(last);
        let t = clamped - (lo as f64);
        let a = self.items[lo].bounds;
        let b = self.items[hi].bounds;
        Some(Rect::new(
            (b.x0 - a.x0).mul_add(t, a.x0),
            (b.y0 - a.y0).mul_add(t, a.y0),
            (b.x1 - a.x1).mul_add(t, a.x1),
            (b.y1 - a.y1).mul_add(t, a.y1),
        ))
    }
}

/// How a bar-level tab selection indicator follows a drag along the bar's
/// axis, and where a release settles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TabsDrag;

impl TabsDrag {
    /// A drag along the bar's axis (horizontal for a tab bar, vertical for a
    /// sidebar strip).
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// The indicator's continuous position in item indices for `items` with
    /// `selected` active under `drag`.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "tab item counts and indices are far below 2^53"
    )]
    pub fn position(&self, selected: usize, drag: Option<Vec2>, items: &[TabItemGeometry]) -> f64 {
        if items.is_empty() {
            return 0.0;
        }
        let selected = selected.min(items.len() - 1);
        let rest = selected as f64;
        let Some(drag) = drag else {
            return rest;
        };
        if items.len() == 1 {
            return 0.0;
        }
        let vertical = matches!(items[0].placement, TabStyle::Sidebar);
        let axis = |rect: Rect| {
            if vertical {
                rect.center().y
            } else {
                rect.center().x
            }
        };
        let delta = if vertical { drag.y } else { drag.x };
        let target = axis(items[selected].bounds) + delta;

        // Project `target` onto the piecewise-linear chain of item centers
        // (supporting both increasing LTR/top-to-bottom and decreasing RTL order).
        let first = axis(items[0].bounds);
        let last = axis(items[items.len() - 1].bounds);
        let ascending = last >= first;
        if (ascending && target <= first) || (!ascending && target >= first) {
            return 0.0;
        }
        if (ascending && target >= last) || (!ascending && target <= last) {
            return (items.len() - 1) as f64;
        }
        for i in 0..(items.len() - 1) {
            let a = axis(items[i].bounds);
            let b = axis(items[i + 1].bounds);
            let span = b - a;
            if span.abs() <= 1e-9 {
                continue;
            }
            let between = if ascending {
                target >= a && target <= b
            } else {
                target <= a && target >= b
            };
            if between {
                return (i as f64) + ((target - a) / span).clamp(0.0, 1.0);
            }
        }
        rest
    }

    /// The item index a release settles on with the indicator at `position`
    /// across `count` items: the nearest one.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the rounded index is a small non-negative integer far below usize::MAX"
    )]
    pub fn settles_on(&self, position: f64, count: usize) -> usize {
        let last = count.saturating_sub(1);
        (position.round().max(0.0) as usize).min(last)
    }
}

/// What gives an item its chrome, from the item and its live selected flag.
type ItemChrome = dyn Fn(&TabItem, Computed<bool>) -> Chrome;

/// What gives an item its highlight, from the item.
type ItemHighlight = dyn Fn(&TabItem) -> Rc<dyn CustomPart<TabItem>>;

/// How a tab container looks and responds: the measures it is laid out by,
/// its surface and divider, its items' chrome, an optional bar-level
/// selection indicator, and optional per-item highlights.
///
/// The selected tab's content records before the bar so a translucent bar
/// surface captures the content behind it. The bar's own parts record in this
/// order: the surface, the divider, the bar-level selection indicator (when
/// set with [`Self::selection_indicator`]), then for each item its highlight
/// and its chrome's layers, then the items' icons and labels, and finally any
/// bar-level indicator set with [`Self::selection_indicator_over_labels`].
///
/// An item's chrome and highlight are evaluated once per item while the
/// items lie where they do; the highlight is a [`Custom`] part over the
/// item's [`TabItem`], recorded on [`TabItemGeometry::bounds`]. A bar-level
/// selection indicator is a single [`Custom`] part over [`TabsValue`],
/// recorded once on the bar's surface frame so it glides smoothly across
/// items on selection changes and drags.
#[derive(Clone)]
pub struct TabsLook {
    vertical: TabsMetrics,
    horizontal: Option<(f32, TabsMetrics)>,
    motion: Motion,
    bar: Rc<dyn CustomPart<()>>,
    divider: Rc<dyn CustomPart<()>>,
    item_chrome: Rc<ItemChrome>,
    highlight: Option<Rc<ItemHighlight>>,
    selection_indicator: Option<Rc<dyn CustomPart<TabsValue>>>,
    selection_indicator_over_labels: Option<Rc<dyn CustomPart<TabsValue>>>,
    drag: Option<TabsDrag>,
    bar_width: TabBarWidth,
    floating_margin: f32,
    selected_foreground: Option<Color>,
    unselected_foreground: Option<Color>,
    label_font: Option<Font>,
}

impl core::fmt::Debug for TabsLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabsLook")
            .field("vertical", &self.vertical)
            .field("horizontal", &self.horizontal)
            .field("motion", &self.motion)
            .field("highlight", &self.highlight.is_some())
            .field("selection_indicator", &self.selection_indicator.is_some())
            .field(
                "selection_indicator_over_labels",
                &self.selection_indicator_over_labels.is_some(),
            )
            .field("drag", &self.drag)
            .field("bar_width", &self.bar_width)
            .field("floating_margin", &self.floating_margin)
            .field("selected_foreground", &self.selected_foreground.is_some())
            .field(
                "unselected_foreground",
                &self.unselected_foreground.is_some(),
            )
            .field("label_font", &self.label_font)
            .finish_non_exhaustive()
    }
}

impl TabsLook {
    /// A tab container whose items are laid out as `vertical` measures,
    /// with `bar` drawn over the bar's surface and `divider` over its
    /// inner edge, each item wearing the chrome `item_chrome` gives it and
    /// its values running with `motion`. It has no highlight and lays its
    /// items out vertically at every width.
    #[must_use]
    pub fn new<B, D>(
        vertical: TabsMetrics,
        bar: Custom<(), B>,
        divider: Custom<(), D>,
        motion: Motion,
        item_chrome: impl Fn(&TabItem, Computed<bool>) -> Chrome + 'static,
    ) -> Self
    where
        B: DrivenSet<()> + 'static,
        B::Live: 'static,
        D: DrivenSet<()> + 'static,
        D::Live: 'static,
    {
        Self {
            vertical,
            horizontal: None,
            motion,
            bar: Rc::new(bar),
            divider: Rc::new(divider),
            item_chrome: Rc::new(item_chrome),
            highlight: None,
            selection_indicator: None,
            selection_indicator_over_labels: None,
            drag: None,
            bar_width: TabBarWidth::Full,
            floating_margin: 0.0,
            selected_foreground: None,
            unselected_foreground: None,
            label_font: None,
        }
    }

    /// Lays the items out horizontally, as `metrics`, in a bar at least
    /// `from_extent` wide: a width class of the bar's own extent, which for
    /// a sidebar strip is its thickness.
    #[must_use]
    pub const fn horizontal(mut self, from_extent: f32, metrics: TabsMetrics) -> Self {
        self.horizontal = Some((from_extent, metrics));
        self
    }

    /// Draws each item's highlight with the part `highlight` gives it,
    /// recorded over the item's bounds beneath the item's chrome. The part
    /// follows the item's [`TabItem::selected`] through its
    /// [`Driven`](super::Driven) values, so a selection change animates it
    /// under the look's motion.
    #[must_use]
    pub fn highlight<D>(
        mut self,
        highlight: impl Fn(&TabItem) -> Custom<TabItem, D> + 'static,
    ) -> Self
    where
        D: DrivenSet<TabItem> + 'static,
        D::Live: 'static,
    {
        self.highlight = Some(Rc::new(move |item| Rc::new(highlight(item))));
        self
    }

    /// Draws a single bar-level selection indicator below the items' icons
    /// and labels, recorded once over the bar's surface frame.
    ///
    /// Its [`Driven`](super::Driven) values follow [`TabsValue`] — the
    /// selected tab index, continuous indicator position, and every item's
    /// geometry — so selection changes glide the indicator across item
    /// frames under [`Self::motion`]'s value motion with zero re-recording.
    #[must_use]
    pub fn selection_indicator<D>(mut self, indicator: Custom<TabsValue, D>) -> Self
    where
        D: DrivenSet<TabsValue> + 'static,
        D::Live: 'static,
    {
        self.selection_indicator = Some(Rc::new(indicator));
        self
    }

    /// Draws a single bar-level selection indicator over the items' icons
    /// and labels, recorded once over the bar's surface frame. A backdrop
    /// material recorded here captures the tab labels underneath.
    #[must_use]
    pub fn selection_indicator_over_labels<D>(mut self, indicator: Custom<TabsValue, D>) -> Self
    where
        D: DrivenSet<TabsValue> + 'static,
        D::Live: 'static,
    {
        self.selection_indicator_over_labels = Some(Rc::new(indicator));
        self
    }

    /// Lets the bar-level selection indicator follow a pointer drag that
    /// starts on the selected tab, settling on the nearest tab at release.
    /// A look with no drag takes taps only.
    #[must_use]
    pub const fn drag(mut self, drag: TabsDrag) -> Self {
        self.drag = Some(drag);
        self
    }

    /// Configures how a bottom tab bar sizes its width (`Full` edge-to-edge
    /// by default, or centered `Compact` fitted to its items). Sidebar strips
    /// keep their fixed thickness.
    #[must_use]
    pub const fn bar_width(mut self, width: TabBarWidth) -> Self {
        self.bar_width = width;
        self
    }

    /// Sets an outer margin around a floating bottom tab bar in logical
    /// points (reserving bottom clearance and inset from horizontal container
    /// edges). Defaults to `0.0` (flush docked bar). Sidebar strips ignore
    /// this margin.
    #[must_use]
    pub const fn floating_margin(mut self, margin: f32) -> Self {
        self.floating_margin = margin;
        self
    }

    /// Overrides the `Foreground` token used by the selected tab item's icon
    /// and label. `None` keeps the surrounding `Foreground`.
    #[must_use]
    pub fn selected_foreground(mut self, color: impl Into<Color>) -> Self {
        self.selected_foreground = Some(color.into());
        self
    }

    /// Overrides the `Foreground` token used by unselected tab items' icons
    /// and labels. `None` keeps the surrounding `Foreground`.
    #[must_use]
    pub fn unselected_foreground(mut self, color: impl Into<Color>) -> Self {
        self.unselected_foreground = Some(color.into());
        self
    }

    /// Overrides the typography used for tab item labels (`Body` slot),
    /// leaving labels that explicitly specify their own slot untouched.
    #[must_use]
    pub fn label_font(mut self, font: impl Into<Font>) -> Self {
        self.label_font = Some(font.into());
        self
    }

    /// How a bar `extent` wide lays its items out.
    #[must_use]
    pub fn item_layout(&self, extent: f32) -> TabItemLayout {
        match self.horizontal {
            Some((from, _)) if extent >= from => TabItemLayout::Horizontal,
            _ => TabItemLayout::Vertical,
        }
    }

    /// The measures of items laid out as `layout`.
    #[must_use]
    pub const fn metrics(&self, layout: TabItemLayout) -> TabsMetrics {
        match (layout, self.horizontal) {
            (TabItemLayout::Horizontal, Some((_, metrics))) => metrics,
            _ => self.vertical,
        }
    }

    /// The motion the look's values run with.
    #[must_use]
    pub const fn motion(&self) -> &Motion {
        &self.motion
    }

    /// The drag configuration, when the selection indicator follows a drag.
    #[must_use]
    pub const fn get_drag(&self) -> Option<TabsDrag> {
        self.drag
    }

    /// How a bottom tab bar sizes its width.
    #[must_use]
    pub const fn get_bar_width(&self) -> TabBarWidth {
        self.bar_width
    }

    /// Outer margin around a floating bottom tab bar in logical points.
    #[must_use]
    pub const fn get_floating_margin(&self) -> f32 {
        self.floating_margin
    }

    /// Selected tab item foreground override, if configured.
    #[must_use]
    pub const fn get_selected_foreground(&self) -> Option<&Color> {
        self.selected_foreground.as_ref()
    }

    /// Unselected tab item foreground override, if configured.
    #[must_use]
    pub const fn get_unselected_foreground(&self) -> Option<&Color> {
        self.unselected_foreground.as_ref()
    }

    /// Tab item label font override, if configured.
    #[must_use]
    pub const fn get_label_font(&self) -> Option<&Font> {
        self.label_font.as_ref()
    }

    /// The chrome of `item`, given its live selected flag.
    #[doc(hidden)]
    #[must_use]
    pub fn item_chrome(&self, item: &TabItem, selected: Computed<bool>) -> Chrome {
        (self.item_chrome)(item, selected)
    }

    /// The highlight of `item`, when the look has one.
    #[doc(hidden)]
    #[must_use]
    pub fn item_highlight(&self, item: &TabItem) -> Option<Rc<dyn CustomPart<TabItem>>> {
        self.highlight.as_ref().map(|highlight| highlight(item))
    }

    /// The bar-level selection indicator beneath the items' labels, if set.
    #[doc(hidden)]
    #[must_use]
    pub fn get_selection_indicator(&self) -> Option<&Rc<dyn CustomPart<TabsValue>>> {
        self.selection_indicator.as_ref()
    }

    /// The bar-level selection indicator over the items' labels, if set.
    #[doc(hidden)]
    #[must_use]
    pub fn get_selection_indicator_over_labels(&self) -> Option<&Rc<dyn CustomPart<TabsValue>>> {
        self.selection_indicator_over_labels.as_ref()
    }

    /// The bar's surface part.
    #[doc(hidden)]
    #[must_use]
    pub fn bar(&self) -> &Rc<dyn CustomPart<()>> {
        &self.bar
    }

    /// The bar's divider part.
    #[doc(hidden)]
    #[must_use]
    pub fn divider(&self) -> &Rc<dyn CustomPart<()>> {
        &self.divider
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use approx::assert_relative_eq;
    use kurbo::{Rect, Vec2};
    use waterui_navigation::tab::TabStyle;

    use super::{TabItemGeometry, TabItemLayout, TabsDrag, TabsValue};

    fn item(bounds: Rect, placement: TabStyle) -> TabItemGeometry {
        TabItemGeometry {
            bounds,
            icon: None,
            title: bounds,
            layout: TabItemLayout::Vertical,
            placement,
        }
    }

    #[test]
    fn tabs_value_interpolates_indicator_bounds_across_items() {
        let items: Rc<[TabItemGeometry]> = Rc::from([
            item(Rect::new(0.0, 0.0, 80.0, 48.0), TabStyle::TabBar),
            item(Rect::new(80.0, 0.0, 180.0, 48.0), TabStyle::TabBar),
            item(Rect::new(180.0, 0.0, 240.0, 48.0), TabStyle::TabBar),
        ]);
        let rest = TabsValue {
            selected: Some(1),
            position: Some(1.0),
            items: Rc::clone(&items),
        };
        assert_eq!(rest.indicator_bounds(), Some(items[1].bounds));

        let mid = TabsValue {
            selected: Some(0),
            position: Some(0.5),
            items,
        };
        let bounds = mid.indicator_bounds().expect("fractional bounds exist");
        assert_relative_eq!(bounds.x0, 40.0, epsilon = 1e-12);
        assert_relative_eq!(bounds.x1, 130.0, epsilon = 1e-12);
    }

    #[test]
    fn tabs_drag_tracks_horizontal_rtl_and_sidebar_axes() {
        let drag = TabsDrag::new();
        let horizontal = [
            item(Rect::new(0.0, 0.0, 100.0, 48.0), TabStyle::TabBar),
            item(Rect::new(100.0, 0.0, 200.0, 48.0), TabStyle::TabBar),
            item(Rect::new(200.0, 0.0, 300.0, 48.0), TabStyle::TabBar),
        ];
        assert_relative_eq!(
            drag.position(0, Some(Vec2::new(50.0, 12.0)), &horizontal),
            0.5
        );
        assert_relative_eq!(
            drag.position(1, Some(Vec2::new(500.0, 0.0)), &horizontal),
            2.0
        );
        assert_eq!(drag.settles_on(1.4, 3), 1);
        assert_eq!(drag.settles_on(1.6, 3), 2);

        let rtl = [
            item(Rect::new(200.0, 0.0, 300.0, 48.0), TabStyle::TabBar),
            item(Rect::new(100.0, 0.0, 200.0, 48.0), TabStyle::TabBar),
            item(Rect::new(0.0, 0.0, 100.0, 48.0), TabStyle::TabBar),
        ];
        assert_relative_eq!(drag.position(0, Some(Vec2::new(-50.0, 0.0)), &rtl), 0.5);

        let sidebar = [
            item(Rect::new(0.0, 0.0, 80.0, 60.0), TabStyle::Sidebar),
            item(Rect::new(0.0, 60.0, 80.0, 120.0), TabStyle::Sidebar),
        ];
        assert_relative_eq!(drag.position(0, Some(Vec2::new(0.0, 30.0)), &sidebar), 0.5);
    }

    #[test]
    fn tabs_look_label_font_defaults_to_none_and_stores_override() {
        use nami::Signal;
        use std::time::Duration;
        use waterui_core::Environment;
        use waterui_graphics::color::Srgb;
        use waterui_text::font::{Body, Font, FontWeight};

        use super::super::{
            Chrome, CornerShape, Custom, FocusIndicator, Motion, PressFeedback, StateLayer,
            Surface, TabsLook, TabsMetrics,
        };

        let metrics = TabsMetrics {
            bar_height: 48.0,
            button_min_width: 64.0,
            button_horizontal_inset: 12.0,
            icon_label_spacing: 4.0,
            icon_size: 24.0,
        };
        let motion = Motion::uniform(waterui_core::animation::Animation::linear(
            Duration::from_millis(10),
        ));
        let chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Srgb::WHITE),
            PressFeedback::new(),
            FocusIndicator::Tint,
            motion.clone(),
        );
        let base = TabsLook::new(
            metrics,
            Custom::new((), |_recorder, _frame, ()| {}),
            Custom::new((), |_recorder, _frame, ()| {}),
            motion,
            move |_item, _selected| chrome.clone(),
        );
        assert!(base.get_label_font().is_none());

        let with_font =
            base.label_font(Font::from(Body).font_size(10.0).weight(FontWeight::Medium));
        let env = Environment::new()
            .store::<Body, nami::Computed<waterui_text::font::ResolvedFont>>(
                nami::Computed::constant(waterui_text::font::ResolvedFont::new(
                    17.0,
                    FontWeight::Normal,
                )),
            );
        let resolved = with_font
            .get_label_font()
            .expect("label_font override must be present")
            .resolve(&env)
            .snapshot();
        assert_relative_eq!(resolved.size(), 10.0, epsilon = 1e-6);
        assert_eq!(resolved.weight(), FontWeight::Medium);
    }

    #[test]
    fn tabs_metrics_icon_size_defaults_and_varies_across_layouts() {
        use std::time::Duration;
        use waterui_graphics::color::Srgb;

        use super::super::{
            Chrome, CornerShape, Custom, FocusIndicator, Motion, PressFeedback, StateLayer,
            Surface, TabsLook, TabsMetrics,
        };

        let legacy = TabsMetrics::new(48.0, 64.0, 12.0, 4.0);
        assert_relative_eq!(legacy.icon_size, 24.0, epsilon = 1e-6);

        let vertical = TabsMetrics {
            bar_height: 62.0,
            button_min_width: 64.0,
            button_horizontal_inset: 12.0,
            icon_label_spacing: 1.5,
            icon_size: 23.0,
        };
        let horizontal = TabsMetrics {
            bar_height: 48.0,
            button_min_width: 64.0,
            button_horizontal_inset: 16.0,
            icon_label_spacing: 8.0,
            icon_size: 18.0,
        };
        let motion = Motion::uniform(waterui_core::animation::Animation::linear(
            Duration::from_millis(10),
        ));
        let chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Srgb::WHITE),
            PressFeedback::new(),
            FocusIndicator::Tint,
            motion.clone(),
        );
        let look = TabsLook::new(
            vertical,
            Custom::new((), |_recorder, _frame, ()| {}),
            Custom::new((), |_recorder, _frame, ()| {}),
            motion,
            move |_item, _selected| chrome.clone(),
        )
        .horizontal(600.0, horizontal);

        assert_relative_eq!(
            look.metrics(TabItemLayout::Vertical).icon_size,
            23.0,
            epsilon = 1e-6
        );
        assert_relative_eq!(
            look.metrics(TabItemLayout::Horizontal).icon_size,
            18.0,
            epsilon = 1e-6
        );
    }
}
