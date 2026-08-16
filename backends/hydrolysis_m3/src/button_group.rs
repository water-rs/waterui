//! Material Design 3 connected button group composed from `WaterUI`
//! primitives.
//!
//! A connected group is a row of related choices that share one silhouette:
//! round on the outside, tucked in where segments meet. The corners are the
//! component — they say the segments belong together, and they move to say
//! which one is selected or held.

use core::fmt::{self, Debug};

use waterui::accessibility::{AccessibilityRole, AccessibilityState};
use waterui::color::Color;
use waterui::gesture::{DragEvent, DragGesture, GesturePhase};
use waterui::layout::padding::EdgeInsets;
use waterui::prelude::dynamic::watch;
use waterui::reactive::{SignalExt as _, binding, zip};
use waterui::shape::{ShapeExt as _, UnevenRoundedRectangle};
use waterui::{Binding, Environment, View, ViewExt as _};
use waterui_controls::label::{IntoLabel, Label};
use waterui_core::handler::{BoxedAction, Handler, boxed_action};

use crate::color::{OnSecondaryContainer, OnSurface, SecondaryContainer, SurfaceContainer};
use crate::semantics::{conditional_color, interaction_style};

/// `ConnectedButtonGroupSmallTokens.ContainerHeight`.
const CONTAINER_HEIGHT: f32 = 40.0;
/// `ConnectedButtonGroupSmallTokens.BetweenSpace`.
const BETWEEN_SPACE: f32 = 2.0;
/// `ConnectedButtonGroupSmallTokens.InnerCornerCornerSize`,
/// `ShapeTokens.CornerValueSmall`.
const INNER_CORNER_RADIUS: f32 = 8.0;
/// `ConnectedButtonGroupSmallTokens.PressedInnerCornerCornerSize`,
/// `ShapeTokens.CornerValueExtraSmall`. Holding a segment tightens its inner
/// corners.
const PRESSED_INNER_CORNER_RADIUS: f32 = 4.0;
/// The outer corners are `CornerFull`, which on this container is half its
/// height. `SelectedInnerCornerCornerSizePercent` is 50, so a selected
/// segment's inner corners round the same amount and it reads as detached.
const OUTER_CORNER_RADIUS: f32 = CONTAINER_HEIGHT / 2.0;
/// `ButtonGroupSmallTokens.LeadingSpace` for the segment's own padding.
const SEGMENT_HORIZONTAL_SPACE: f32 = 16.0;

/// Corner radii normalize against the shorter side, and every segment is
/// exactly `CONTAINER_HEIGHT` tall.
const fn normalized(radius: f32) -> f32 {
    radius / CONTAINER_HEIGHT
}

/// The inner corner radius of a segment given its state.
///
/// Selected wins over pressed: a selected segment reads as lifted out of the
/// group, and tightening its corners under a press would fight that.
const fn inner_radius(selected: bool, pressed: bool) -> f32 {
    if selected {
        OUTER_CORNER_RADIUS
    } else if pressed {
        PRESSED_INNER_CORNER_RADIUS
    } else {
        INNER_CORNER_RADIUS
    }
}

/// Where a segment sits in the group, which decides which corners are outer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentPosition {
    /// The only segment: every corner is an outer corner.
    Only,
    /// First: outer on the leading edge, tucked on the trailing.
    Leading,
    /// Between two others: tucked on both edges.
    Middle,
    /// Last: tucked on the leading edge, outer on the trailing.
    Trailing,
}

impl SegmentPosition {
    const fn of(index: usize, count: usize) -> Self {
        match (index, count) {
            (_, 0 | 1) => Self::Only,
            (0, _) => Self::Leading,
            (index, count) if index + 1 == count => Self::Trailing,
            _ => Self::Middle,
        }
    }

    /// The four corner radii for this position, given the inner radius its
    /// state currently calls for.
    const fn radii(self, inner: f32) -> (f32, f32) {
        match self {
            Self::Only => (OUTER_CORNER_RADIUS, OUTER_CORNER_RADIUS),
            Self::Leading => (OUTER_CORNER_RADIUS, inner),
            Self::Middle => (inner, inner),
            Self::Trailing => (inner, OUTER_CORNER_RADIUS),
        }
    }
}

/// One choice in a [`ConnectedButtonGroup`].
pub struct ConnectedButton {
    label: Label,
    selected: Binding<bool>,
    action: BoxedAction<()>,
}

impl Debug for ConnectedButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectedButton")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

impl ConnectedButton {
    /// Creates a segment.
    #[must_use]
    pub fn new<F, Args>(label: impl IntoLabel, selected: &Binding<bool>, action: F) -> Self
    where
        F: Handler<Args, ()> + 'static,
    {
        Self {
            label: label.into_label(),
            selected: selected.clone(),
            action: Box::new(boxed_action(action)),
        }
    }
}

/// Creates a connected button group segment.
#[must_use]
pub fn connected_button<F, Args>(
    label: impl IntoLabel,
    selected: &Binding<bool>,
    action: F,
) -> ConnectedButton
where
    F: Handler<Args, ()> + 'static,
{
    ConnectedButton::new(label, selected, action)
}

/// A Material Design 3 connected button group.
#[derive(Debug)]
pub struct ConnectedButtonGroup {
    segments: Vec<ConnectedButton>,
}

impl ConnectedButtonGroup {
    /// Creates an empty group.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Adds a segment.
    #[must_use]
    pub fn segment(mut self, segment: ConnectedButton) -> Self {
        self.segments.push(segment);
        self
    }

    /// Adds several segments.
    #[must_use]
    pub fn segments(mut self, segments: impl IntoIterator<Item = ConnectedButton>) -> Self {
        self.segments.extend(segments);
        self
    }
}

impl Default for ConnectedButtonGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl View for ConnectedButtonGroup {
    fn body(self, _env: &Environment) -> impl View {
        let count = self.segments.len();
        let segments = self
            .segments
            .into_iter()
            .enumerate()
            .map(|(index, segment)| {
                let position = SegmentPosition::of(index, count);
                let mut action = segment.action;
                let selected = segment.selected;
                // Press state is the segment's own, and it only changes corners
                // and the state layer, so it lives here rather than in the
                // caller's model.
                let pressed = binding(false);
                let pressed_for_gesture = pressed.clone();

                let shape = zip::zip(selected.clone(), pressed).map(move |(selected, pressed)| {
                    let (leading, trailing) = position.radii(inner_radius(selected, pressed));
                    UnevenRoundedRectangle::new(
                        normalized(leading),
                        normalized(trailing),
                        normalized(leading),
                        normalized(trailing),
                    )
                });
                let container =
                    conditional_color(selected.clone(), SecondaryContainer, SurfaceContainer);
                let content = conditional_color(selected.clone(), OnSecondaryContainer, OnSurface);
                let accessibility_state =
                    selected.map(|selected| AccessibilityState::new().selected(selected));

                segment
                    .label
                    .foreground(content.clone())
                    .padding_with(EdgeInsets::new(
                        0.0,
                        0.0,
                        SEGMENT_HORIZONTAL_SPACE,
                        SEGMENT_HORIZONTAL_SPACE,
                    ))
                    .height(CONTAINER_HEIGHT)
                    .background(ReactiveSegmentShape {
                        shape,
                        color: container,
                    })
                    // The tap is what activates the segment, and it is also
                    // what makes it a real interaction target — a view carrying
                    // only a gesture emits no accessibility node, so a
                    // gesture-only segment would be invisible to assistive
                    // technology. The drag gesture rides alongside purely to
                    // track the press that tightens the inner corners.
                    .gesture(DragGesture::new(0.0), move |env: Environment| {
                        let phase = env
                            .get::<DragEvent>()
                            .expect("connected button gesture is missing its DragEvent")
                            .phase;
                        pressed_for_gesture.set(matches!(
                            phase,
                            GesturePhase::Started | GesturePhase::Updated
                        ));
                    })
                    .on_tap(move |env: Environment| action(&env))
                    .a11y_role(AccessibilityRole::Button)
                    .a11y_state_signal(accessibility_state)
                    .install(interaction_style(content, f64::from(OUTER_CORNER_RADIUS)))
            })
            .collect::<Vec<_>>();

        waterui::component::HStack::new(
            waterui::layout::stack::VerticalAlignment::Center,
            BETWEEN_SPACE,
            segments,
        )
    }
}

/// A segment's container, whose corners follow its selected and pressed state.
///
/// This is one of the few places `watch` is the right tool. It replaces the
/// subtree it watches, which normally loses any state that subtree owns — but
/// here that subtree is a single stateless fill, so there is nothing to lose.
/// The alternative, a morph, interpolates between exactly two shapes, and a
/// segment moves between three (resting, pressed, selected).
#[derive(Debug)]
struct ReactiveSegmentShape<S> {
    shape: S,
    color: Color,
}

impl<S> View for ReactiveSegmentShape<S>
where
    S: waterui::Signal<Output = UnevenRoundedRectangle> + 'static,
{
    fn body(self, _env: &Environment) -> impl View {
        let color = self.color;
        watch(self.shape, move |shape| shape.fill(color.clone()))
    }
}

/// Creates a Material Design 3 connected button group.
#[must_use]
pub const fn connected_button_group() -> ConnectedButtonGroup {
    ConnectedButtonGroup::new()
}

#[cfg(test)]
mod tests {
    use super::{
        BETWEEN_SPACE, CONTAINER_HEIGHT, INNER_CORNER_RADIUS, OUTER_CORNER_RADIUS,
        PRESSED_INNER_CORNER_RADIUS, SegmentPosition, inner_radius, normalized,
    };

    /// Values from `ConnectedButtonGroupSmallTokens`.
    #[test]
    fn connected_group_tokens_match_compose_button_group_tokens() {
        assert_eq!(CONTAINER_HEIGHT, 40.0);
        assert_eq!(BETWEEN_SPACE, 2.0);
        // InnerCornerCornerSize is ShapeTokens.CornerValueSmall.
        assert_eq!(INNER_CORNER_RADIUS, 8.0);
        // PressedInnerCornerCornerSize is CornerValueExtraSmall.
        assert_eq!(PRESSED_INNER_CORNER_RADIUS, 4.0);
        // CornerFull on a 40dp container, which is also the 50% a selected
        // segment's inner corners take.
        assert_eq!(OUTER_CORNER_RADIUS, 20.0);
    }

    /// Only the edges of the group are round; everything between is tucked in.
    #[test]
    fn only_the_groups_outer_edges_are_fully_round() {
        let inner = INNER_CORNER_RADIUS;

        // A lone segment is a normal pill.
        assert_eq!(
            SegmentPosition::of(0, 1).radii(inner),
            (OUTER_CORNER_RADIUS, OUTER_CORNER_RADIUS)
        );
        // In a row of three, only the ends keep an outer corner.
        assert_eq!(
            SegmentPosition::of(0, 3).radii(inner),
            (OUTER_CORNER_RADIUS, inner)
        );
        assert_eq!(SegmentPosition::of(1, 3).radii(inner), (inner, inner));
        assert_eq!(
            SegmentPosition::of(2, 3).radii(inner),
            (inner, OUTER_CORNER_RADIUS)
        );
    }

    /// Selection outranks a press: a selected segment stays detached rather
    /// than tightening under the finger.
    #[test]
    fn selection_outranks_a_press_on_the_inner_corners() {
        assert_eq!(inner_radius(false, false), INNER_CORNER_RADIUS);
        assert_eq!(inner_radius(false, true), PRESSED_INNER_CORNER_RADIUS);
        assert_eq!(inner_radius(true, false), OUTER_CORNER_RADIUS);
        assert_eq!(inner_radius(true, true), OUTER_CORNER_RADIUS);
    }

    /// Every radius the group uses stays inside the range a normalized corner
    /// can express; above 0.5 it would clamp and the tucked corners would
    /// silently round as much as the outer ones.
    #[test]
    fn every_corner_radius_normalizes_within_range() {
        for radius in [
            INNER_CORNER_RADIUS,
            PRESSED_INNER_CORNER_RADIUS,
            OUTER_CORNER_RADIUS,
        ] {
            let normalized = normalized(radius);
            assert!(
                normalized > 0.0 && normalized <= 0.5,
                "{radius} -> {normalized}"
            );
        }
    }
}
