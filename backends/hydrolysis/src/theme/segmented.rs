//! The segmented picker look of the theme contract (water-rs/waterui#4539):
//! what a theme returns for a segmented picker ([`SegmentedLook`]).
//!
//! A segmented picker has two kinds of parts. A control part draws over
//! the whole row and is a [`Custom`] part over the picker's
//! [`SegmentedValue`]: the selected index and the thumb's position, which
//! equals the selected index at rest, follows the touch during a drag and
//! moves to a new index under the look's [`Motion::value`]. A segment part
//! is recorded once per segment, on that segment's frame, and is a
//! [`Custom`] part over its [`Segment`]: its index and the segment count.
//!
//! A control part's [`Inputs::state`](super::Inputs) is the thumb's state:
//! the selected segment's hover, press and focus, plus `DRAGGED` while the
//! thumb is dragged and `DISABLED`. A segment part's is the segment's own
//! hover, press and focus, plus `SELECTED` while it is the selected segment
//! and `DISABLED`. A selection change retargets the operands under the
//! look's motion and snaps the segments' `SELECTED` flags; it records
//! nothing.
//!
//! The look draws left to right. Hydrolysis mirrors every part about the
//! row in a right-to-left layout and places the labels in the mirrored
//! segments.

use std::rc::Rc;
use std::time::Duration;

use kurbo::{Rect, Vec2};
use nami::Computed;
use waterui_controls::ControlSize;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;
use waterui_text::font::Font;

use super::look::{Custom, CustomPart, DrivenSet, HitRegion, Motion, SpecTint};
use super::{Chrome, CornerShape, Driven, FocusIndicator, PressFeedback, StateLayer, Surface};

/// What Hydrolysis asks a theme's
/// [`segmented_picker`](super::WidgetTheme::segmented_picker) look for:
/// everything structural about one segmented picker.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`] values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentedSpec {
    size: ControlSize,
    segment_count: usize,
    direction: LayoutDirection,
    tint: SpecTint,
}

impl SegmentedSpec {
    /// The spec of a `size` picker of `segment_count` segments, laid out
    /// `direction`, drawn with `tint`.
    #[must_use]
    pub const fn new(
        size: ControlSize,
        segment_count: usize,
        direction: LayoutDirection,
        tint: SpecTint,
    ) -> Self {
        Self {
            size,
            segment_count,
            direction,
            tint,
        }
    }

    /// The picker's control size.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// How many segments the picker has: one per item.
    #[must_use]
    pub const fn segment_count(&self) -> usize {
        self.segment_count
    }

    /// The layout direction the picker is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The picker's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// The whole control's value: what a control part follows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentedValue {
    /// The index of the selected segment, counted from the leading one.
    pub selected: usize,
    /// Where the thumb is, in segments from the leading one: `selected` at
    /// rest, between two segments while a drag or a selection change moves
    /// it.
    pub position: f64,
}

/// One segment's value, fixed while the spec is: what a segment part
/// follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// The segment's index, counted from the leading one.
    pub index: usize,
    /// How many segments the picker has.
    pub count: usize,
}

impl Segment {
    /// Whether this is the leading segment.
    #[must_use]
    pub const fn is_first(&self) -> bool {
        self.index == 0
    }

    /// Whether this is the trailing segment.
    #[must_use]
    pub const fn is_last(&self) -> bool {
        self.index + 1 == self.count
    }
}

/// The horizontal and vertical measures a segmented picker is laid out by.
/// Hydrolysis measures and places the picker from these alone, so its size
/// and its drawing cannot diverge.
///
/// The picker is a label band above a row of equal segments, each holding
/// its item's label.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SegmentedLayout {
    /// The row's least height: the tallest label and its vertical padding
    /// grow it beyond this.
    pub row_height: f64,
    /// The room around a label inside its segment: `x` on each side, `y`
    /// above and below.
    pub label_padding: Vec2,
    /// The narrowest a segment is measured, its padding included.
    pub minimum_segment_width: f64,
    /// The narrowest the picker is measured.
    pub minimum_width: f64,
    /// The space between the picker's label above the row and the row.
    pub label_spacing: f64,
    /// How far the parts draw beyond the row on every side, so a group
    /// opacity layer does not clip them.
    pub reach: f64,
}

/// How a segmented picker styles each segment's label: its font and its
/// colour, both per segment.
///
/// The font follows the segment's selected state: Hydrolysis lays a label
/// out again only when that state changes, and measures each label at the
/// wider of its selected and unselected fonts, so the picker's size never
/// depends on the selection. The colour is live: it follows the segment's
/// hover, press and focus as an operand and snaps on a selection change.
/// Either applies to the parts of the item's text that set none of their
/// own.
#[derive(Debug, Clone)]
pub struct SegmentLabel {
    font: Driven<Font, Segment>,
    color: Driven<Color, Segment>,
}

impl SegmentLabel {
    /// Labels in `font` and `color`.
    #[must_use]
    pub fn new(
        font: impl Into<Driven<Font, Segment>>,
        color: impl Into<Driven<Color, Segment>>,
    ) -> Self {
        Self {
            font: font.into(),
            color: color.into(),
        }
    }

    /// The labels' font.
    #[must_use]
    pub const fn get_font(&self) -> &Driven<Font, Segment> {
        &self.font
    }

    /// The labels' colour.
    #[must_use]
    pub const fn get_color(&self) -> &Driven<Color, Segment> {
        &self.color
    }
}

/// How the thumb follows a drag, and where a release settles it.
///
/// The thumb follows a drag that starts on the selected segment and settles
/// on the nearest segment at release. A drag never cancels. The look's
/// [`Driven`] values read the same position through
/// [`SegmentedValue::position`], so the thumb it draws and the segment the
/// release selects cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentedDrag {
    direction: LayoutDirection,
}

impl SegmentedDrag {
    /// A drag in a layout running `direction`.
    #[must_use]
    pub const fn new(direction: LayoutDirection) -> Self {
        Self { direction }
    }

    /// The thumb's position, in segments from the leading one, for a picker
    /// of `count` segments `segment_width` wide showing `selected` under
    /// `drag`: `selected` moved by the drag's horizontal offset over the
    /// segment width, mirrored in a right-to-left layout, and held within
    /// the segments.
    #[must_use]
    pub fn position(
        &self,
        selected: usize,
        drag: Option<Vec2>,
        segment_width: f64,
        count: usize,
    ) -> f64 {
        let rest = segments(selected);
        let Some(drag) = drag.filter(|_| segment_width > 0.0) else {
            return rest;
        };
        let toward_trailing = match self.direction {
            LayoutDirection::LeftToRight => drag.x,
            LayoutDirection::RightToLeft => -drag.x,
        };
        let last = segments(count.saturating_sub(1));
        (rest + toward_trailing / segment_width).clamp(0.0, last)
    }

    /// The segment a release settles on with the thumb at `position` in a
    /// picker of `count` segments: the nearest one.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the rounded position is a whole, non-negative number of segments, far below usize::MAX"
    )]
    pub fn settles_on(&self, position: f64, count: usize) -> usize {
        let last = count.saturating_sub(1);
        (position.round().max(0.0) as usize).min(last)
    }
}

/// A count of segments as a position along the row.
#[expect(
    clippy::cast_precision_loss,
    reason = "a segment count is far below 2^53"
)]
const fn segments(count: usize) -> f64 {
    count as f64
}

/// What gives a segment its chrome, from the segment and its live selected
/// flag.
type SegmentChrome = dyn Fn(Segment, Computed<bool>) -> Chrome;

/// How a segmented picker looks and responds: the measures it is laid out
/// by, its label styling, its parts, its drag and its group opacity.
///
/// Hydrolysis measures, places, hit-tests, records and animates the picker
/// from this one value. The parts record in this order: the control parts
/// added with [`Self::part`], then for each segment its chrome set with
/// [`Self::segment_chrome`] and its parts added with [`Self::segment_part`],
/// the labels, then the control parts added with [`Self::over_labels`]. A
/// backdrop material recorded over the labels captures them. Within each
/// kind, the part added later is above.
#[derive(Clone)]
pub struct SegmentedLook {
    layout: SegmentedLayout,
    label: SegmentLabel,
    /// What Hydrolysis binds interaction with — the motion and the touch
    /// region — drawing nothing of its own.
    chrome: Chrome,
    drag: Option<SegmentedDrag>,
    below: Vec<Rc<dyn CustomPart<SegmentedValue>>>,
    segment_chrome: Option<Rc<SegmentChrome>>,
    segment: Vec<Rc<dyn CustomPart<Segment>>>,
    above: Vec<Rc<dyn CustomPart<SegmentedValue>>>,
    group_opacity: Driven<f32>,
}

impl core::fmt::Debug for SegmentedLook {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SegmentedLook")
            .field("layout", &self.layout)
            .field("label", &self.label)
            .field("motion", self.chrome.motion())
            .field("hit", self.chrome.get_hit())
            .field("drag", &self.drag)
            .field("below", &self.below.len())
            .field("segment_chrome", &self.segment_chrome.is_some())
            .field("segment", &self.segment.len())
            .field("above", &self.above.len())
            .field("group_opacity", &self.group_opacity)
            .finish()
    }
}

impl SegmentedLook {
    /// A picker laid out by `layout`, its labels styled by `label` and its
    /// values running with `motion`. It draws nothing until parts are
    /// added, takes touches on each segment's frame, takes taps only and is
    /// drawn opaque.
    #[must_use]
    pub fn new(layout: SegmentedLayout, label: SegmentLabel, motion: Motion) -> Self {
        let chrome = Chrome::new(
            Surface::clear(CornerShape::radius(0.0)),
            StateLayer::new(Color::new(waterui::theme::color::Foreground)),
            PressFeedback::new(),
            FocusIndicator::Tint,
            motion,
        )
        .hit(HitRegion::NONE.press_delay(Duration::ZERO));
        Self {
            layout,
            label,
            chrome,
            drag: None,
            below: Vec::new(),
            segment_chrome: None,
            segment: Vec::new(),
            above: Vec::new(),
            group_opacity: Driven::constant(1.0),
        }
    }

    /// Takes touches in `hit`, per segment: each segment's frame grown to
    /// the region's minimum about its centre, and held within the
    /// segment's own column so neighbouring targets never overlap.
    #[must_use]
    pub fn hit(mut self, hit: HitRegion) -> Self {
        self.chrome = self.chrome.hit(hit);
        self
    }

    /// Lets the thumb follow a drag that starts on the selected segment,
    /// settling as `drag` decides. A look with no drag takes taps only.
    #[must_use]
    pub const fn drag(mut self, drag: SegmentedDrag) -> Self {
        self.drag = Some(drag);
        self
    }

    /// Records the control part `part` below the labels, above the control
    /// parts already added below them. Its frame is the row.
    #[must_use]
    pub fn part<D: DrivenSet<SegmentedValue> + 'static>(
        mut self,
        part: Custom<SegmentedValue, D>,
    ) -> Self
    where
        D::Live: 'static,
    {
        self.below.push(Rc::new(part));
        self
    }

    /// Each segment's chrome — surface, state layer, press feedback, focus
    /// indicator — realized by Hydrolysis on that segment's frame the way a
    /// button's chrome is (one ripple wave per press, from the press point).
    /// Evaluated once per segment while the spec holds, with the segment
    /// and its live selected flag: `true` while it is the selected segment.
    /// A colour built from the flag — a state layer that differs on the
    /// selected segment — updates the realized chrome in place.
    ///
    /// The chrome follows the segment's own hover, press, focus and press
    /// point, `SELECTED` while it is the selected segment and `DISABLED`,
    /// and is mirrored with the other parts in a right-to-left layout. It
    /// records above the control parts added with [`Self::part`] and below
    /// the segment's parts added with [`Self::segment_part`]. Its hit
    /// region does not apply: each segment takes touches as [`Self::hit`]
    /// sets.
    #[must_use]
    pub fn segment_chrome(
        mut self,
        chrome: impl Fn(Segment, Computed<bool>) -> Chrome + 'static,
    ) -> Self {
        self.segment_chrome = Some(Rc::new(chrome));
        self
    }

    /// Records `part` once per segment, below the labels and above the
    /// control parts added with [`Self::part`] and the segment's chrome.
    /// Its frame is the segment.
    #[must_use]
    pub fn segment_part<D: DrivenSet<Segment> + 'static>(mut self, part: Custom<Segment, D>) -> Self
    where
        D::Live: 'static,
    {
        self.segment.push(Rc::new(part));
        self
    }

    /// Records the control part `part` over the labels, above the parts
    /// already added over them. Its frame is the row; a backdrop material it
    /// records captures the labels.
    #[must_use]
    pub fn over_labels<D: DrivenSet<SegmentedValue> + 'static>(
        mut self,
        part: Custom<SegmentedValue, D>,
    ) -> Self
    where
        D::Live: 'static,
    {
        self.above.push(Rc::new(part));
        self
    }

    /// Draws the whole control — its parts and its labels — as one group at
    /// `opacity`: a disabled picker that fades as a whole. An opacity that
    /// varies with the thumb's hover, press or focus follows the operand
    /// live; one that varies with the disabled state applies at the next
    /// render.
    #[must_use]
    pub fn group_opacity(mut self, opacity: impl Into<Driven<f32>>) -> Self {
        self.group_opacity = opacity.into();
        self
    }

    /// The measures the picker is laid out by.
    #[must_use]
    pub const fn layout(&self) -> &SegmentedLayout {
        &self.layout
    }

    /// The label styling.
    #[must_use]
    pub const fn label(&self) -> &SegmentLabel {
        &self.label
    }

    /// The motion the look's values run with.
    #[must_use]
    pub fn motion(&self) -> &Motion {
        self.chrome.motion()
    }

    /// The drag, when the thumb follows one.
    #[must_use]
    pub const fn get_drag(&self) -> Option<SegmentedDrag> {
        self.drag
    }

    /// The group opacity.
    #[must_use]
    pub const fn get_group_opacity(&self) -> &Driven<f32> {
        &self.group_opacity
    }

    /// Where a segment laid out on `frame` takes touches: the frame grown
    /// to the hit region's minimum about its centre, held within the
    /// segment's column.
    #[must_use]
    pub fn target(&self, frame: Rect) -> Rect {
        let grown = self.chrome.get_hit().target(frame, frame);
        Rect::new(frame.x0, grown.y0, frame.x1, grown.y1)
    }

    /// The chrome Hydrolysis binds the picker's interaction with.
    #[cfg(hydrolysis_runtime)]
    pub(crate) const fn chrome(&self) -> &Chrome {
        &self.chrome
    }

    /// The control parts below the labels, in paint order.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn below(&self) -> &[Rc<dyn CustomPart<SegmentedValue>>] {
        &self.below
    }

    /// Each segment's chrome, leading first, given each one's live selected
    /// flag of `selected`; empty without a segment chrome.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn segment_chromes(&self, selected: &[Computed<bool>]) -> Vec<Chrome> {
        let count = selected.len();
        self.segment_chrome
            .as_ref()
            .map_or_else(Vec::new, |chrome| {
                selected
                    .iter()
                    .enumerate()
                    .map(|(index, flag)| chrome(Segment { index, count }, flag.clone()))
                    .collect()
            })
    }

    /// The parts recorded once per segment, in paint order.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn segments(&self) -> &[Rc<dyn CustomPart<Segment>>] {
        &self.segment
    }

    /// The control parts over the labels, in paint order.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn above(&self) -> &[Rc<dyn CustomPart<SegmentedValue>>] {
        &self.above
    }
}

#[cfg(test)]
mod tests {
    use kurbo::Vec2;
    use waterui_core::layout::LayoutDirection;

    use super::{Segment, SegmentedDrag};

    #[test]
    fn a_segment_knows_its_ends() {
        let first = Segment { index: 0, count: 3 };
        let middle = Segment { index: 1, count: 3 };
        let last = Segment { index: 2, count: 3 };
        assert!(first.is_first() && !first.is_last());
        assert!(!middle.is_first() && !middle.is_last());
        assert!(!last.is_first() && last.is_last());
    }

    /// The thumb rests on the selection, follows the drag by segment widths,
    /// mirrored in a right-to-left layout, and stays within the row.
    #[test]
    fn the_drag_moves_the_thumb_by_segment_widths() {
        let ltr = SegmentedDrag::new(LayoutDirection::LeftToRight);
        let rtl = SegmentedDrag::new(LayoutDirection::RightToLeft);
        approx::assert_relative_eq!(ltr.position(1, None, 50.0, 3), 1.0);
        approx::assert_relative_eq!(ltr.position(1, Some(Vec2::new(25.0, 9.0)), 50.0, 3), 1.5);
        approx::assert_relative_eq!(rtl.position(1, Some(Vec2::new(25.0, 0.0)), 50.0, 3), 0.5);
        approx::assert_relative_eq!(ltr.position(1, Some(Vec2::new(500.0, 0.0)), 50.0, 3), 2.0);
        approx::assert_relative_eq!(ltr.position(1, Some(Vec2::new(-500.0, 0.0)), 50.0, 3), 0.0);
    }

    #[test]
    fn a_release_settles_on_the_nearest_segment() {
        let drag = SegmentedDrag::new(LayoutDirection::LeftToRight);
        assert_eq!(drag.settles_on(0.4, 3), 0);
        assert_eq!(drag.settles_on(0.6, 3), 1);
        assert_eq!(drag.settles_on(2.0, 3), 2);
        assert_eq!(drag.settles_on(7.0, 3), 2);
    }
}
