//! Material Design 3 navigation drawer composed from `WaterUI` primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole, AccessibilityState};
use waterui::color::Color;
use waterui::layout::{
    Layout, ProposalSize, Rect, Size, StretchAxis, SubView, container::FixedContainer,
    padding::EdgeInsets,
};
use waterui::prelude::{PositionExt as _, UnitPoint, absolute};
use waterui::reactive::SignalExt as _;
use waterui::shape::{RoundedRectangle, ShapeExt as _, UnevenRoundedRectangle};
use waterui::{AnyView, Binding, Environment, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, SharedAction, boxed_action};

use crate::ModalInteraction;
use crate::color::{
    OnSecondaryContainer, OnSurfaceVariant, Scrim, SecondaryContainer, Surface, SurfaceContainerLow,
};
use crate::elevation::{MaterialElevationLevel, material_elevation};
use crate::semantics::{conditional_color, interaction_style, label_plain_text};
use crate::theme::{motion, typography};

const NAVIGATION_DRAWER_CONTAINER_WIDTH: f32 = 360.0;
const NAVIGATION_DRAWER_MODAL_MAX_VIEWPORT_FRACTION: f32 = 0.8;
const NAVIGATION_DRAWER_CONTAINER_SHAPE: f32 = 16.0;
const NAVIGATION_DRAWER_CONTAINER_CLIP_RADIUS: f32 =
    NAVIGATION_DRAWER_CONTAINER_SHAPE / NAVIGATION_DRAWER_CONTAINER_WIDTH;
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
    modal: bool,
    close_on_overlay_click: bool,
    close_on_escape: bool,
    overlay_action: SharedAction,
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
            modal: false,
            close_on_overlay_click: false,
            close_on_escape: false,
            overlay_action: SharedAction::new(|_: Environment| {}),
        }
    }

    /// Sets the drawer accessibility label.
    #[must_use]
    pub fn label(mut self, label: impl Into<Str>) -> Self {
        self.accessibility_label = label.into();
        self
    }

    /// Uses the modal drawer presentation with a scrim and focus trap.
    #[must_use]
    pub const fn modal(mut self) -> Self {
        self.modal = true;
        self
    }

    /// Closes a modal drawer when its scrim is tapped.
    #[must_use]
    pub const fn close_on_overlay_click(mut self) -> Self {
        self.close_on_overlay_click = true;
        self
    }

    /// Closes a modal drawer when Escape is pressed.
    #[must_use]
    pub const fn close_on_escape(mut self) -> Self {
        self.close_on_escape = true;
        self
    }

    /// Sets the notification action emitted whenever the modal scrim is tapped.
    #[must_use]
    pub fn overlay_action<F, Args>(mut self, action: F) -> Self
    where
        F: Handler<Args, ()> + 'static,
    {
        let mut action = boxed_action(action);
        self.overlay_action = SharedAction::new(move |env: Environment| action(&env));
        self
    }
}

impl<Content> View for NavigationDrawer<Content>
where
    Content: View + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let accessibility_state = self.opened.map(|opened| {
            AccessibilityState::new()
                .expanded(Some(opened))
                .hidden(!opened)
        });
        let offset = motion::navigation_drawer(
            self.opened.computed(),
            -NAVIGATION_DRAWER_CONTAINER_WIDTH,
            0.0,
        );
        let opened = self.opened;

        if self.modal {
            let panel_content = FixedContainer::new(NavigationDrawerPanelLayout, (self.content,))
                .background(
                    UnevenRoundedRectangle::new(
                        0.0,
                        NAVIGATION_DRAWER_CONTAINER_CLIP_RADIUS,
                        0.0,
                        NAVIGATION_DRAWER_CONTAINER_CLIP_RADIUS,
                    )
                    .fill(SurfaceContainerLow),
                )
                .a11y_label(self.accessibility_label)
                .a11y_role(AccessibilityRole::Group)
                .a11y_state_signal(accessibility_state);
            let panel = material_elevation(MaterialElevationLevel::LEVEL1, panel_content)
                .offset(offset, 0.0)
                .position_in(UnitPoint::TOP_LEADING);
            let close_on_overlay_click = self.close_on_overlay_click;
            let opened_for_overlay = opened.clone();
            let overlay_action = self.overlay_action;
            let scrim = Scrim
                .with_opacity(1.0)
                .opacity(motion::navigation_drawer_scrim(opened.computed(), 0.0, 0.4))
                .on_tap(move |env: Environment| {
                    overlay_action.call(&env);
                    if close_on_overlay_click {
                        opened_for_overlay.set(false);
                    }
                })
                .a11y_hidden(true)
                .install(interaction_style(Scrim.with_opacity(0.0), 0.0).pointer_only());
            let opened_for_escape = opened.clone();
            let modal = ModalInteraction::new(
                self.close_on_escape,
                SharedAction::new(move |_: Environment| opened_for_escape.set(false)),
            )
            .active(opened.clone());

            return AnyView::new(absolute((scrim, panel)).hittable(opened).install(modal));
        }

        AnyView::new(
            self.content
                .width(NAVIGATION_DRAWER_CONTAINER_WIDTH)
                .background(Surface)
                .offset(offset, 0.0)
                .a11y_label(self.accessibility_label)
                .a11y_role(AccessibilityRole::Group)
                .a11y_state_signal(accessibility_state),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct NavigationDrawerPanelLayout;

impl Layout for NavigationDrawerPanelLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let proposed_width = proposal.width.unwrap_or(NAVIGATION_DRAWER_CONTAINER_WIDTH);
        let width = NAVIGATION_DRAWER_CONTAINER_WIDTH
            .min(proposed_width * NAVIGATION_DRAWER_MODAL_MAX_VIEWPORT_FRACTION);
        let height = proposal.height.unwrap_or_else(|| {
            children.first().map_or(0.0, |content| {
                content
                    .measure(ProposalSize::new(Some(width), None))
                    .size
                    .height
            })
        });
        Size::new(width, height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        children.iter().map(|_| bounds).collect()
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Vertical
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
            .map(|selected| AccessibilityState::new().selected(selected));
        let foreground = conditional_color(
            self.selected.clone(),
            OnSecondaryContainer,
            OnSurfaceVariant,
        );
        let background = conditional_color(
            self.selected.clone(),
            SecondaryContainer,
            SurfaceContainerLow,
        );
        let state_layer_color =
            conditional_color(self.selected, OnSecondaryContainer, OnSurfaceVariant);

        drawer_item_content(self.label, self.icon, foreground, background)
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_state_signal(accessibility_state)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(
                state_layer_color,
                f64::from(NAVIGATION_DRAWER_ITEM_CONTAINER_SHAPE),
            ))
    }
}

fn drawer_item_content(
    label: Label,
    icon: impl View,
    foreground: Color,
    background: Color,
) -> impl View {
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
        NAVIGATION_DRAWER_MODAL_MAX_VIEWPORT_FRACTION,
    };

    #[test]
    fn navigation_drawer_tokens_match_mdui_2_1_5() {
        assert_eq!(NAVIGATION_DRAWER_CONTAINER_WIDTH, 360.0);
        assert_eq!(NAVIGATION_DRAWER_CONTAINER_SHAPE, 16.0);
        assert_eq!(NAVIGATION_DRAWER_MODAL_MAX_VIEWPORT_FRACTION, 0.8);
        assert_eq!(NAVIGATION_DRAWER_ITEM_HEIGHT, 56.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_ICON_SIZE, 24.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_ICON_LABEL_SPACE, 12.0);
        assert_eq!(NAVIGATION_DRAWER_ITEM_HORIZONTAL_PADDING, 16.0);
    }
}
