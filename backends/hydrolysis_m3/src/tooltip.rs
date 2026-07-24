//! Material Design 3 tooltips composed from `WaterUI` primitives.

use core::fmt::{self, Debug};
use core::time::Duration;

use waterui::accessibility::{AccessibilityRole, AccessibilityState};
use waterui::layout::{
    Layout, ProposalSize, Rect, Size, SubView, container::FixedContainer, padding::EdgeInsets,
};
use waterui::reactive::SignalExt as _;
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::style::Anchor;
use waterui::task::{sleep, spawn_local};
use waterui::{Binding, Environment, Str, View, ViewExt as _};
use waterui_backend_core::widget::InteractionFocusBinding;
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{Handler, SharedAction, boxed_action};

use crate::color::{InverseOnSurface, InverseSurface, OnSurfaceVariant, Primary, SurfaceContainer};
use crate::elevation::{MaterialElevationLevel, material_elevation};
use crate::semantics::{interaction_style, label_plain_text};
use crate::theme::{motion, typography};

const PLAIN_TOOLTIP_CONTAINER_HEIGHT: f32 = 24.0;
const PLAIN_TOOLTIP_CONTAINER_SHAPE: f32 = 4.0;
const PLAIN_TOOLTIP_CONTAINER_CLIP_RADIUS: f32 =
    PLAIN_TOOLTIP_CONTAINER_SHAPE / PLAIN_TOOLTIP_CONTAINER_HEIGHT;
const PLAIN_TOOLTIP_TOP_SPACE: f32 = 4.0;
const PLAIN_TOOLTIP_BOTTOM_SPACE: f32 = 4.0;
const PLAIN_TOOLTIP_LEADING_SPACE: f32 = 8.0;
const PLAIN_TOOLTIP_TRAILING_SPACE: f32 = 8.0;
const RICH_TOOLTIP_CONTAINER_SHAPE: f32 = 12.0;
const RICH_TOOLTIP_CONTAINER_CLIP_RADIUS: f32 =
    RICH_TOOLTIP_CONTAINER_SHAPE / RICH_TOOLTIP_MAX_WIDTH;
const RICH_TOOLTIP_MAX_WIDTH: f32 = 312.0;
const RICH_TOOLTIP_HORIZONTAL_PADDING: f32 = 16.0;
const RICH_TOOLTIP_TOP_PADDING: f32 = 12.0;
const RICH_TOOLTIP_BOTTOM_PADDING: f32 = 8.0;
const RICH_TOOLTIP_CONTENT_SPACING: f32 = 4.0;
const RICH_TOOLTIP_ACTION_TOP_SPACE: f32 = 8.0;
const RICH_TOOLTIP_ACTION_HEIGHT: f32 = 40.0;
const PLAIN_TOOLTIP_TARGET_GAP: f32 = 4.0;
const RICH_TOOLTIP_TARGET_GAP: f32 = 0.0;
const TOOLTIP_OPEN_DELAY: Duration = Duration::from_millis(150);
const TOOLTIP_CLOSE_DELAY: Duration = Duration::from_millis(150);

/// A tooltip attached to the target that owns its hover interaction.
pub struct TooltipAnchor<Target, Popup> {
    target: Target,
    popup: Popup,
    rich: bool,
    visibility: TooltipVisibility,
}

impl<Target, Popup> Debug for TooltipAnchor<Target, Popup> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TooltipAnchor")
            .field("rich", &self.rich)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct TooltipVisibility {
    open: Binding<bool>,
    generation: Binding<u64>,
    target_hovered: Binding<bool>,
    popup_hovered: Binding<bool>,
    focused: Binding<bool>,
}

impl TooltipVisibility {
    fn new() -> Self {
        Self {
            open: Binding::bool(false),
            generation: Binding::container(0),
            target_hovered: Binding::bool(false),
            popup_hovered: Binding::bool(false),
            focused: Binding::bool(false),
        }
    }

    fn next_generation(&self) -> u64 {
        let generation = self
            .generation
            .get()
            .checked_add(1)
            .expect("Material tooltip generation overflow");
        self.generation.set(generation);
        generation
    }

    fn cancel_pending(&self) {
        self.next_generation();
    }

    fn schedule_open(&self) {
        if self.open.get() {
            self.cancel_pending();
            return;
        }
        let generation = self.next_generation();
        let state = self.clone();
        spawn_local(async move {
            sleep(TOOLTIP_OPEN_DELAY).await;
            if state.generation.get() == generation && state.target_hovered.get() {
                state.open.set(true);
            }
        })
        .detach();
    }

    fn schedule_close(&self) {
        let generation = self.next_generation();
        let state = self.clone();
        spawn_local(async move {
            sleep(TOOLTIP_CLOSE_DELAY).await;
            if state.generation.get() == generation
                && !state.target_hovered.get()
                && !state.popup_hovered.get()
                && !state.focused.get()
            {
                state.open.set(false);
            }
        })
        .detach();
    }

    fn set_target_hovered(&self, hovered: bool) {
        self.target_hovered.set(hovered);
        if hovered {
            self.schedule_open();
        } else if self.open.get() && !self.focused.get() {
            self.schedule_close();
        } else {
            self.cancel_pending();
        }
    }

    fn set_popup_hovered(&self, hovered: bool) {
        self.popup_hovered.set(hovered);
        if hovered {
            self.cancel_pending();
        } else if self.open.get() && !self.target_hovered.get() && !self.focused.get() {
            self.schedule_close();
        }
    }

    fn set_focused(&self, focused: bool) {
        self.focused.set(focused);
        self.cancel_pending();
        if focused {
            self.open.set(true);
        } else if !self.target_hovered.get() && !self.popup_hovered.get() {
            self.open.set(false);
        }
    }

    fn dismiss(&self) {
        self.focused.set(false);
        self.cancel_pending();
        self.open.set(false);
    }
}

impl<Target, Popup> View for TooltipAnchor<Target, Popup>
where
    Target: View + 'static,
    Popup: View + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let target_enter = self.visibility.clone();
        let target_exit = self.visibility.clone();
        let popup_enter = self.visibility.clone();
        let popup_exit = self.visibility.clone();
        let focus_change = self.visibility.clone();
        let escape = self.visibility.clone();
        let focused = Binding::bool(false);
        let focus_binding = InteractionFocusBinding::new(&focused)
            .escape_action(SharedAction::new(move |_: Environment| escape.dismiss()));
        let open = self.visibility.open.clone();
        let popup_accessibility = open.map(|open| AccessibilityState::new().hidden(!open));
        let popup_scale = open
            .map(|open| if open { 1.0 } else { 0.0 })
            .with(motion::tooltip());
        let target = self
            .target
            .on_hover_enter(move || target_enter.set_target_hovered(true))
            .on_hover_exit(move || target_exit.set_target_hovered(false))
            .install(focus_binding)
            .on_change(&focused, move |focused| focus_change.set_focused(focused));
        let popup_anchor = if self.rich {
            Anchor::TOP_LEFT
        } else {
            Anchor::new(0.5, 1.0)
        };
        let popup = self
            .popup
            .on_hover_enter(move || popup_enter.set_popup_hovered(true))
            .on_hover_exit(move || popup_exit.set_popup_hovered(false))
            .scale_from(popup_scale.clone(), popup_scale, popup_anchor)
            .a11y_state_signal(popup_accessibility)
            .hittable(open);

        FixedContainer::new(TooltipLayout { rich: self.rich }, (target, popup))
    }
}

#[derive(Debug, Clone, Copy)]
struct TooltipLayout {
    rich: bool,
}

impl Layout for TooltipLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let [target, _popup] = children else {
            return Size::zero();
        };
        target.measure(proposal).size
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let [target, popup] = children else {
            return Vec::new();
        };
        let target_size = target
            .measure(ProposalSize::new(
                Some(bounds.width()),
                Some(bounds.height()),
            ))
            .size;
        let target_rect = Rect::new(bounds.origin(), target_size);
        let popup_size = popup
            .measure(ProposalSize::new(
                Some(if self.rich {
                    RICH_TOOLTIP_MAX_WIDTH
                } else {
                    320.0
                }),
                None,
            ))
            .size;
        let popup_x = if self.rich {
            target_rect.x() + target_rect.width() + RICH_TOOLTIP_TARGET_GAP
        } else {
            (target_rect.width() - popup_size.width).mul_add(0.5, target_rect.x())
        };
        let popup_y = if self.rich {
            target_rect.y() + target_rect.height() + RICH_TOOLTIP_TARGET_GAP
        } else {
            target_rect.y() - popup_size.height - PLAIN_TOOLTIP_TARGET_GAP
        };
        vec![
            target_rect,
            Rect::new(waterui::layout::Point::new(popup_x, popup_y), popup_size),
        ]
    }
}

/// A Material Design 3 plain tooltip.
pub struct PlainTooltip {
    supporting_text: Label,
    accessibility_label: Str,
}

impl Debug for PlainTooltip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlainTooltip")
            .field("supporting_text", &self.supporting_text)
            .finish_non_exhaustive()
    }
}

impl PlainTooltip {
    /// Creates a plain tooltip with supporting text.
    #[must_use]
    pub fn new(supporting_text: impl IntoLabel) -> Self {
        let supporting_text = supporting_text.into_label();
        let accessibility_label = label_plain_text(&supporting_text);
        Self {
            supporting_text,
            accessibility_label,
        }
    }

    /// Attaches this tooltip to its interactive target.
    #[must_use]
    pub fn for_target<Target>(self, target: Target) -> TooltipAnchor<Target, Self>
    where
        Target: View + 'static,
    {
        TooltipAnchor {
            target,
            popup: self,
            rich: false,
            visibility: TooltipVisibility::new(),
        }
    }
}

impl View for PlainTooltip {
    fn body(self, _env: &Environment) -> impl View {
        self.supporting_text
            .font(typography::body_small())
            .foreground(InverseOnSurface)
            .padding_with(EdgeInsets::new(
                PLAIN_TOOLTIP_TOP_SPACE,
                PLAIN_TOOLTIP_BOTTOM_SPACE,
                PLAIN_TOOLTIP_LEADING_SPACE,
                PLAIN_TOOLTIP_TRAILING_SPACE,
            ))
            .background(
                RoundedRectangle::new(PLAIN_TOOLTIP_CONTAINER_CLIP_RADIUS).fill(InverseSurface),
            )
            .min_width(28.0)
            .max_width(320.0)
            .min_height(PLAIN_TOOLTIP_CONTAINER_HEIGHT)
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Group)
    }
}

/// A Material Design 3 rich tooltip.
pub struct RichTooltip<Action = fn(&Environment)> {
    subhead: Label,
    supporting_text: Label,
    accessibility_label: Str,
    action: Option<(Label, Action)>,
}

impl<Action> Debug for RichTooltip<Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RichTooltip")
            .field("subhead", &self.subhead)
            .field("supporting_text", &self.supporting_text)
            .finish_non_exhaustive()
    }
}

impl RichTooltip<fn(&Environment)> {
    /// Creates a rich tooltip with a subhead and supporting text.
    #[must_use]
    pub fn new(subhead: impl IntoLabel, supporting_text: impl IntoLabel) -> Self {
        let subhead = subhead.into_label();
        let supporting_text = supporting_text.into_label();
        let accessibility_label = label_plain_text(&supporting_text);
        Self {
            subhead,
            supporting_text,
            accessibility_label,
            action: None,
        }
    }
}

impl<Action> RichTooltip<Action> {
    /// Adds a Material rich tooltip action.
    #[must_use]
    pub fn action<F, Args>(
        self,
        label: impl IntoLabel,
        action: F,
    ) -> RichTooltip<impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        RichTooltip {
            subhead: self.subhead,
            supporting_text: self.supporting_text,
            accessibility_label: self.accessibility_label,
            action: Some((label.into_label(), boxed_action(action))),
        }
    }

    /// Attaches this tooltip to its interactive target.
    #[must_use]
    pub fn for_target<Target>(self, target: Target) -> TooltipAnchor<Target, Self>
    where
        Target: View + 'static,
    {
        TooltipAnchor {
            target,
            popup: self,
            rich: true,
            visibility: TooltipVisibility::new(),
        }
    }
}

impl<Action> View for RichTooltip<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let content = waterui::component::vstack((
            self.subhead
                .font(typography::title_small())
                .foreground(OnSurfaceVariant),
            self.supporting_text
                .font(typography::body_medium())
                .foreground(OnSurfaceVariant),
            rich_tooltip_action(self.action),
        ))
        .spacing(RICH_TOOLTIP_CONTENT_SPACING)
        .padding_with(EdgeInsets::new(
            RICH_TOOLTIP_TOP_PADDING,
            RICH_TOOLTIP_BOTTOM_PADDING,
            RICH_TOOLTIP_HORIZONTAL_PADDING,
            RICH_TOOLTIP_HORIZONTAL_PADDING,
        ))
        .background(
            RoundedRectangle::new(RICH_TOOLTIP_CONTAINER_CLIP_RADIUS).fill(SurfaceContainer),
        )
        .max_width(RICH_TOOLTIP_MAX_WIDTH);

        material_elevation(MaterialElevationLevel::LEVEL2, content)
            .a11y_label(self.accessibility_label)
            .a11y_role(AccessibilityRole::Group)
    }
}

fn rich_tooltip_action<Action>(action: Option<(Label, Action)>) -> impl View
where
    Action: FnMut(&Environment) + 'static,
{
    let Some((label, mut action)) = action else {
        return waterui::component::hstack(((),)).anyview();
    };
    let accessibility_label = label_plain_text(&label);

    label
        .font(typography::label_large())
        .foreground(Primary)
        .height(RICH_TOOLTIP_ACTION_HEIGHT)
        .padding_with(EdgeInsets::new(
            RICH_TOOLTIP_ACTION_TOP_SPACE,
            0.0,
            0.0,
            0.0,
        ))
        .on_tap(move |env: Environment| action(&env))
        .a11y_label(accessibility_label)
        .a11y_role(AccessibilityRole::Button)
        .install(interaction_style(Primary, 20.0))
        .anyview()
}

/// Creates a Material Design 3 plain tooltip.
#[must_use]
pub fn plain_tooltip(supporting_text: impl IntoLabel) -> PlainTooltip {
    PlainTooltip::new(supporting_text)
}

/// Creates a Material Design 3 rich tooltip.
#[must_use]
pub fn rich_tooltip(
    subhead: impl IntoLabel,
    supporting_text: impl IntoLabel,
) -> RichTooltip<fn(&Environment)> {
    RichTooltip::new(subhead, supporting_text)
}

#[cfg(test)]
mod tests {
    use super::{
        PLAIN_TOOLTIP_CONTAINER_HEIGHT, PLAIN_TOOLTIP_CONTAINER_SHAPE, PLAIN_TOOLTIP_LEADING_SPACE,
        PLAIN_TOOLTIP_TARGET_GAP, PLAIN_TOOLTIP_TOP_SPACE, RICH_TOOLTIP_CONTAINER_SHAPE,
        RICH_TOOLTIP_HORIZONTAL_PADDING, RICH_TOOLTIP_MAX_WIDTH, RICH_TOOLTIP_TARGET_GAP,
        TOOLTIP_CLOSE_DELAY, TOOLTIP_OPEN_DELAY,
    };
    use core::time::Duration;

    #[test]
    fn plain_tooltip_tokens_match_mdui_2_1_5() {
        assert_eq!(PLAIN_TOOLTIP_CONTAINER_HEIGHT, 24.0);
        assert_eq!(PLAIN_TOOLTIP_CONTAINER_SHAPE, 4.0);
        assert_eq!(PLAIN_TOOLTIP_TOP_SPACE, 4.0);
        assert_eq!(PLAIN_TOOLTIP_LEADING_SPACE, 8.0);
        assert_eq!(PLAIN_TOOLTIP_TARGET_GAP, 4.0);
        assert_eq!(TOOLTIP_OPEN_DELAY, Duration::from_millis(150));
        assert_eq!(TOOLTIP_CLOSE_DELAY, Duration::from_millis(150));
    }

    #[test]
    fn rich_tooltip_tokens_match_mdui_2_1_5() {
        assert_eq!(RICH_TOOLTIP_CONTAINER_SHAPE, 12.0);
        assert_eq!(RICH_TOOLTIP_MAX_WIDTH, 312.0);
        assert_eq!(RICH_TOOLTIP_HORIZONTAL_PADDING, 16.0);
        assert_eq!(RICH_TOOLTIP_TARGET_GAP, 0.0);
    }
}
