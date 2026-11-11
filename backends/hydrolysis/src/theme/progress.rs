//! The progress looks of the theme contract (water-rs/waterui#1434): what a
//! theme returns for a linear progress bar ([`LinearProgressLook`]), a
//! circular progress indicator and a loading indicator ([`IndicatorLook`]).
//!
//! A progress indicator's value is an `f64` normalized to `0.0..=1.0`, so
//! every part of its control is a [`Custom`] part over it: its [`Driven`]
//! values read the value and carry the [`ValueMotion`](super::ValueMotion)
//! their changes run with, so a value change moves the recorded operands and
//! records nothing. The motion of an indeterminate indicator is the engine's:
//! a part declares a [`Driven::repeating`] value for the operand that moves
//! and [`Custom::rotating`] for the turn of its whole layer, and a running
//! indicator records nothing and does no Hydrolysis work per frame.
//!
//! A determinate and an indeterminate indicator are two looks: the spec says
//! which one the indicator is, and a spec change requests the look again.
//! The value a part reads is held to `0.0..=1.0`.

use std::rc::Rc;

use kurbo::{Rect, Size};
use waterui::component::progress::ProgressStyle;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;

use super::look::{Custom, CustomPart, DrivenSet, Motion, SpecTint};
#[cfg(hydrolysis_runtime)]
use super::look::{Inputs, RealizedCustom};

/// What Hydrolysis asks a theme's
/// [`linear_progress`](super::WidgetTheme::linear_progress),
/// [`circular_progress`](super::WidgetTheme::circular_progress) or
/// [`loading_indicator`](super::WidgetTheme::loading_indicator) look for:
/// everything structural about one indicator.
///
/// A spec change requests the look again; the value reaches the look
/// through its [`Driven`](super::Driven) values.
#[expect(
    clippy::struct_excessive_bools,
    reason = "the spec is plain data: four independent structural facts of one indicator"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProgressSpec {
    style: ProgressStyle,
    determinate: bool,
    label: bool,
    value_label: bool,
    four_color: bool,
    direction: LayoutDirection,
    tint: SpecTint,
}

impl ProgressSpec {
    /// The spec of a `style` indicator, `determinate` when its value is a
    /// number and indeterminate when it is not, laid out `direction` and
    /// drawn with `tint`.
    ///
    /// It has no label and no value label, and cycles one colour; see
    /// [`Self::with_label`], [`Self::with_value_label`] and
    /// [`Self::with_four_color`].
    #[must_use]
    pub const fn new(
        style: ProgressStyle,
        determinate: bool,
        direction: LayoutDirection,
        tint: SpecTint,
    ) -> Self {
        Self {
            style,
            determinate,
            label: false,
            value_label: false,
            four_color: false,
            direction,
            tint,
        }
    }

    /// Whether the indicator has a label: a linear indicator reserves a band
    /// for it above the bar.
    #[must_use]
    pub const fn with_label(mut self, label: bool) -> Self {
        self.label = label;
        self
    }

    /// Whether the indicator has a value label: a determinate linear
    /// indicator reserves a band for it below the bar.
    #[must_use]
    pub const fn with_value_label(mut self, value_label: bool) -> Self {
        self.value_label = value_label;
        self
    }

    /// Whether an indeterminate indicator cycles through the theme's four
    /// accent colours instead of drawing in the tint.
    #[must_use]
    pub const fn with_four_color(mut self, four_color: bool) -> Self {
        self.four_color = four_color;
        self
    }

    /// The indicator's style.
    #[must_use]
    pub const fn style(&self) -> ProgressStyle {
        self.style
    }

    /// Whether the indicator shows how far an operation is, rather than
    /// that one is under way. A loading indicator never does.
    #[must_use]
    pub const fn determinate(&self) -> bool {
        self.determinate
    }

    /// Whether the indicator draws a label.
    #[must_use]
    pub const fn label(&self) -> bool {
        self.label
    }

    /// Whether the indicator draws a value label.
    #[must_use]
    pub const fn value_label(&self) -> bool {
        self.value_label
    }

    /// Whether an indeterminate indicator cycles four colours.
    #[must_use]
    pub const fn four_color(&self) -> bool {
        self.four_color
    }

    /// The layout direction the indicator is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The indicator's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// The drawn part of a progress indicator: its size, the parts it is drawn
/// from and the [`Motion`] its value changes run with.
///
/// The parts record in the order they are added, the later one above; each
/// is its own engine layer, so a part that [rotates](Custom::rotating)
/// turns alone.
#[derive(Clone)]
pub struct ProgressControl(Rc<ControlParts>);

#[derive(Clone)]
struct ControlParts {
    size: Size,
    motion: Motion,
    parts: Vec<Rc<dyn CustomPart<f64>>>,
}

impl core::fmt::Debug for ProgressControl {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProgressControl")
            .field("size", &self.0.size)
            .field("motion", &self.0.motion)
            .field("parts", &self.0.parts.len())
            .finish()
    }
}

impl ProgressControl {
    /// A control of `size` whose value changes run with `motion`, with no
    /// parts. Only the motion's [`value`](Motion::value) is read: an
    /// indicator takes no hover, focus, press or drag.
    ///
    /// A linear indicator's control is the bar: its width is the narrowest
    /// the bar is laid out at, and the bar fills the width it is given. A
    /// circular or loading indicator's control is its square.
    #[must_use]
    pub fn new(size: Size, motion: Motion) -> Self {
        Self(Rc::new(ControlParts {
            size,
            motion,
            parts: Vec::new(),
        }))
    }

    /// Records `part` above the parts already added. It draws in the
    /// indicator's own space; its frame is the control's.
    #[must_use]
    pub fn part<D: DrivenSet<f64> + 'static>(mut self, part: Custom<f64, D>) -> Self
    where
        D::Live: 'static,
    {
        Rc::make_mut(&mut self.0).parts.push(Rc::new(part));
        self
    }

    /// The control's size.
    #[must_use]
    pub fn size(&self) -> Size {
        self.0.size
    }

    /// The motion the control's values run with.
    #[must_use]
    pub fn motion(&self) -> &Motion {
        &self.0.motion
    }

    /// The parts, bottom first, realized at `inputs`.
    #[cfg(hydrolysis_runtime)]
    pub(crate) fn realize(&self, inputs: &Inputs<'_, f64>) -> Vec<RealizedCustom<f64>> {
        self.0
            .parts
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

/// How a circular progress indicator or a loading indicator looks: its
/// [`ProgressControl`], drawn in a square.
#[derive(Debug, Clone)]
pub struct IndicatorLook {
    control: ProgressControl,
}

impl IndicatorLook {
    /// An indicator drawn as `control`.
    #[must_use]
    pub const fn new(control: ProgressControl) -> Self {
        Self { control }
    }

    /// The indicator's control.
    #[must_use]
    pub const fn control(&self) -> &ProgressControl {
        &self.control
    }

    /// The control's frame in an indicator laid out in `bounds`: the largest
    /// square about their centre.
    #[must_use]
    pub fn frame(&self, bounds: Rect) -> Rect {
        let side = bounds.width().min(bounds.height());
        Rect::from_center_size(bounds.center(), Size::new(side, side))
    }
}

/// How a linear progress indicator looks: the [`ProgressControl`] of its
/// bar, and the bands for its labels around it.
///
/// Hydrolysis measures, places, records and animates the indicator from this
/// one value, and measures and places the bands from the same
/// [`layout`](Self::layout), so what it measures is what it draws: a band is
/// reserved only for a label the indicator has.
#[derive(Debug, Clone)]
pub struct LinearProgressLook {
    control: ProgressControl,
    inset: f64,
    label_height: f64,
    label_spacing: f64,
    value_label_spacing: f64,
}

impl LinearProgressLook {
    /// A linear indicator whose bar is `control`, inset `inset` from each
    /// side, with no minimum label height and no spacing around the labels.
    #[must_use]
    pub const fn new(control: ProgressControl, inset: f64) -> Self {
        Self {
            control,
            inset,
            label_height: 0.0,
            label_spacing: 0.0,
            value_label_spacing: 0.0,
        }
    }

    /// Reserves a label band at least `height` tall above the bar, with
    /// `spacing` between the band and the bar.
    #[must_use]
    pub const fn label_band(mut self, height: f64, spacing: f64) -> Self {
        self.label_height = height;
        self.label_spacing = spacing;
        self
    }

    /// Reserves a value-label band below the bar, `spacing` beneath it.
    #[must_use]
    pub const fn value_label_band(mut self, spacing: f64) -> Self {
        self.value_label_spacing = spacing;
        self
    }

    /// The bar's control.
    #[must_use]
    pub const fn control(&self) -> &ProgressControl {
        &self.control
    }

    /// How far the bar is inset from each side.
    #[must_use]
    pub const fn get_inset(&self) -> f64 {
        self.inset
    }

    /// The minimum height of the label band.
    #[must_use]
    pub const fn get_label_height(&self) -> f64 {
        self.label_height
    }

    /// The space between the label band and the bar.
    #[must_use]
    pub const fn get_label_spacing(&self) -> f64 {
        self.label_spacing
    }

    /// The space between the bar and the value-label band.
    #[must_use]
    pub const fn get_value_label_spacing(&self) -> f64 {
        self.value_label_spacing
    }

    /// Where the bar and the labels lie in an indicator of `width` whose
    /// label is `label` tall and value label `value_label` tall, each
    /// `None` when the indicator has none. The origin is the indicator's.
    #[must_use]
    pub fn layout(&self, width: f64, label: Option<f64>, value_label: Option<f64>) -> LinearLayout {
        let label_band = label.map(|height| height.max(self.label_height));
        let bar_top = label_band.map_or(0.0, |height| height + self.label_spacing);
        let bar = Rect::new(
            self.inset,
            bar_top,
            width - self.inset,
            bar_top + self.control.size().height,
        );
        let value_label_band = value_label.map(|height| {
            let top = bar.y1 + self.value_label_spacing;
            Rect::new(0.0, top, width, top + height.max(self.label_height))
        });
        LinearLayout {
            label: label_band.map(|height| Rect::new(0.0, 0.0, width, height)),
            bar,
            value_label: value_label_band,
            height: value_label_band.map_or(bar.y1, |band| band.y1),
        }
    }

    /// The narrowest an indicator is laid out at: the bar's minimum width
    /// with its insets.
    #[must_use]
    pub fn minimum_width(&self) -> f64 {
        self.inset.mul_add(2.0, self.control.size().width)
    }
}

/// The geometry of a linear progress indicator, in its own space: from
/// [`LinearProgressLook::layout`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinearLayout {
    /// The label band, when the indicator has a label.
    pub label: Option<Rect>,
    /// The bar: the control's frame.
    pub bar: Rect,
    /// The value-label band, when the indicator has a value label.
    pub value_label: Option<Rect>,
    /// The indicator's height.
    pub height: f64,
}
