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

use kurbo::Rect;
use nami::Computed;
use waterui::navigation::tab::NativeTabStyle;

use super::Chrome;
use super::look::{Custom, CustomPart, DrivenSet, Motion};

/// What Hydrolysis asks a theme's [`tabs`](super::WidgetTheme::tabs) look
/// for: everything structural about one tab container.
///
/// A spec change requests the look again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabsSpec {
    placement: NativeTabStyle,
}

impl TabsSpec {
    /// The spec of a tab container presented as `placement`.
    #[must_use]
    pub const fn new(placement: NativeTabStyle) -> Self {
        Self { placement }
    }

    /// Whether the items sit in a bar or a sidebar strip; `Automatic` and
    /// `TabBar` are both a bar.
    #[must_use]
    pub const fn placement(&self) -> NativeTabStyle {
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
    pub placement: NativeTabStyle,
}

/// Layout metrics for tab containers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TabsMetrics {
    /// Tab bar height.
    pub bar_height: f64,
    /// Minimum width for one tab button.
    pub button_min_width: f64,
    /// Horizontal label inset inside one tab button.
    pub button_horizontal_inset: f64,
    /// Gap between a tab item's icon and its label: vertical space when the
    /// icon sits above the label, horizontal space when it sits beside it.
    pub icon_label_spacing: f64,
}

impl TabsMetrics {
    /// Create tab layout metrics.
    #[must_use]
    pub const fn new(
        bar_height: f64,
        button_min_width: f64,
        button_horizontal_inset: f64,
        icon_label_spacing: f64,
    ) -> Self {
        Self {
            bar_height,
            button_min_width,
            button_horizontal_inset,
            icon_label_spacing,
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

/// What gives an item its chrome, from the item and its live selected flag.
type ItemChrome = dyn Fn(&TabItem, Computed<bool>) -> Chrome;

/// What gives an item its highlight, from the item.
type ItemHighlight = dyn Fn(&TabItem) -> Rc<dyn CustomPart<TabItem>>;

/// How a tab container looks and responds: the measures it is laid out by,
/// its surface and divider, its items' chrome and the highlight of the
/// selected item.
///
/// The parts record in this order: the surface, the divider, then for each
/// item its highlight and its chrome's layers. The items' icons and labels
/// draw above all of it.
///
/// An item's chrome and highlight are evaluated once per item while the
/// items lie where they do; the highlight is a [`Custom`] part over the
/// item's [`TabItem`], recorded on [`TabItemGeometry::bounds`].
#[derive(Clone)]
pub struct TabsLook {
    vertical: TabsMetrics,
    horizontal: Option<(f64, TabsMetrics)>,
    motion: Motion,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the bar's surface; a build without a painter compiles the theme contract alone"
        )
    )]
    bar: Rc<dyn CustomPart<()>>,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer records the bar's divider; a build without a painter compiles the theme contract alone"
        )
    )]
    divider: Rc<dyn CustomPart<()>>,
    #[cfg_attr(
        not(hydrolysis_runtime),
        expect(
            dead_code,
            reason = "only the renderer realizes the items' chrome; a build without a painter compiles the theme contract alone"
        )
    )]
    item_chrome: Rc<ItemChrome>,
    highlight: Option<Rc<ItemHighlight>>,
}

impl core::fmt::Debug for TabsLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TabsLook")
            .field("vertical", &self.vertical)
            .field("horizontal", &self.horizontal)
            .field("motion", &self.motion)
            .field("highlight", &self.highlight.is_some())
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
        }
    }

    /// Lays the items out horizontally, as `metrics`, in a bar at least
    /// `from_extent` wide: a width class of the bar's own extent, which for
    /// a sidebar strip is its thickness.
    #[must_use]
    pub const fn horizontal(mut self, from_extent: f64, metrics: TabsMetrics) -> Self {
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

    /// How a bar `extent` wide lays its items out.
    #[must_use]
    pub fn item_layout(&self, extent: f64) -> TabItemLayout {
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

    /// The chrome of `item`, given its live selected flag.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn item_chrome(&self, item: &TabItem, selected: Computed<bool>) -> Chrome {
        (self.item_chrome)(item, selected)
    }

    /// The highlight of `item`, when the look has one.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn item_highlight(&self, item: &TabItem) -> Option<Rc<dyn CustomPart<TabItem>>> {
        self.highlight.as_ref().map(|highlight| highlight(item))
    }

    /// The bar's surface part.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn bar(&self) -> &Rc<dyn CustomPart<()>> {
        &self.bar
    }

    /// The bar's divider part.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn divider(&self) -> &Rc<dyn CustomPart<()>> {
        &self.divider
    }
}
