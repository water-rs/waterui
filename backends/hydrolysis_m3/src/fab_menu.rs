//! Material Design 3 floating action button menu composed from `WaterUI`
//! primitives.
//!
//! Tapping the toggle expands a column of labelled actions above it. The items
//! reveal from the bottom up rather than all at once, which is what stops a
//! long menu appearing as a wall and keeps the eye travelling away from the
//! button that produced it.

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::layout::padding::EdgeInsets;
use waterui::reactive::{Binding, Computed, SignalExt as _};
use waterui::{AnyView, Environment, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{BoxedAction, Handler, boxed_action};

use crate::color::{OnPrimaryContainer, PrimaryContainer};
use crate::elevation::MaterialElevationLevel;
use crate::semantics::interaction_style;
use crate::theme::motion;

/// `FabMenuBaselineTokens.ListItemContainerHeight`.
const ITEM_HEIGHT: f32 = 56.0;
/// `FabMenuBaselineTokens.ListItemLeadingSpace` / `ListItemTrailingSpace`.
const ITEM_HORIZONTAL_SPACE: f32 = 24.0;
/// `FabMenuBaselineTokens.ListItemIconLabelSpace`.
const ITEM_ICON_LABEL_SPACE: f32 = 8.0;
/// `FabMenuBaselineTokens.ListItemIconSize`.
const ITEM_ICON_SIZE: f32 = 24.0;
/// `FabMenuBaselineTokens.ListItemBetweenSpace`.
const ITEM_BETWEEN_SPACE: f32 = 4.0;
/// `FabMenuBaselineTokens.CloseButtonBetweenSpace`: the gap between the last
/// item and the toggle.
const CLOSE_BUTTON_BETWEEN_SPACE: f32 = 8.0;
/// `FabMenuBaselineTokens.CloseButtonContainerHeight` / `ContainerWidth`.
const CLOSE_BUTTON_CONTAINER: f32 = 56.0;
/// `FabMenuBaselineTokens.CloseButtonIconSize`.
const CLOSE_BUTTON_ICON_SIZE: f32 = 20.0;

/// Fraction of the expansion each item's own fade occupies. The remainder is
/// what the items stagger across, so a longer menu spreads rather than
/// speeding up.
const ITEM_FADE_FRACTION: f32 = 0.5;

/// One action in a [`FloatingActionButtonMenu`].
pub struct FabMenuItem {
    label: Label,
    icon: AnyView,
    action: BoxedAction<()>,
}

impl core::fmt::Debug for FabMenuItem {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FabMenuItem")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl FabMenuItem {
    /// Creates a menu item.
    #[must_use]
    pub fn new<F, Args>(label: impl IntoLabel, icon: impl View + 'static, action: F) -> Self
    where
        F: Handler<Args, ()> + 'static,
    {
        Self {
            label: label.into_label(),
            icon: AnyView::new(icon),
            action: Box::new(boxed_action(action)),
        }
    }
}

/// Creates a floating action button menu item.
#[must_use]
pub fn fab_menu_item<F, Args>(
    label: impl IntoLabel,
    icon: impl View + 'static,
    action: F,
) -> FabMenuItem
where
    F: Handler<Args, ()> + 'static,
{
    FabMenuItem::new(label, icon, action)
}

/// A Material Design 3 floating action button menu.
#[derive(Debug)]
pub struct FloatingActionButtonMenu {
    items: Vec<FabMenuItem>,
    expanded: Binding<bool>,
    toggle: AnyView,
    toggle_accessibility_label: Str,
}

impl FloatingActionButtonMenu {
    /// Creates a menu whose toggle shows `toggle_icon`.
    ///
    /// `expanded` is owned by the caller so the menu can be closed by anything
    /// that dismisses it, not only by its own toggle.
    #[must_use]
    pub fn new(
        expanded: &Binding<bool>,
        toggle_accessibility_label: impl Into<Str>,
        toggle_icon: impl View + 'static,
    ) -> Self {
        Self {
            items: Vec::new(),
            expanded: expanded.clone(),
            toggle: AnyView::new(toggle_icon),
            toggle_accessibility_label: toggle_accessibility_label.into(),
        }
    }

    /// Adds an action to the menu.
    #[must_use]
    pub fn item(mut self, item: FabMenuItem) -> Self {
        self.items.push(item);
        self
    }

    /// Adds several actions to the menu.
    #[must_use]
    pub fn items(mut self, items: impl IntoIterator<Item = FabMenuItem>) -> Self {
        self.items.extend(items);
        self
    }
}

/// The opacity of the item at `index` of `count`, given the expansion progress.
///
/// Items reveal from the bottom up, so the last item leads. Each fade occupies
/// [`ITEM_FADE_FRACTION`] of the expansion and they are spread evenly across
/// the rest, which keeps the whole menu inside one progress signal instead of
/// needing a delayed animation per item.
fn item_opacity(progress: f32, index: usize, count: usize) -> f32 {
    if count == 0 {
        return 0.0;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "a menu holds a handful of items, exact in f32"
    )]
    let (index, count) = (index as f32, count as f32);
    // The bottom item (highest index) starts first.
    let position = count - 1.0 - index;
    let stagger_span = 1.0 - ITEM_FADE_FRACTION;
    let start = if count > 1.0 {
        position / (count - 1.0) * stagger_span
    } else {
        0.0
    };
    ((progress - start) / ITEM_FADE_FRACTION).clamp(0.0, 1.0)
}

impl View for FloatingActionButtonMenu {
    fn body(self, _env: &Environment) -> impl View {
        let Self {
            items,
            expanded,
            toggle,
            toggle_accessibility_label,
        } = self;
        let count = items.len();
        let progress: Computed<f32> = motion::fab_menu(expanded.computed(), 0.0, 1.0).computed();

        let rows = items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let mut action = item.action;
                let opacity = progress
                    .clone()
                    .map(move |progress| f64::from(item_opacity(progress, index, count)));
                // The row grows into place as it fades in. That is what keeps a
                // not-yet-revealed item from being tappable: at zero reveal it
                // has no height, so it has no hit area either — a merely
                // transparent row would still swallow taps.
                let height = progress
                    .clone()
                    .map(move |progress| ITEM_HEIGHT * item_opacity(progress, index, count));
                let row = waterui::component::hstack((
                    item.icon.size(ITEM_ICON_SIZE, ITEM_ICON_SIZE),
                    item.label,
                ))
                .spacing(ITEM_ICON_LABEL_SPACE)
                .padding_with(EdgeInsets::new(
                    0.0,
                    0.0,
                    ITEM_HORIZONTAL_SPACE,
                    ITEM_HORIZONTAL_SPACE,
                ))
                .foreground(OnPrimaryContainer)
                .floating_with(item_style())
                .opacity(opacity)
                .on_tap(move |env: Environment| action(&env))
                .a11y_role(AccessibilityRole::Button)
                .install(interaction_style(
                    OnPrimaryContainer,
                    f64::from(ITEM_HEIGHT / 2.0),
                ));
                waterui::layout::frame::Frame::new(row).height(height)
            })
            .collect::<Vec<_>>();

        let expanded_for_toggle = expanded;
        let toggle_button = toggle
            .size(CLOSE_BUTTON_ICON_SIZE, CLOSE_BUTTON_ICON_SIZE)
            .foreground(OnPrimaryContainer)
            .size(CLOSE_BUTTON_CONTAINER, CLOSE_BUTTON_CONTAINER)
            .floating_with(item_style())
            .on_tap(move |_env: Environment| {
                let open = expanded_for_toggle.get();
                expanded_for_toggle.set(!open);
            })
            .a11y_label(toggle_accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                OnPrimaryContainer,
                f64::from(CLOSE_BUTTON_CONTAINER / 2.0),
            ));

        waterui::component::vstack((
            waterui::component::VStack::new(
                waterui::layout::stack::HorizontalAlignment::Trailing,
                ITEM_BETWEEN_SPACE,
                rows,
            ),
            toggle_button,
        ))
        .spacing(CLOSE_BUTTON_BETWEEN_SPACE)
        .alignment(waterui::layout::stack::HorizontalAlignment::Trailing)
    }
}

/// The pill chrome shared by the items and the toggle: `PrimaryContainer` at
/// `ElevationTokens.Level3`, fully rounded.
fn item_style() -> waterui::style::FloatingStyle {
    let mut style = waterui::style::FloatingStyle {
        container_color: PrimaryContainer.into(),
        content_color: OnPrimaryContainer.into(),
        state_layer_color: OnPrimaryContainer.into(),
        clip_radius: 0.5,
        content_inset_x: 0.0,
        content_inset_y: 0.0,
        minimum_width: 0.0,
        minimum_height: f64::from(ITEM_HEIGHT),
        disabled_content_opacity: 0.38,
        ..waterui::style::FloatingStyle::default()
    };
    crate::elevation::apply_to_floating_style(&mut style, MaterialElevationLevel::LEVEL3);
    style
}

/// Creates a Material Design 3 floating action button menu.
#[must_use]
pub fn fab_menu(
    expanded: &Binding<bool>,
    toggle_accessibility_label: impl Into<Str>,
    toggle_icon: impl View + 'static,
) -> FloatingActionButtonMenu {
    FloatingActionButtonMenu::new(expanded, toggle_accessibility_label, toggle_icon)
}

#[cfg(test)]
mod tests {
    use super::{
        CLOSE_BUTTON_BETWEEN_SPACE, CLOSE_BUTTON_CONTAINER, CLOSE_BUTTON_ICON_SIZE,
        ITEM_BETWEEN_SPACE, ITEM_HEIGHT, ITEM_HORIZONTAL_SPACE, ITEM_ICON_LABEL_SPACE,
        ITEM_ICON_SIZE, item_opacity,
    };

    /// Values from `androidx.compose.material3.tokens.FabMenuBaselineTokens`.
    #[test]
    fn fab_menu_tokens_match_compose_fab_menu_tokens() {
        assert_eq!(ITEM_HEIGHT, 56.0);
        assert_eq!(ITEM_HORIZONTAL_SPACE, 24.0);
        assert_eq!(ITEM_ICON_LABEL_SPACE, 8.0);
        assert_eq!(ITEM_ICON_SIZE, 24.0);
        assert_eq!(ITEM_BETWEEN_SPACE, 4.0);
        assert_eq!(CLOSE_BUTTON_BETWEEN_SPACE, 8.0);
        assert_eq!(CLOSE_BUTTON_CONTAINER, 56.0);
        assert_eq!(CLOSE_BUTTON_ICON_SIZE, 20.0);
    }

    /// Items reveal from the bottom up: at any progress mid-expansion, a lower
    /// item is at least as visible as the one above it.
    #[test]
    fn items_reveal_from_the_bottom_up() {
        const COUNT: usize = 4;
        for step in 0..=20 {
            #[allow(clippy::cast_precision_loss, reason = "small loop counter")]
            let progress = step as f32 / 20.0;
            for index in 1..COUNT {
                let above = item_opacity(progress, index - 1, COUNT);
                let below = item_opacity(progress, index, COUNT);
                assert!(
                    below >= above - 1e-6,
                    "item {index} should lead item {} at {progress}: {below} vs {above}",
                    index - 1
                );
            }
        }
    }

    /// Collapsed hides every item and fully expanded shows every item, so the
    /// menu has no half-open resting state.
    #[test]
    fn the_expansion_ends_are_fully_closed_and_fully_open() {
        const COUNT: usize = 5;
        for index in 0..COUNT {
            assert_eq!(item_opacity(0.0, index, COUNT), 0.0, "collapsed hides all");
            assert_eq!(item_opacity(1.0, index, COUNT), 1.0, "expanded shows all");
        }
        // An empty menu is not a panic.
        assert_eq!(item_opacity(1.0, 0, 0), 0.0);
    }
}
