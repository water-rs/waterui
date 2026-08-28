//! Material Design 3 navigation rail composed from `WaterUI` primitives.
//!
//! The rail is the navigation bar's counterpart on wide screens: the same
//! destinations, stood up along the leading edge. It has two forms — collapsed
//! to a column of icons, or expanded to a list wide enough for labels beside
//! them — and the item is the same component in both, which is why the layout
//! is an attribute of the rail rather than two separate item types.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::reactive::SignalExt as _;
use waterui::shape::{Capsule, ShapeExt as _};
use waterui::{Binding, Environment, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};
use waterui_core::view::TupleViews;

use crate::color::{
    OnSecondaryContainer, OnSurface, OnSurfaceVariant, SecondaryContainer, Surface,
};
use crate::semantics::{conditional_color, interaction_style};
use crate::theme::typography;

/// `NavigationRailCollapsedTokens.ContainerWidth`.
const COLLAPSED_CONTAINER_WIDTH: f32 = 96.0;
/// `NavigationRailCollapsedTokens.NarrowContainerWidth`.
const COLLAPSED_NARROW_CONTAINER_WIDTH: f32 = 80.0;
/// `NavigationRailCollapsedTokens.TopSpace`, shared with the expanded rail.
const TOP_SPACE: f32 = 44.0;
/// `NavigationRailCollapsedTokens.ItemVerticalSpace`.
const COLLAPSED_ITEM_VERTICAL_SPACE: f32 = 4.0;
/// `NavigationRailExpandedTokens.ContainerWidthMinimum`.
const EXPANDED_CONTAINER_WIDTH_MINIMUM: f32 = 220.0;
/// `NavigationRailExpandedTokens.ContainerWidthMaximum`.
const EXPANDED_CONTAINER_WIDTH_MAXIMUM: f32 = 360.0;
/// `NavigationRailBaselineItemTokens.ContainerHeight`.
const ITEM_HEIGHT: f32 = 64.0;
/// `NavigationRailBaselineItemTokens.ContainerVerticalSpace`.
const ITEM_VERTICAL_SPACE: f32 = 6.0;
/// `NavigationRailBaselineItemTokens.IconSize`.
const ITEM_ICON_SIZE: f32 = 24.0;
/// `NavigationRailBaselineItemTokens.ActiveIndicatorLeadingSpace` /
/// `ActiveIndicatorTrailingSpace`.
const ACTIVE_INDICATOR_HORIZONTAL_SPACE: f32 = 16.0;
/// `NavigationRailBaselineItemTokens.ActiveIndicatorIconLabelSpace`.
const ACTIVE_INDICATOR_ICON_LABEL_SPACE: f32 = 8.0;

/// How wide the rail stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum NavigationRailLayout {
    /// A column of icons with their labels beneath, at
    /// `NavigationRailCollapsedTokens.ContainerWidth`.
    #[default]
    Collapsed,
    /// A narrower collapsed rail, at `NarrowContainerWidth`.
    CollapsedNarrow,
    /// Labels beside their icons, between the expanded rail's minimum and
    /// maximum widths.
    Expanded,
}

impl NavigationRailLayout {
    /// Whether items lay their label beside the icon rather than beneath it.
    #[must_use]
    pub const fn is_expanded(self) -> bool {
        matches!(self, Self::Expanded)
    }

    /// The rail's resting width.
    #[must_use]
    pub const fn container_width(self) -> f32 {
        match self {
            Self::Collapsed => COLLAPSED_CONTAINER_WIDTH,
            Self::CollapsedNarrow => COLLAPSED_NARROW_CONTAINER_WIDTH,
            Self::Expanded => EXPANDED_CONTAINER_WIDTH_MINIMUM,
        }
    }

    /// The widest the rail may grow.
    ///
    /// A collapsed rail is a fixed column, so its maximum is its width. The
    /// expanded rail is a range: wide enough for labels, bounded so it does not
    /// become a sidebar.
    #[must_use]
    pub const fn max_container_width(self) -> f32 {
        match self {
            Self::Collapsed => COLLAPSED_CONTAINER_WIDTH,
            Self::CollapsedNarrow => COLLAPSED_NARROW_CONTAINER_WIDTH,
            Self::Expanded => EXPANDED_CONTAINER_WIDTH_MAXIMUM,
        }
    }

    /// Vertical gap between items.
    ///
    /// The collapsed rail packs items tighter than the baseline item spacing,
    /// because a stacked icon and label already carry their own breathing room.
    #[must_use]
    pub const fn item_spacing(self) -> f32 {
        match self {
            Self::Collapsed | Self::CollapsedNarrow => COLLAPSED_ITEM_VERTICAL_SPACE,
            Self::Expanded => ITEM_VERTICAL_SPACE,
        }
    }
}

/// A Material Design 3 navigation rail.
pub struct NavigationRail<Items> {
    items: Items,
    layout: NavigationRailLayout,
}

impl<Items> Debug for NavigationRail<Items> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationRail")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl<Items> NavigationRail<Items> {
    /// Creates a navigation rail containing Material rail items.
    #[must_use]
    pub const fn new(items: Items) -> Self {
        Self {
            items,
            layout: NavigationRailLayout::Collapsed,
        }
    }

    /// Sets how wide the rail stands.
    #[must_use]
    pub const fn layout(mut self, layout: NavigationRailLayout) -> Self {
        self.layout = layout;
        self
    }
}

impl<Items> View for NavigationRail<Items>
where
    Items: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let layout = self.layout;
        waterui::component::vstack(self.items)
            .spacing(layout.item_spacing())
            .padding_with(EdgeInsets::new(TOP_SPACE, 0.0, 0.0, 0.0))
            .min_width(layout.container_width())
            .max_width(layout.max_container_width())
            .max_height(f32::MAX)
            .background(Surface)
            .a11y_label("Navigation")
            .a11y_role(AccessibilityRole::TabList)
    }
}

/// One destination in a [`NavigationRail`].
pub struct NavigationRailItem<Icon, Action = fn(&Environment)> {
    label: Label,
    accessibility_label: Str,
    icon: Icon,
    selected: Binding<bool>,
    layout: NavigationRailLayout,
    action: Action,
}

impl<Icon, Action> Debug for NavigationRailItem<Icon, Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationRailItem")
            .field("label", &self.label)
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl<Icon> NavigationRailItem<Icon> {
    /// Creates a rail item.
    #[must_use]
    pub fn new(label: impl IntoLabel, icon: Icon, selected: &Binding<bool>) -> Self {
        let label = label.into_label();
        let accessibility_label = crate::semantics::label_plain_text(&label);
        Self {
            label,
            accessibility_label,
            icon,
            selected: selected.clone(),
            layout: NavigationRailLayout::Collapsed,
            action: noop,
        }
    }
}

impl<Icon, Action> NavigationRailItem<Icon, Action> {
    /// Matches the item to the rail's layout.
    #[must_use]
    pub const fn layout(mut self, layout: NavigationRailLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Sets the action performed when the item is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> NavigationRailItem<Icon, impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        NavigationRailItem {
            label: self.label,
            accessibility_label: self.accessibility_label,
            icon: self.icon,
            selected: self.selected,
            layout: self.layout,
            action: boxed_action(action),
        }
    }
}

impl<Icon, Action> View for NavigationRailItem<Icon, Action>
where
    Icon: Clone + View + 'static,
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let accessibility_label = self.accessibility_label.clone();
        let accessibility_state = self
            .selected
            .map(|selected| AccessibilityState::new().selected(selected));
        let indicator_color = conditional_color(
            self.selected.clone(),
            SecondaryContainer,
            Color::transparent(),
        );
        let icon_color = conditional_color(
            self.selected.clone(),
            OnSecondaryContainer,
            OnSurfaceVariant,
        );
        let label_color = conditional_color(self.selected.clone(), OnSurface, OnSurfaceVariant);
        let state_layer_color =
            conditional_color(self.selected, OnSecondaryContainer, OnSurfaceVariant);

        let icon = self
            .icon
            .foreground(icon_color)
            .width(ITEM_ICON_SIZE)
            .height(ITEM_ICON_SIZE);
        let label = self
            .label
            .font(typography::label_medium())
            .foreground(label_color);

        // Expanded lays the label beside the icon inside one pill; collapsed
        // stacks it beneath, so only the icon sits on the active indicator.
        let content = if self.layout.is_expanded() {
            waterui::component::hstack((icon, label))
                .spacing(ACTIVE_INDICATOR_ICON_LABEL_SPACE)
                .padding_with(EdgeInsets::new(
                    0.0,
                    0.0,
                    ACTIVE_INDICATOR_HORIZONTAL_SPACE,
                    ACTIVE_INDICATOR_HORIZONTAL_SPACE,
                ))
                .height(ITEM_HEIGHT)
                .background(Capsule.fill(indicator_color))
                .anyview()
        } else {
            waterui::component::vstack((
                waterui::component::zstack((
                    Capsule
                        .fill(indicator_color)
                        .width(COLLAPSED_NARROW_CONTAINER_WIDTH / 2.0)
                        .height(ITEM_ICON_SIZE + ACTIVE_INDICATOR_ICON_LABEL_SPACE),
                    icon,
                )),
                label,
            ))
            .spacing(ACTIVE_INDICATOR_ICON_LABEL_SPACE)
            .height(ITEM_HEIGHT)
            .anyview()
        };

        content
            .max_width(f32::INFINITY)
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(accessibility_label)
            .a11y_role(AccessibilityRole::Tab)
            .a11y_state_signal(accessibility_state)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                state_layer_color,
                f64::from(ITEM_HEIGHT / 2.0),
            ))
    }
}

const fn noop(_env: &Environment) {}

/// Creates a Material Design 3 navigation rail.
#[must_use]
pub const fn navigation_rail<Items>(items: Items) -> NavigationRail<Items> {
    NavigationRail::new(items)
}

/// Creates a Material Design 3 navigation rail item.
#[must_use]
pub fn navigation_rail_item<Icon>(
    label: impl IntoLabel,
    icon: Icon,
    selected: &Binding<bool>,
) -> NavigationRailItem<Icon> {
    NavigationRailItem::new(label, icon, selected)
}

#[cfg(test)]
mod tests {
    use super::{
        ACTIVE_INDICATOR_HORIZONTAL_SPACE, ACTIVE_INDICATOR_ICON_LABEL_SPACE,
        COLLAPSED_CONTAINER_WIDTH, COLLAPSED_ITEM_VERTICAL_SPACE, COLLAPSED_NARROW_CONTAINER_WIDTH,
        EXPANDED_CONTAINER_WIDTH_MAXIMUM, EXPANDED_CONTAINER_WIDTH_MINIMUM, ITEM_HEIGHT,
        ITEM_ICON_SIZE, ITEM_VERTICAL_SPACE, NavigationRailLayout, TOP_SPACE,
    };

    /// Values from `NavigationRailCollapsedTokens`, `NavigationRailExpandedTokens`
    /// and `NavigationRailBaselineItemTokens`.
    #[test]
    fn navigation_rail_tokens_match_compose_navigation_rail_tokens() {
        assert_eq!(COLLAPSED_CONTAINER_WIDTH, 96.0);
        assert_eq!(COLLAPSED_NARROW_CONTAINER_WIDTH, 80.0);
        assert_eq!(TOP_SPACE, 44.0);
        assert_eq!(COLLAPSED_ITEM_VERTICAL_SPACE, 4.0);
        assert_eq!(EXPANDED_CONTAINER_WIDTH_MINIMUM, 220.0);
        assert_eq!(EXPANDED_CONTAINER_WIDTH_MAXIMUM, 360.0);
        assert_eq!(ITEM_HEIGHT, 64.0);
        assert_eq!(ITEM_VERTICAL_SPACE, 6.0);
        assert_eq!(ITEM_ICON_SIZE, 24.0);
        assert_eq!(ACTIVE_INDICATOR_HORIZONTAL_SPACE, 16.0);
        assert_eq!(ACTIVE_INDICATOR_ICON_LABEL_SPACE, 8.0);
    }

    /// Each layout reports its own width, and only the expanded one puts the
    /// label beside the icon.
    #[test]
    fn each_layout_reports_its_own_geometry() {
        assert!(!NavigationRailLayout::Collapsed.is_expanded());
        assert!(!NavigationRailLayout::CollapsedNarrow.is_expanded());
        assert!(NavigationRailLayout::Expanded.is_expanded());

        assert_eq!(
            NavigationRailLayout::Collapsed.container_width(),
            COLLAPSED_CONTAINER_WIDTH
        );
        assert_eq!(
            NavigationRailLayout::CollapsedNarrow.container_width(),
            COLLAPSED_NARROW_CONTAINER_WIDTH
        );
        assert_eq!(
            NavigationRailLayout::Expanded.container_width(),
            EXPANDED_CONTAINER_WIDTH_MINIMUM
        );

        // A rail wide enough for labels must be wider than one that is not.
        assert!(
            NavigationRailLayout::Expanded.container_width()
                > NavigationRailLayout::Collapsed.container_width()
        );
        // A collapsed rail is a fixed column; only the expanded one is a range.
        for layout in [
            NavigationRailLayout::Collapsed,
            NavigationRailLayout::CollapsedNarrow,
        ] {
            assert_eq!(layout.container_width(), layout.max_container_width());
        }
        assert_eq!(
            NavigationRailLayout::Expanded.max_container_width(),
            EXPANDED_CONTAINER_WIDTH_MAXIMUM
        );
    }
}
