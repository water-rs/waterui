//! Material Design 3 navigation drawer composed from WaterUI primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::color::Color;
use waterui::layout::padding::EdgeInsets;
use waterui::reactive::SignalExt as _;
use waterui::shape::{RoundedRectangle, ShapeExt as _, UnevenRoundedRectangle};
use waterui::style::{Shadow, Vector};
use waterui::widget::condition::when;
use waterui::{Binding, Environment, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, boxed_action};

use crate::color::{
    OnSecondaryContainer, OnSurfaceVariant, SecondaryContainer, Surface, SurfaceContainerLow,
};
use crate::semantics::label_plain_text;
use crate::theme::{motion, typography};

const NAVIGATION_DRAWER_CONTAINER_WIDTH: f32 = 360.0;
const NAVIGATION_DRAWER_CONTAINER_SHAPE: f32 = 16.0;
const NAVIGATION_DRAWER_CONTAINER_CLIP_RADIUS: f32 =
    NAVIGATION_DRAWER_CONTAINER_SHAPE / NAVIGATION_DRAWER_CONTAINER_WIDTH;
const NAVIGATION_DRAWER_MODAL_ELEVATION_Y: f32 = 1.0;
const NAVIGATION_DRAWER_MODAL_ELEVATION_BLUR: f32 = 3.0;
const NAVIGATION_DRAWER_MODAL_SHADOW_OPACITY: f32 = 0.18;
const NAVIGATION_DRAWER_ITEM_HEIGHT: f32 = 56.0;
const NAVIGATION_DRAWER_ITEM_CONTAINER_SHAPE: f32 = 28.0;
const NAVIGATION_DRAWER_ITEM_CLIP_RADIUS: f32 =
    NAVIGATION_DRAWER_ITEM_CONTAINER_SHAPE / NAVIGATION_DRAWER_CONTAINER_WIDTH;
const NAVIGATION_DRAWER_ITEM_HORIZONTAL_PADDING: f32 = 16.0;
const NAVIGATION_DRAWER_ITEM_ICON_SIZE: f32 = 24.0;
const NAVIGATION_DRAWER_ITEM_ICON_LABEL_SPACE: f32 = 12.0;

/// A Material Design 3 navigation drawer surface.
pub struct NavigationDrawer<Content> {
    opened: Binding<bool>,
    content: Content,
    accessibility_label: Str,
}

impl<Content> Debug for NavigationDrawer<Content> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationDrawer")
            .field("accessibility_label", &self.accessibility_label)
            .finish_non_exhaustive()
    }
}

impl<Content> NavigationDrawer<Content> {
    /// Creates a navigation drawer controlled by an opened binding.
    #[must_use]
    pub fn new(opened: &Binding<bool>, content: Content) -> Self {
        Self {
            opened: opened.clone(),
            content,
            accessibility_label: "Navigation drawer".into(),
        }
    }

    /// Sets the drawer accessibility label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Str>) -> Self {
        self.accessibility_label = label.into();
        self
    }
}

impl<Content> View for NavigationDrawer<Content>
where
    Content: View + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let accessibility_state = self.opened.clone().map(|opened| {
            AccessibilityState::new()
                .expanded(Some(opened))
                .hidden(!opened)
        });
        let offset = self
            .opened
            .map(|opened| {
                if opened {
                    0.0
                } else {
                    -NAVIGATION_DRAWER_CONTAINER_WIDTH
                }
            })
            .with(motion::navigation_drawer());

        self.content
            .width(NAVIGATION_DRAWER_CONTAINER_WIDTH)
            .background(
                UnevenRoundedRectangle::new(
                    0.0,
                    NAVIGATION_DRAWER_CONTAINER_CLIP_RADIUS,
                    0.0,
                    NAVIGATION_DRAWER_CONTAINER_CLIP_RADIUS,
                )
                .fill(Surface),
            )
            .shadow(Shadow::new(
                Color::srgb(0, 0, 0).with_opacity(NAVIGATION_DRAWER_MODAL_SHADOW_OPACITY),
                Vector::new(0.0, NAVIGATION_DRAWER_MODAL_ELEVATION_Y),
                NAVIGATION_DRAWER_MODAL_ELEVATION_BLUR,
            ))
            .offset(offset, 0.0)
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Group)
            .a11y_state_signal(accessibility_state)
    }
}

/// A selectable Material Design 3 navigation drawer item.
pub struct NavigationDrawerItem<Icon, Action = fn(&Environment)> {
    label: Label,
    accessibility_label: Str,
    icon: Icon,
    selected: Binding<bool>,
    action: Action,
}

impl<Icon, Action> Debug for NavigationDrawerItem<Icon, Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NavigationDrawerItem")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl<Icon> NavigationDrawerItem<Icon, fn(&Environment)> {
    /// Creates a navigation drawer item.
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

impl<Icon, Action> NavigationDrawerItem<Icon, Action> {
    /// Sets the action performed when the drawer item is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> NavigationDrawerItem<Icon, impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        NavigationDrawerItem {
            label: self.label,
            accessibility_label: self.accessibility_label,
            icon: self.icon,
            selected: self.selected,
            action: boxed_action(action),
        }
    }
}

impl<Icon, Action> View for NavigationDrawerItem<Icon, Action>
where
    Icon: Clone + View + 'static,
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let mut action = self.action;
        let accessibility_label = self.accessibility_label.clone();
        let accessibility_state = self
            .selected
            .clone()
            .map(|selected| AccessibilityState::new().selected(selected));
        let selected_for_state = self.selected;
        let selected_icon = self.icon.clone();
        let unselected_icon = self.icon;
        let selected_label = self.label.clone();
        let unselected_label = self.label;

        when(selected_for_state, move || {
            drawer_item_content(selected_label.clone(), selected_icon.clone(), true)
        })
        .otherwise(move || {
            drawer_item_content(unselected_label.clone(), unselected_icon.clone(), false)
        })
        .height(NAVIGATION_DRAWER_ITEM_HEIGHT)
        .on_tap(move |env: Environment| action(&env))
        .a11y_label(accessibility_label)
        .a11y_role(AccessibilityRole::Button)
        .a11y_state_signal(accessibility_state)
        .a11y_children(AccessibilityChildren::ExcludeDescendants)
    }
}

fn drawer_item_content(label: Label, icon: impl View, selected: bool) -> impl View {
    let foreground: Color = if selected {
        OnSecondaryContainer.into()
    } else {
        OnSurfaceVariant.into()
    };
    let background: Color = if selected {
        SecondaryContainer.into()
    } else {
        SurfaceContainerLow.into()
    };

    waterui::component::hstack((
        icon.foreground(foreground.clone())
            .width(NAVIGATION_DRAWER_ITEM_ICON_SIZE)
            .height(NAVIGATION_DRAWER_ITEM_ICON_SIZE),
        label.font(typography::label_large()).foreground(foreground),
        waterui::component::spacer(),
    ))
    .spacing(NAVIGATION_DRAWER_ITEM_ICON_LABEL_SPACE)
    .height(NAVIGATION_DRAWER_ITEM_HEIGHT)
    .padding_with(EdgeInsets::new(
        0.0,
        0.0,
        NAVIGATION_DRAWER_ITEM_HORIZONTAL_PADDING,
        NAVIGATION_DRAWER_ITEM_HORIZONTAL_PADDING,
    ))
    .background(RoundedRectangle::new(NAVIGATION_DRAWER_ITEM_CLIP_RADIUS).fill(background))
}

const fn noop(_env: &Environment) {}

/// Creates a Material Design 3 navigation drawer.
#[must_use]
pub fn navigation_drawer<Content>(
    opened: &Binding<bool>,
    content: Content,
) -> NavigationDrawer<Content> {
    NavigationDrawer::new(opened, content)
}

/// Creates a Material Design 3 navigation drawer item.
#[must_use]
pub fn navigation_drawer_item<Icon>(
    label: impl IntoLabel,
    icon: Icon,
    selected: &Binding<bool>,
) -> NavigationDrawerItem<Icon> {
    NavigationDrawerItem::new(label, icon, selected)
}

#[cfg(test)]
mod tests {
    use super::{
        NAVIGATION_DRAWER_CONTAINER_SHAPE, NAVIGATION_DRAWER_CONTAINER_WIDTH,
        NAVIGATION_DRAWER_ITEM_HEIGHT, NAVIGATION_DRAWER_ITEM_HORIZONTAL_PADDING,
        NAVIGATION_DRAWER_ITEM_ICON_LABEL_SPACE, NAVIGATION_DRAWER_ITEM_ICON_SIZE,
    };

    #[test]
    fn navigation_drawer_tokens_match_material_web_labs_reference() {
        assert_eq!(NAVIGATION_DRAWER_CONTAINER_WIDTH, 360.0);
        assert_eq!(NAVIGATION_DRAWER_CONTAINER_SHAPE, 16.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_HEIGHT, 56.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_ICON_SIZE, 24.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_ICON_LABEL_SPACE, 12.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_HORIZONTAL_PADDING, 16.0);
    }
}
