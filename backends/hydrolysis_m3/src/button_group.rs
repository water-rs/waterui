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
use waterui::layout::{
    Layout, LayoutInvalidationCallback, Point, ProposalSize, Rect, Size, SubView,
    container::FixedContainer,
};
use waterui::prelude::dynamic::watch;
use waterui::reactive::watcher::BoxWatcherGuard;
use waterui::reactive::{Signal as _, SignalExt as _, binding, zip};
use waterui::shape::{ShapeExt as _, UnevenRoundedRectangle};
use waterui::{AnyView, Binding, Environment, View, ViewExt as _};
use waterui_controls::button::Button;
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

// ---------------------------------------------------------------------------
// Standard (spaced) button group
// ---------------------------------------------------------------------------

/// `ButtonGroupSmallTokens.BetweenSpace`.
const GROUP_BETWEEN_SPACE: f32 = 12.0;
/// `ButtonGroupDefaults.ExpandedRatio`: a pressed button reaches for 15% of its
/// own width, split evenly across its two sides.
const GROUP_EXPANDED_RATIO: f32 = 0.15;

/// One button in a [`ButtonGroup`].
pub struct GroupButton {
    label: Label,
    action: BoxedAction<()>,
    compression_limit: f32,
}

impl Debug for GroupButton {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GroupButton")
            .field("label", &self.label)
            .field("compression_limit", &self.compression_limit)
            .finish_non_exhaustive()
    }
}

impl GroupButton {
    /// Creates a button for a group.
    #[must_use]
    pub fn new<F, Args>(label: impl IntoLabel, action: F) -> Self
    where
        F: Handler<Args, ()> + 'static,
    {
        Self {
            label: label.into_label(),
            action: Box::new(boxed_action(action)),
            compression_limit: SEGMENT_HORIZONTAL_SPACE,
        }
    }

    /// Caps how far a neighbour's press may squeeze this button.
    ///
    /// Compose defaults this to `0.dp`, which makes the whole interaction inert
    /// — nothing can give, so nothing can grow. The default here is the button's
    /// own horizontal content space, which is the room it actually has to lose.
    #[must_use]
    pub const fn compression_limit(mut self, limit: f32) -> Self {
        self.compression_limit = limit;
        self
    }
}

/// Creates a button for a [`ButtonGroup`].
#[must_use]
pub fn group_button<F, Args>(label: impl IntoLabel, action: F) -> GroupButton
where
    F: Handler<Args, ()> + 'static,
{
    GroupButton::new(label, action)
}

/// A Material Design 3 button group.
///
/// Unlike a [`ConnectedButtonGroup`], the buttons stand apart and each keeps its
/// own shape. What binds them is the press: holding one widens it into the room
/// its neighbours give up, so the row reads as a single elastic strip.
#[derive(Debug)]
pub struct ButtonGroup {
    buttons: Vec<GroupButton>,
}

impl ButtonGroup {
    /// Creates an empty group.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buttons: Vec::new(),
        }
    }

    /// Adds a button.
    #[must_use]
    pub fn button(mut self, button: GroupButton) -> Self {
        self.buttons.push(button);
        self
    }

    /// Adds several buttons.
    #[must_use]
    pub fn buttons(mut self, buttons: impl IntoIterator<Item = GroupButton>) -> Self {
        self.buttons.extend(buttons);
        self
    }
}

impl Default for ButtonGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl View for ButtonGroup {
    fn body(self, _env: &Environment) -> impl View {
        // The group owns one press flag per button. They are the layout's
        // inputs, so they belong to the group rather than to the buttons: a
        // press changes three widths, not one.
        let pressed: Vec<Binding<bool>> = self.buttons.iter().map(|_| binding(false)).collect();
        let limits: Vec<f32> = self
            .buttons
            .iter()
            .map(|button| button.compression_limit)
            .collect();
        let layout = ButtonGroupLayout {
            pressed: pressed.clone(),
            limits,
        };
        let children = self
            .buttons
            .into_iter()
            .zip(pressed)
            .map(|(button, pressed)| {
                let mut action = button.action;
                AnyView::new(
                    // A group button keeps the default filled-tonal chrome:
                    // the press widens its container, so it needs one.
                    Button::new(button.label)
                        .gesture(DragGesture::new(0.0), move |env: Environment| {
                            let phase = env
                                .get::<DragEvent>()
                                .expect("group button gesture is missing its DragEvent")
                                .phase;
                            pressed.set(matches!(
                                phase,
                                GesturePhase::Started | GesturePhase::Updated
                            ));
                        })
                        .on_tap(move |env: Environment| action(&env)),
                )
            })
            .collect::<Vec<_>>();
        FixedContainer::from_parts(Box::new(layout), children).a11y_role(AccessibilityRole::Group)
    }
}

/// Creates a Material Design 3 button group.
#[must_use]
pub const fn button_group() -> ButtonGroup {
    ButtonGroup::new()
}

/// `ButtonGroupMeasurePolicy`: a pressed button grows by exactly the width its
/// neighbours give up, so the row's total width never changes.
///
/// A pressed button reaches for `ExpandedRatio * width / 2` on each side, capped
/// by what that side's neighbour is willing to lose and by how wide the
/// neighbour is at all. A button at either end has only one neighbour and so
/// grows by half as much.
struct ButtonGroupLayout {
    pressed: Vec<Binding<bool>>,
    limits: Vec<f32>,
}

impl Debug for ButtonGroupLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ButtonGroupLayout")
            .field("buttons", &self.pressed.len())
            .finish_non_exhaustive()
    }
}

impl ButtonGroupLayout {
    /// The intrinsic width of every child, before any press moves them.
    fn resting_widths(children: &[&dyn SubView]) -> Vec<f32> {
        children
            .iter()
            .map(|child| child.measure(ProposalSize::UNSPECIFIED).size.width)
            .collect()
    }

    /// Applies the press expansion to `widths` in place.
    fn expand(&self, widths: &mut [f32]) {
        let count = widths.len();
        for index in 0..count {
            if !self.pressed[index].get() {
                continue;
            }
            let reach = GROUP_EXPANDED_RATIO * widths[index] / 2.0;
            let mut growth = 0.0;
            for neighbour in [
                index.checked_sub(1),
                (index + 1 < count).then_some(index + 1),
            ]
            .into_iter()
            .flatten()
            {
                let limit = self.limits[neighbour];
                let taken = reach.min(limit).min(widths[neighbour]).max(0.0);
                widths[neighbour] -= taken;
                growth += taken;
            }
            widths[index] += growth;
        }
    }

    /// The height the row stands at: the tallest child, floored by the tokens.
    fn row_height(children: &[&dyn SubView]) -> f32 {
        children
            .iter()
            .map(|child| child.measure(ProposalSize::UNSPECIFIED).size.height)
            .fold(CONTAINER_HEIGHT, f32::max)
    }
}

impl Layout for ButtonGroupLayout {
    fn size_that_fits(&self, _proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let widths = Self::resting_widths(children);
        // A press only moves width between children, so the row measures the
        // same whether or not one is held.
        let gap_count = u16::try_from(children.len().saturating_sub(1))
            .expect("a button group holds a sane number of buttons");
        let gaps = GROUP_BETWEEN_SPACE * f32::from(gap_count);
        Size::new(
            widths.iter().sum::<f32>() + gaps,
            Self::row_height(children),
        )
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let mut widths = Self::resting_widths(children);
        self.expand(&mut widths);
        let height = Self::row_height(children).min(bounds.height());
        let top = (bounds.height() - height).mul_add(0.5, bounds.y());
        let mut x = bounds.x();
        widths
            .into_iter()
            .map(|width| {
                let frame = Rect::new(Point::new(x, top), Size::new(width, height));
                x += width + GROUP_BETWEEN_SPACE;
                frame
            })
            .collect()
    }

    fn watch_invalidation(&self, invalidate: LayoutInvalidationCallback) -> Vec<BoxWatcherGuard> {
        // Press state is a layout input here, not a paint one: it decides how
        // wide each button is, so a press has to re-run layout.
        self.pressed
            .iter()
            .map(|pressed| {
                let invalidate = invalidate.clone();
                pressed.watch(move |_| invalidate())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BETWEEN_SPACE, ButtonGroupLayout, CONTAINER_HEIGHT, GROUP_BETWEEN_SPACE,
        GROUP_EXPANDED_RATIO, INNER_CORNER_RADIUS, OUTER_CORNER_RADIUS,
        PRESSED_INNER_CORNER_RADIUS, SegmentPosition, inner_radius, normalized,
    };
    use waterui::reactive::binding;

    /// Builds a layout over `widths.len()` buttons, with `pressed` held down.
    fn layout(limits: &[f32], pressed: Option<usize>) -> ButtonGroupLayout {
        ButtonGroupLayout {
            pressed: limits
                .iter()
                .enumerate()
                .map(|(index, _)| binding(Some(index) == pressed))
                .collect(),
            limits: limits.to_vec(),
        }
    }

    /// Values from `androidx.compose.material3.tokens.ButtonGroupSmallTokens`
    /// and `ButtonGroupDefaults`.
    #[test]
    fn button_group_tokens_match_compose_button_group_defaults() {
        assert_eq!(GROUP_BETWEEN_SPACE, 12.0);
        assert_eq!(GROUP_EXPANDED_RATIO, 0.15);
    }

    /// A held middle button takes from both neighbours, and the row's total
    /// width is unchanged — the press moves space, it does not create it.
    #[test]
    fn a_pressed_middle_button_grows_by_what_both_neighbours_give_up() {
        let mut widths = [100.0, 100.0, 100.0];
        let before: f32 = widths.iter().sum();
        layout(&[16.0, 16.0, 16.0], Some(1)).expand(&mut widths);

        // reach = 0.15 * 100 / 2 = 7.5, under the 16dp limit, so both sides
        // give 7.5 and the pressed button grows by 15.
        assert_eq!(widths[0], 92.5);
        assert_eq!(widths[1], 115.0);
        assert_eq!(widths[2], 92.5);
        assert_eq!(widths.iter().sum::<f32>(), before);
    }

    /// A button at either end has one neighbour, so it grows by half as much.
    #[test]
    fn a_pressed_end_button_only_leans_on_the_neighbour_it_has() {
        let mut widths = [100.0, 100.0, 100.0];
        layout(&[16.0, 16.0, 16.0], Some(0)).expand(&mut widths);

        assert_eq!(widths[0], 107.5);
        assert_eq!(widths[1], 92.5);
        assert_eq!(widths[2], 100.0);
    }

    /// A neighbour never gives up more than its compression limit, however wide
    /// the pressed button is.
    #[test]
    fn a_neighbours_compression_limit_caps_what_it_gives_up() {
        let mut widths = [100.0, 400.0, 100.0];
        // reach is 0.15 * 400 / 2 = 30, well past the 4dp the neighbours allow.
        layout(&[4.0, 4.0, 4.0], Some(1)).expand(&mut widths);

        assert_eq!(widths[0], 96.0);
        assert_eq!(widths[1], 408.0);
        assert_eq!(widths[2], 96.0);
    }

    /// With nothing held, every button keeps the width it measured at.
    #[test]
    fn an_untouched_group_leaves_every_width_alone() {
        let mut widths = [80.0, 120.0];
        layout(&[16.0, 16.0], None).expand(&mut widths);

        assert_eq!(widths, [80.0, 120.0]);
    }

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
