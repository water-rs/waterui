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
//! A toggle's parts record into live layers, once, and keep those
//! recordings while the toggle's look and frame are unchanged. A control may
//! declare a [`ToggleBackdrop`] stage among its parts: the parts added before
//! it record below the backdrop material, the parts added after it above.

use std::rc::Rc;

use cherenkov_record::ShapeData;
use kurbo::{Rect, Size, Vec2};
use waterui_controls::ControlSize;
use waterui_controls::toggle::ToggleStyle;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;

#[cfg(hydrolysis_runtime)]
use super::look::RealizedCustom;
use super::look::{Custom, CustomPart, DrivenSet, HitRegion, Inputs, Motion, SpecTint};
use super::{Backdrop, Driven};

/// What Hydrolysis asks a theme's [`switch`](super::WidgetTheme::switch) or
/// [`checkbox`](super::WidgetTheme::checkbox) look for: everything
/// structural about one toggle.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`](super::Driven) values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToggleSpec {
    style: ToggleStyle,
    size: ControlSize,
    direction: LayoutDirection,
    tint: SpecTint,
}

impl ToggleSpec {
    /// The spec of a `style` toggle of `size`, laid out `direction`, drawn
    /// with `tint`.
    #[must_use]
    pub const fn new(
        style: ToggleStyle,
        size: ControlSize,
        direction: LayoutDirection,
        tint: SpecTint,
    ) -> Self {
        Self {
            style,
            size,
            direction,
            tint,
        }
    }

    /// The toggle's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
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

/// A backdrop material a toggle's control draws between its parts.
///
/// It is a live shape the material covers and the [`Backdrop`] it is made
/// of; the backdrop's effect ([`Backdrop::effect`]) retargets as the toggle's
/// inputs change. A switch's pressed knob is the reason: it becomes a glass
/// lens over the track, below the knob's own fill.
#[derive(Debug, Clone)]
pub struct ToggleBackdrop {
    shape: Driven<ShapeData, bool>,
    material: Backdrop,
}

impl ToggleBackdrop {
    /// The `material` over `shape`, in the widget's own space.
    #[must_use]
    pub const fn new(shape: Driven<ShapeData, bool>, material: Backdrop) -> Self {
        Self { shape, material }
    }

    /// The shape the material covers.
    #[must_use]
    pub const fn shape(&self) -> &Driven<ShapeData, bool> {
        &self.shape
    }

    /// The material.
    #[must_use]
    pub const fn material(&self) -> &Backdrop {
        &self.material
    }
}

#[derive(Clone)]
struct ControlParts {
    size: Size,
    reach: f64,
    label_spacing: f64,
    motion: Motion,
    hit: HitRegion,
    group_opacity: Driven<f32, bool>,
    /// The parts recorded below the backdrop, bottom first; every part when
    /// the control has no backdrop.
    below: Vec<Rc<dyn CustomPart<bool>>>,
    backdrop: Option<ToggleBackdrop>,
    /// The parts recorded above the backdrop, bottom first.
    above: Vec<Rc<dyn CustomPart<bool>>>,
}

/// The control a toggle draws beside its label: its size, the parts it is
/// drawn from, the [`Motion`] its values run with and where it takes touches.
///
/// Hydrolysis measures, places, hit-tests, records and animates the toggle
/// from this one value, so its layout and its drawing cannot diverge. The
/// parts record in the order they are added: the later one is above. A
/// [`ToggleBackdrop`] added among them splits them in two layers, the parts
/// before it below the material and the parts after it above.
#[derive(Clone)]
pub struct ToggleControl(Rc<ControlParts>);

impl core::fmt::Debug for ToggleControl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ToggleControl")
            .field("size", &self.0.size)
            .field("reach", &self.0.reach)
            .field("label_spacing", &self.0.label_spacing)
            .field("motion", &self.0.motion)
            .field("hit", &self.0.hit)
            .field("group_opacity", &self.0.group_opacity)
            .field("below", &self.0.below.len())
            .field("backdrop", &self.0.backdrop)
            .field("above", &self.0.above.len())
            .finish()
    }
}

impl ToggleControl {
    /// A control of `size` whose values run with `motion`, with no parts,
    /// no spacing to its label, no reach beyond its frame, a hit region of
    /// its own frame and drawn opaque.
    #[must_use]
    pub fn new(size: Size, motion: Motion) -> Self {
        Self(Rc::new(ControlParts {
            size,
            reach: 0.0,
            label_spacing: 0.0,
            motion,
            hit: HitRegion::NONE,
            group_opacity: Driven::constant(1.0),
            below: Vec::new(),
            backdrop: None,
            above: Vec::new(),
        }))
    }

    /// Declares that the parts draw up to `reach` beyond the control's frame
    /// on every side — an overhanging track, a grown press lens — so a group
    /// opacity layer does not clip them.
    #[must_use]
    pub fn reach(mut self, reach: f64) -> Self {
        self.parts_mut().reach = reach;
        self
    }

    /// Draws the whole control as one group at `opacity` — a disabled switch
    /// that fades as a whole, its overlapping parts blending once. An
    /// opacity that varies with hover, press, focus or the value follows the
    /// operand live; one that varies with the disabled state applies at the
    /// next render.
    #[must_use]
    pub fn group_opacity(mut self, opacity: impl Into<Driven<f32, bool>>) -> Self {
        self.parts_mut().group_opacity = opacity.into();
        self
    }

    /// Records `backdrop` above the parts already added: the parts added
    /// before it record below the material, those added after it above.
    ///
    /// # Panics
    ///
    /// Panics when the control already has a backdrop: it has one stage.
    #[must_use]
    pub fn backdrop(mut self, backdrop: ToggleBackdrop) -> Self {
        let parts = self.parts_mut();
        assert!(
            parts.backdrop.is_none(),
            "a toggle control has one backdrop stage"
        );
        parts.backdrop = Some(backdrop);
        self
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
        let parts = self.parts_mut();
        let layer = if parts.backdrop.is_some() {
            &mut parts.above
        } else {
            &mut parts.below
        };
        layer.push(Rc::new(part));
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

    /// How far the parts draw beyond the control's frame.
    #[must_use]
    pub fn get_reach(&self) -> f64 {
        self.0.reach
    }

    /// The group opacity.
    #[must_use]
    pub fn get_group_opacity(&self) -> &Driven<f32, bool> {
        &self.0.group_opacity
    }

    /// The backdrop stage, when the control has one.
    #[must_use]
    pub fn get_backdrop(&self) -> Option<&ToggleBackdrop> {
        self.0.backdrop.as_ref()
    }

    /// The parts recorded below the backdrop, bottom first, realized at
    /// `inputs`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn realize_below(&self, inputs: &Inputs<'_, bool>) -> Vec<RealizedCustom<bool>> {
        self.0
            .below
            .iter()
            .map(|part| part.realize(inputs))
            .collect()
    }

    /// The parts recorded above the backdrop, bottom first, realized at
    /// `inputs`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn realize_above(&self, inputs: &Inputs<'_, bool>) -> Vec<RealizedCustom<bool>> {
        self.0
            .above
            .iter()
            .map(|part| part.realize(inputs))
            .collect()
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
