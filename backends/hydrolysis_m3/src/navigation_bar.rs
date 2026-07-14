//! Material Design 3 navigation bar composed from `WaterUI` primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::reactive::SignalExt as _;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::style::{Shadow, Vector};
use waterui::widget::condition::when;
use waterui::{Binding, Environment, Signal, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};
use waterui_core::view::TupleViews;

use crate::color::{
    OnSecondaryContainer, OnSurface, OnSurfaceVariant, SecondaryContainer, SurfaceContainer,
};
use crate::theme::typography;

const NAVIGATION_BAR_CONTAINER_HEIGHT: f32 = 80.0;
const NAVIGATION_BAR_ITEM_MIN_WIDTH: f32 = 48.0;
const NAVIGATION_BAR_ITEM_TOP_PADDING: f32 = 8.0;
const NAVIGATION_BAR_ITEM_BOTTOM_PADDING: f32 = 12.0;
const NAVIGATION_BAR_ICON_SIZE: f32 = 24.0;
const NAVIGATION_BAR_ICON_SLOT_HEIGHT: f32 = 32.0;
const NAVIGATION_BAR_ACTIVE_INDICATOR_WIDTH: f32 = 64.0;
const NAVIGATION_BAR_ACTIVE_INDICATOR_HEIGHT: f32 = 32.0;
const NAVIGATION_BAR_ACTIVE_INDICATOR_CLIP_RADIUS: f32 =
    NAVIGATION_BAR_ACTIVE_INDICATOR_HEIGHT / NAVIGATION_BAR_ACTIVE_INDICATOR_WIDTH;
const NAVIGATION_BAR_LABEL_TOP_SPACE: f32 = 4.0;
const NAVIGATION_BAR_CONTAINER_ELEVATION_Y: f32 = 2.0;
const NAVIGATION_BAR_CONTAINER_ELEVATION_BLUR: f32 = 4.0;
const NAVIGATION_BAR_CONTAINER_SHADOW_OPACITY: f32 = 0.2;

/// A Material Design 3 navigation bar.
pub struct NavigationBar<Tabs> {
    tabs: Tabs,
}

impl<Tabs> Debug for NavigationBar<Tabs> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationBar").finish_non_exhaustive()
    }
}

impl<Tabs> NavigationBar<Tabs> {
    /// Creates a navigation bar containing Material navigation tabs.
    #[must_use]
    pub const fn new(tabs: Tabs) -> Self {
        Self { tabs }
    }
}

impl<Tabs> View for NavigationBar<Tabs>
where
    Tabs: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        waterui::component::hstack(self.tabs)
            .height(NAVIGATION_BAR_CONTAINER_HEIGHT)
            .max_width(f32::MAX)
            .background(SurfaceContainer)
            .shadow(Shadow::new(
                Color::srgb(0, 0, 0).with_opacity(NAVIGATION_BAR_CONTAINER_SHADOW_OPACITY),
                Vector::new(0.0, NAVIGATION_BAR_CONTAINER_ELEVATION_Y),
                NAVIGATION_BAR_CONTAINER_ELEVATION_BLUR,
            ))
            .a11y_label("Navigation")
            .a11y_role(AccessibilityRole::TabList)
    }
}

/// A Material Design 3 navigation tab.
pub struct NavigationTab<Icon, Action = fn(&Environment)> {
    label: Label,
    accessibility_label: Str,
    icon: Icon,
    selected: Binding<bool>,
    action: Action,
}

impl<Icon, Action> Debug for NavigationTab<Icon, Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationTab")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<Icon> NavigationTab<Icon, fn(&Environment)> {
    /// Creates a navigation tab with a semantic label, icon, and selected state.
    #[must_use]
    pub fn new(label: impl IntoLabel, icon: Icon, selected: &Binding<bool>) -> Self {
        let label = label.into_label();
        let accessibility_label = label_plain_text(&label);
        Self {
            label,
            accessibility_label,
            icon,
            selected: selected.clone(),
            action: noop,
        }
    }
}

impl<Icon, Action> NavigationTab<Icon, Action> {
    /// Sets the action performed when the navigation tab is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> NavigationTab<Icon, impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        NavigationTab {
            label: self.label,
            accessibility_label: self.accessibility_label,
            icon: self.icon,
            selected: self.selected,
            action: boxed_action(action),
        }
    }
}

impl<Icon, Action> View for NavigationTab<Icon, Action>
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
        let selected_for_state = self.selected;
        let selected_icon = self.icon.clone();
        let unselected_icon = self.icon;
        let selected_label = self.label.clone();
        let unselected_label = self.label;

        when(selected_for_state, move || {
            navigation_tab_content(selected_label.clone(), selected_icon.clone(), true)
        })
        .otherwise(move || {
            navigation_tab_content(unselected_label.clone(), unselected_icon.clone(), false)
        })
        .min_width(NAVIGATION_BAR_ITEM_MIN_WIDTH)
        .max_width(f32::MAX)
        .height(NAVIGATION_BAR_CONTAINER_HEIGHT)
        .padding_with(EdgeInsets::new(
            NAVIGATION_BAR_ITEM_TOP_PADDING,
            NAVIGATION_BAR_ITEM_BOTTOM_PADDING,
            0.0,
            0.0,
        ))
        .on_tap(move |env: Environment| action(&env))
        .a11y_label(accessibility_label)
        .a11y_role(AccessibilityRole::Tab)
        .a11y_state_signal(accessibility_state)
        .a11y_children(AccessibilityChildren::ExcludeDescendants)
    }
}

fn navigation_tab_content(label: Label, icon: impl View, selected: bool) -> impl View {
    let indicator_color: Color = if selected {
        SecondaryContainer.into()
    } else {
        Color::transparent()
    };
    let icon_color: Color = if selected {
        OnSecondaryContainer.into()
    } else {
        OnSurfaceVariant.into()
    };
    let label_color: Color = if selected {
        OnSurface.into()
    } else {
        OnSurfaceVariant.into()
    };
    let icon_slot = icon
        .foreground(icon_color)
        .width(NAVIGATION_BAR_ICON_SIZE)
        .height(NAVIGATION_BAR_ICON_SIZE);
    let icon_container = waterui::component::zstack((
        RoundedRectangle::new(NAVIGATION_BAR_ACTIVE_INDICATOR_CLIP_RADIUS)
            .fill(indicator_color)
            .width(NAVIGATION_BAR_ACTIVE_INDICATOR_WIDTH)
            .height(NAVIGATION_BAR_ACTIVE_INDICATOR_HEIGHT),
        icon_slot,
    ));

    waterui::component::vstack((
        icon_container
            .width(NAVIGATION_BAR_ACTIVE_INDICATOR_WIDTH)
            .height(NAVIGATION_BAR_ICON_SLOT_HEIGHT),
        label
            .font(typography::label_medium())
            .foreground(label_color)
            .height(16.0),
    ))
    .spacing(NAVIGATION_BAR_LABEL_TOP_SPACE)
}

fn label_plain_text(label: &Label) -> Str {
    label
        .semantic_text()
        .clone()
        .resolve(&Environment::new())
        .content
        .get()
        .to_plain()
}

const fn noop(_env: &Environment) {}

/// Creates a Material Design 3 navigation bar.
#[must_use]
pub const fn navigation_bar<Tabs>(tabs: Tabs) -> NavigationBar<Tabs> {
    NavigationBar::new(tabs)
}

/// Creates a Material Design 3 navigation tab.
#[must_use]
pub fn navigation_tab<Icon>(
    label: impl IntoLabel,
    icon: Icon,
    selected: &Binding<bool>,
) -> NavigationTab<Icon> {
    NavigationTab::new(label, icon, selected)
}

#[cfg(test)]
mod tests {
    use super::{
        NAVIGATION_BAR_ACTIVE_INDICATOR_HEIGHT, NAVIGATION_BAR_ACTIVE_INDICATOR_WIDTH,
        NAVIGATION_BAR_CONTAINER_ELEVATION_BLUR, NAVIGATION_BAR_CONTAINER_ELEVATION_Y,
        NAVIGATION_BAR_CONTAINER_HEIGHT, NAVIGATION_BAR_CONTAINER_SHADOW_OPACITY,
        NAVIGATION_BAR_ICON_SIZE, NAVIGATION_BAR_ITEM_MIN_WIDTH, NAVIGATION_BAR_LABEL_TOP_SPACE,
    };

    #[test]
    fn navigation_bar_tokens_match_material_web_v0_192() {
        assert_eq!(NAVIGATION_BAR_CONTAINER_HEIGHT, 80.0);
        assert_eq!(NAVIGATION_BAR_ITEM_MIN_WIDTH, 48.0);
        assert_eq!(NAVIGATION_BAR_ACTIVE_INDICATOR_WIDTH, 64.0);
        assert_eq!(NAVIGATION_BAR_ACTIVE_INDICATOR_HEIGHT, 32.0);
        assert_eq!(NAVIGATION_BAR_ICON_SIZE, 24.0);
        assert_eq!(NAVIGATION_BAR_LABEL_TOP_SPACE, 4.0);
        assert_eq!(NAVIGATION_BAR_CONTAINER_ELEVATION_Y, 2.0);
        assert_eq!(NAVIGATION_BAR_CONTAINER_ELEVATION_BLUR, 4.0);
        assert_eq!(NAVIGATION_BAR_CONTAINER_SHADOW_OPACITY, 0.2);
    }
}
