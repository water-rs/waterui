//! Material Design 3 dialogs composed from `WaterUI` primitives.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::layout::{
    HorizontalAlignment, Layout, PlacedSubview, Point, ProposalSize, Rect, Size, SubView,
    container::FixedContainer, padding::EdgeInsets,
};
use waterui::prelude::{PositionExt as _, UnitPoint, absolute};
use waterui::shape::{RoundedRectangle, ShapeExt as _};
use waterui::signal::IntoComputed;
use waterui::style::Anchor;
use waterui::{Computed, Environment, SignalExt as _, Str, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::AnyView;
use waterui_core::handler::{Handler, SharedAction, boxed_action};
use waterui_core::view::TupleViews;

use crate::ModalInteraction;
use crate::color::{OnSurface, OnSurfaceVariant, Primary, Scrim, SurfaceContainerHigh};
use crate::elevation::{MaterialElevationLevel, material_elevation};
use crate::semantics::{interaction_style, label_plain_text};
use crate::theme::{motion, typography};

const DIALOG_CONTAINER_MIN_WIDTH: f32 = 280.0;
const DIALOG_CONTAINER_MAX_WIDTH: f32 = 560.0;
const DIALOG_CONTAINER_MIN_HEIGHT: f32 = 140.0;
const DIALOG_CONTAINER_SHAPE: f32 = 28.0;
const DIALOG_CONTAINER_CLIP_RADIUS: f32 = DIALOG_CONTAINER_SHAPE / DIALOG_CONTAINER_MAX_WIDTH;
const DIALOG_VIEWPORT_PADDING: f32 = 48.0;
const DIALOG_CONTENT_PADDING: f32 = 24.0;
const DIALOG_HEADLINE_BODY_SPACING: f32 = 16.0;
const DIALOG_CONTENT_BOTTOM_SPACE_WITH_ACTIONS: f32 = 8.0;
const DIALOG_ACTION_TOP_SPACE: f32 = 16.0;
const DIALOG_ACTION_TRAILING_SPACE: f32 = 24.0;
const DIALOG_ACTION_BOTTOM_SPACE: f32 = 24.0;
const DIALOG_ACTION_SPACING: f32 = 8.0;
const DIALOG_HIDDEN_OFFSET_Y: f32 = -30.0;

/// A Material Design 3 dialog action button.
pub struct DialogAction<Action = fn(&Environment)> {
    label: Label,
    action: Action,
}

impl<Action> Debug for DialogAction<Action> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DialogAction")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl DialogAction<fn(&Environment)> {
    /// Creates a dialog action with the given semantic label.
    #[must_use]
    pub fn new(label: impl IntoLabel) -> Self {
        Self {
            label: label.into_label(),
            action: noop,
        }
    }
}

impl<Action> DialogAction<Action> {
    /// Sets the action performed when the dialog action is tapped.
    #[must_use]
    pub fn action<F, Args>(self, action: F) -> DialogAction<impl FnMut(&Environment)>
    where
        F: Handler<Args, ()> + 'static,
    {
        DialogAction {
            label: self.label,
            action: boxed_action(action),
        }
    }
}

impl<Action> View for DialogAction<Action>
where
    Action: FnMut(&Environment) + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let accessibility_label = label_plain_text(&self.label);
        let mut action = self.action;
        self.label
            .font(typography::label_large())
            .foreground(Primary)
            .height(40.0)
            .padding_with(EdgeInsets::new(0.0, 0.0, 12.0, 12.0))
            .on_tap(move |env: Environment| action(&env))
            .a11y_label(accessibility_label)
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants)
            .install(interaction_style(Primary, 20.0))
    }
}

/// A Material Design 3 basic dialog.
pub struct Dialog<Actions = ()> {
    headline: Label,
    supporting_text: Label,
    accessibility_label: Str,
    actions: Actions,
    presented: Computed<bool>,
    overlay_action: SharedAction,
    close_on_escape: bool,
    escape_action: SharedAction,
}

impl<Actions> Debug for Dialog<Actions> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Dialog")
            .field("headline", &self.headline)
            .field("supporting_text", &self.supporting_text)
            .finish_non_exhaustive()
    }
}

impl Dialog<()> {
    /// Creates a Material dialog with headline and supporting text.
    #[must_use]
    pub fn new(headline: impl IntoLabel, supporting_text: impl IntoLabel) -> Self {
        let headline = headline.into_label();
        let supporting_text = supporting_text.into_label();
        let accessibility_label = label_plain_text(&headline);
        Self {
            headline,
            supporting_text,
            accessibility_label,
            actions: (),
            presented: Computed::constant(true),
            overlay_action: SharedAction::new(|_: Environment| {}),
            close_on_escape: false,
            escape_action: SharedAction::new(|_: Environment| {}),
        }
    }
}

impl<Actions> Dialog<Actions> {
    /// Sets the dialog action row.
    #[must_use]
    pub fn actions<NewActions>(self, actions: NewActions) -> Dialog<NewActions> {
        Dialog {
            headline: self.headline,
            supporting_text: self.supporting_text,
            accessibility_label: self.accessibility_label,
            actions,
            presented: self.presented,
            overlay_action: self.overlay_action,
            close_on_escape: self.close_on_escape,
            escape_action: self.escape_action,
        }
    }

    /// Controls whether the dialog is visible and modal.
    ///
    /// Keeping a controlled dialog mounted allows its MDUI open and close
    /// transitions to finish without rebuilding the surrounding view tree.
    #[must_use]
    pub fn presented(mut self, presented: impl IntoComputed<bool>) -> Self {
        self.presented = presented.into_computed();
        self
    }

    /// Sets the action emitted when the modal scrim is tapped.
    #[must_use]
    pub fn overlay_action<F, Args>(mut self, action: F) -> Self
    where
        F: Handler<Args, ()> + 'static,
    {
        let mut action = boxed_action(action);
        self.overlay_action = SharedAction::new(move |env: Environment| action(&env));
        self
    }

    /// Enables Escape-key dismissal and sets the emitted action.
    #[must_use]
    pub fn escape_action<F, Args>(mut self, action: F) -> Self
    where
        F: Handler<Args, ()> + 'static,
    {
        let mut action = boxed_action(action);
        self.close_on_escape = true;
        self.escape_action = SharedAction::new(move |env: Environment| action(&env));
        self
    }
}

impl<Actions> View for Dialog<Actions>
where
    Actions: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let presented = self.presented;
        let modal = ModalInteraction::new(self.close_on_escape, self.escape_action)
            .active(presented.clone());
        let surface = DialogSurface::new(
            self.headline
                .font(typography::headline_small())
                .foreground(OnSurface),
            self.supporting_text
                .font(typography::body_medium())
                .foreground(OnSurfaceVariant),
            DialogActions {
                actions: self.actions,
            },
        )
        .background(RoundedRectangle::new(DIALOG_CONTAINER_CLIP_RADIUS).fill(SurfaceContainerHigh))
        .background(
            SurfaceContainerHigh
                .with_opacity(0.0)
                .on_tap(|_: Environment| {})
                .a11y_hidden(true)
                .install(
                    interaction_style(SurfaceContainerHigh.with_opacity(0.0), 0.0).pointer_only(),
                ),
        )
        .a11y_label(self.accessibility_label)
        .a11y_role(AccessibilityRole::Group);

        let surface = material_elevation(MaterialElevationLevel::LEVEL3, surface)
            .opacity(motion::dialog_opacity(presented.clone(), 0.0, 1.0))
            .offset(
                0.0,
                motion::dialog_transform(presented.clone(), DIALOG_HIDDEN_OFFSET_Y, 0.0),
            )
            .scale_from(
                1.0,
                motion::dialog_transform(presented.clone(), 0.0, 1.0),
                Anchor::new(0.5, 0.0),
            )
            .padding_with(EdgeInsets::all(DIALOG_VIEWPORT_PADDING))
            .position_in(UnitPoint::CENTER);

        let overlay_action = self.overlay_action;
        let scrim = Scrim
            .with_opacity(1.0)
            .opacity(motion::dialog_opacity(presented.clone(), 0.0, 0.4))
            .on_tap(move |env: Environment| overlay_action.call(&env))
            .a11y_hidden(true)
            .install(interaction_style(Scrim.with_opacity(0.0), 0.0).pointer_only());

        let accessibility_state = presented
            .map(|presented| waterui::accessibility::AccessibilityState::new().hidden(!presented));

        absolute((scrim, surface))
            .hittable(presented)
            .a11y_state_signal(accessibility_state)
            .install(modal)
    }
}

#[derive(Debug)]
struct DialogSurface {
    headline: AnyView,
    supporting_text: AnyView,
    actions: AnyView,
}

impl DialogSurface {
    fn new(headline: impl View, supporting_text: impl View, actions: impl View) -> Self {
        Self {
            headline: AnyView::new(headline),
            supporting_text: AnyView::new(supporting_text),
            actions: AnyView::new(actions),
        }
    }
}

impl View for DialogSurface {
    fn body(self, _env: &Environment) -> impl View {
        FixedContainer::new(
            DialogSurfaceLayout,
            (self.headline, self.supporting_text, self.actions),
        )
    }
}

#[derive(Debug, Clone, Copy)]
struct DialogSurfaceLayout;

impl Layout for DialogSurfaceLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let [headline, supporting_text, actions] = children else {
            return Size::zero();
        };

        let proposed_max_width = proposal
            .width
            .unwrap_or(DIALOG_CONTAINER_MAX_WIDTH)
            .min(DIALOG_CONTAINER_MAX_WIDTH);
        let action_size = actions.measure(ProposalSize::new(None, None)).size;
        let headline_intrinsic = headline.measure(ProposalSize::new(None, None)).size;
        let supporting_intrinsic = supporting_text.measure(ProposalSize::new(None, None)).size;
        let content_width = headline_intrinsic
            .width
            .max(supporting_intrinsic.width)
            .max(action_size.width);
        let width = DIALOG_CONTENT_PADDING
            .mul_add(2.0, content_width)
            .max(DIALOG_CONTAINER_MIN_WIDTH)
            .min(proposed_max_width);
        let text_width = DIALOG_CONTENT_PADDING.mul_add(-2.0, width).max(0.0);
        let headline_size = headline
            .measure(ProposalSize::new(Some(text_width), None))
            .size;
        let supporting_size = supporting_text
            .measure(ProposalSize::new(Some(text_width), None))
            .size
            .height;
        let height = (DIALOG_CONTENT_PADDING
            + headline_size.height
            + DIALOG_HEADLINE_BODY_SPACING
            + supporting_size
            + DIALOG_CONTENT_BOTTOM_SPACE_WITH_ACTIONS
            + DIALOG_ACTION_TOP_SPACE
            + action_size.height
            + DIALOG_ACTION_BOTTOM_SPACE)
            .max(DIALOG_CONTAINER_MIN_HEIGHT);

        Size::new(width, height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let [headline, supporting_text, actions] = children else {
            return vec![];
        };

        let text_width = DIALOG_CONTENT_PADDING
            .mul_add(-2.0, bounds.width())
            .max(0.0);
        let headline_size = headline
            .measure(ProposalSize::new(Some(text_width), None))
            .size;
        let supporting_size = supporting_text
            .measure(ProposalSize::new(Some(text_width), None))
            .size;
        let action_size = actions.measure(ProposalSize::new(None, None)).size;
        let headline_origin = Point::new(
            bounds.x() + DIALOG_CONTENT_PADDING,
            bounds.y() + DIALOG_CONTENT_PADDING,
        );
        let supporting_origin = Point::new(
            bounds.x() + DIALOG_CONTENT_PADDING,
            headline_origin.y + headline_size.height + DIALOG_HEADLINE_BODY_SPACING,
        );
        let actions_origin = Point::new(
            bounds.x() + bounds.width() - DIALOG_ACTION_TRAILING_SPACE - action_size.width,
            supporting_origin.y
                + supporting_size.height
                + DIALOG_CONTENT_BOTTOM_SPACE_WITH_ACTIONS
                + DIALOG_ACTION_TOP_SPACE,
        );

        vec![
            Rect::new(headline_origin, Size::new(text_width, headline_size.height)),
            Rect::new(
                supporting_origin,
                Size::new(text_width, supporting_size.height),
            ),
            Rect::new(actions_origin, action_size),
        ]
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .first()
            .and_then(|child| child.explicit_horizontal(alignment))
    }
}

struct DialogActions<Actions> {
    actions: Actions,
}

impl<Actions> View for DialogActions<Actions>
where
    Actions: TupleViews + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        waterui::component::hstack(self.actions).spacing(DIALOG_ACTION_SPACING)
    }
}

const fn noop(_env: &Environment) {}

/// Creates a Material Design 3 dialog.
#[must_use]
pub fn dialog(headline: impl IntoLabel, supporting_text: impl IntoLabel) -> Dialog<()> {
    Dialog::new(headline, supporting_text)
}

/// Creates a Material Design 3 dialog action.
#[must_use]
pub fn dialog_action(label: impl IntoLabel) -> DialogAction {
    DialogAction::new(label)
}

#[cfg(test)]
mod tests {
    use super::{
        DIALOG_ACTION_BOTTOM_SPACE, DIALOG_ACTION_SPACING, DIALOG_ACTION_TRAILING_SPACE,
        DIALOG_CONTAINER_MAX_WIDTH, DIALOG_CONTAINER_MIN_HEIGHT, DIALOG_CONTAINER_MIN_WIDTH,
        DIALOG_CONTAINER_SHAPE, DIALOG_CONTENT_BOTTOM_SPACE_WITH_ACTIONS, DIALOG_CONTENT_PADDING,
        DIALOG_HIDDEN_OFFSET_Y, DIALOG_VIEWPORT_PADDING,
    };

    #[test]
    fn dialog_tokens_match_mdui_2_1_5() {
        assert_eq!(DIALOG_CONTAINER_MIN_WIDTH, 280.0);
        assert_eq!(DIALOG_CONTAINER_MAX_WIDTH, 560.0);
        assert_eq!(DIALOG_CONTAINER_MIN_HEIGHT, 140.0);
        assert_eq!(DIALOG_CONTAINER_SHAPE, 28.0);
        assert_eq!(DIALOG_VIEWPORT_PADDING, 48.0);
        assert_eq!(DIALOG_HIDDEN_OFFSET_Y, -30.0);
        assert_eq!(DIALOG_CONTENT_PADDING, 24.0);
        assert_eq!(DIALOG_CONTENT_BOTTOM_SPACE_WITH_ACTIONS, 8.0);
        assert_eq!(DIALOG_ACTION_SPACING, 8.0);
        assert_eq!(DIALOG_ACTION_TRAILING_SPACE, 24.0);
        assert_eq!(DIALOG_ACTION_BOTTOM_SPACE, 24.0);
    }
}
