//! The toggle looks of the theme contract (water-rs/waterui#1434): what a
//! theme returns for a switch ([`SwitchLook`]) and for a checkbox
//! ([`CheckboxLook`]).
//!
//! A toggle's value is a `bool`, so every part of its control is a
//! [`Custom`] part over it: its [`Driven`] values read the value, the
//! interaction state and, for a switch, the drag, and each carries the
//! [`ValueMotion`](super::ValueMotion) its value changes run with. The knob's
//! position, the track's fade, the thumb's scale and the check glyph are
//! operands of the recording the engine animates, so a toggle, a hover, a
//! press or a drag records nothing and walks no tree.
//!
//! A toggle's parts record into one live layer, once, and keep that
//! recording while the toggle's look and frame are unchanged. A part cannot
//! declare a backdrop material: a layered recording is a draw-time context a
//! live layer does not have.

use std::rc::Rc;

use kurbo::{Rect, Size, Vec2};
use waterui_controls::ControlSize;
use waterui_controls::toggle::ToggleStyle;
use waterui_core::layout::LayoutDirection;

use super::look::{Custom, CustomPart, DrivenSet, HitRegion, Motion};

/// What Hydrolysis asks a theme's [`switch`](super::WidgetTheme::switch) or
/// [`checkbox`](super::WidgetTheme::checkbox) look for: everything
/// structural about one toggle.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`](super::Driven) values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToggleSpec {
    style: ToggleStyle,
    size: ControlSize,
    direction: LayoutDirection,
}

impl ToggleSpec {
    /// The spec of a `style` toggle of `size`, laid out `direction`.
    #[must_use]
    pub const fn new(style: ToggleStyle, size: ControlSize, direction: LayoutDirection) -> Self {
        Self {
            style,
            size,
            direction,
        }
    }

    /// The toggle's resolved style: never `Automatic`.
    #[must_use]
    pub const fn style(&self) -> ToggleStyle {
        self.style
    }

    /// The toggle's control size.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// The layout direction the toggle is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }
}

#[derive(Clone)]
struct ControlParts {
    size: Size,
    label_spacing: f64,
    motion: Motion,
    hit: HitRegion,
    parts: Vec<Rc<dyn CustomPart<bool>>>,
}

/// The control a toggle draws beside its label: its size, the parts it is
/// drawn from, the [`Motion`] its values run with and where it takes touches.
///
/// Hydrolysis measures, places, hit-tests, records and animates the toggle
/// from this one value, so its layout and its drawing cannot diverge. The
/// parts record in the order they are added: the later one is above.
#[derive(Clone)]
pub struct ToggleControl(Rc<ControlParts>);

impl core::fmt::Debug for ToggleControl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToggleControl")
            .field("size", &self.0.size)
            .field("label_spacing", &self.0.label_spacing)
            .field("motion", &self.0.motion)
            .field("hit", &self.0.hit)
            .field("parts", &self.0.parts.len())
            .finish()
    }
}

impl ToggleControl {
    /// A control of `size` whose values run with `motion`, with no parts,
    /// no spacing to its label and a hit region of its own frame.
    #[must_use]
    pub fn new(size: Size, motion: Motion) -> Self {
        Self(Rc::new(ControlParts {
            size,
            label_spacing: 0.0,
            motion,
            hit: HitRegion::NONE,
            parts: Vec::new(),
        }))
    }

    fn parts_mut(&mut self) -> &mut ControlParts {
        Rc::make_mut(&mut self.0)
    }

    /// Spaces the control `spacing` from its label.
    #[must_use]
    pub fn label_spacing(mut self, spacing: f64) -> Self {
        self.parts_mut().label_spacing = spacing;
        self
    }

    /// Takes touches in `hit`, around the control's frame.
    #[must_use]
    pub fn hit(mut self, hit: HitRegion) -> Self {
        self.parts_mut().hit = hit;
        self
    }

    /// Records `part` above the parts already added. It draws in the
    /// widget's own space; its frame is the control's.
    #[must_use]
    pub fn part<D: DrivenSet<bool> + 'static>(mut self, part: Custom<bool, D>) -> Self
    where
        D::Live: 'static,
    {
        self.parts_mut().parts.push(Rc::new(part));
        self
    }

    /// The control's size: what the toggle measures its control at.
    #[must_use]
    pub fn size(&self) -> Size {
        self.0.size
    }

    /// The space between the control and the label.
    #[must_use]
    pub fn get_label_spacing(&self) -> f64 {
        self.0.label_spacing
    }

    /// The motion the control's values run with.
    #[must_use]
    pub fn motion(&self) -> &Motion {
        &self.0.motion
    }

    /// Where the control takes touches.
    #[must_use]
    pub fn get_hit(&self) -> &HitRegion {
        &self.0.hit
    }

    /// The touch target of a control whose frame is `frame`: the frame grown
    /// to the hit region's minimum, about its centre.
    #[must_use]
    pub fn target(&self, frame: Rect) -> Rect {
        self.0.hit.target(frame, frame)
    }

    /// The parts, bottom first.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn parts(&self) -> &[Rc<dyn CustomPart<bool>>] {
        &self.0.parts
    }

    /// Whether `other` is this control.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn same(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// How a switch's knob follows a drag: its travel, and where a release
/// settles it.
///
/// A drag moves the knob by the touch's horizontal offset over
/// [`travel`](Self::travel), from the end the value puts it at, mirrored in a
/// right-to-left layout. A release settles it on the nearer end — the value
/// the knob is nearer to, on at the half way point. The look's
/// [`Driven`](super::Driven) values read the same mapping, so the knob it
/// draws and the value the release sets cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchDrag {
    travel: f64,
    direction: LayoutDirection,
}

impl SwitchDrag {
    /// A drag over `travel` points of horizontal movement, in a layout
    /// running `direction`.
    #[must_use]
    pub const fn new(travel: f64, direction: LayoutDirection) -> Self {
        Self { travel, direction }
    }

    /// The distance the knob travels between its ends.
    #[must_use]
    pub const fn travel(&self) -> f64 {
        self.travel
    }

    /// The knob's position between its ends — `0` at the off end, `1` at the
    /// on end — for a switch showing `value` under `drag`: the end `value`
    /// puts it at, moved by the drag and held within the ends.
    #[must_use]
    pub fn progress(&self, value: bool, drag: Option<Vec2>) -> f64 {
        let rest = if value { 1.0 } else { 0.0 };
        let Some(drag) = drag else {
            return rest;
        };
        let toward_on = match self.direction {
            LayoutDirection::LeftToRight => drag.x,
            LayoutDirection::RightToLeft => -drag.x,
        };
        (rest + toward_on / self.travel).clamp(0.0, 1.0)
    }

    /// The value a release settles on after `drag` of a switch showing
    /// `value`: on when the knob is at or beyond the half way point.
    #[must_use]
    pub fn settles_on(&self, value: bool, drag: Vec2) -> bool {
        self.progress(value, Some(drag)) >= 0.5
    }
}

/// How a switch looks and responds: its [`ToggleControl`] and the
/// [`SwitchDrag`] that maps a drag of its knob to a value.
///
/// The control's touch target is also where a drag begins; a touch that moves
/// farther than the hit region's slop horizontally drags the knob, one that
/// moves no farther than it taps, and one released beyond the cancel
/// distance does neither.
#[derive(Debug, Clone)]
pub struct SwitchLook {
    control: ToggleControl,
    drag: SwitchDrag,
}

impl SwitchLook {
    /// A switch drawn as `control` whose knob follows `drag`.
    #[must_use]
    pub const fn new(control: ToggleControl, drag: SwitchDrag) -> Self {
        Self { control, drag }
    }

    /// The switch's control.
    #[must_use]
    pub const fn control(&self) -> &ToggleControl {
        &self.control
    }

    /// The drag mapping.
    #[must_use]
    pub const fn drag(&self) -> &SwitchDrag {
        &self.drag
    }
}

/// How a checkbox looks and responds: its [`ToggleControl`].
#[derive(Debug, Clone)]
pub struct CheckboxLook {
    control: ToggleControl,
}

impl CheckboxLook {
    /// A checkbox drawn as `control`.
    #[must_use]
    pub const fn new(control: ToggleControl) -> Self {
        Self { control }
    }

    /// The checkbox's control.
    #[must_use]
    pub const fn control(&self) -> &ToggleControl {
        &self.control
    }
}
