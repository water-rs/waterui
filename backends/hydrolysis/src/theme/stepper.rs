//! The stepper look of the theme contract (water-rs/waterui#1434): what a
//! theme returns for a stepper ([`StepperLook`]).
//!
//! A stepper is a pair of buttons, and a theme may join the pair into one
//! silhouette: the far corners fully round, the corners either side of the
//! seam tucked in. Each end is therefore a [`Chrome`] of its own, with its
//! own asymmetric corners and press morph, the icon a custom part of it and
//! its state a function of the end's own inputs: an end at the limit of the
//! stepper's range is disabled.
//!
//! Hydrolysis measures, places, hit-tests and records the stepper from this
//! one look, so the geometry it measures is the geometry it draws at every
//! height it is laid out in, and a press, a hover or a value change moves
//! live operands and records nothing.

use kurbo::Size;
use waterui_controls::ControlSize;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;

use super::Chrome;
use super::look::SpecTint;

/// What Hydrolysis asks a theme's [`stepper`](super::WidgetTheme::stepper)
/// look for: everything structural about one stepper.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`](super::Driven) values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepperSpec {
    size: ControlSize,
    direction: LayoutDirection,
    tint: SpecTint,
}

impl StepperSpec {
    /// The spec of a stepper of `size`, laid out `direction`, drawn with
    /// `tint`.
    #[must_use]
    pub const fn new(size: ControlSize, direction: LayoutDirection, tint: SpecTint) -> Self {
        Self {
            size,
            direction,
            tint,
        }
    }

    /// The stepper's control size.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// The layout direction the stepper is laid out in. The decrement end
    /// leads the pair: it lies at the left in a left-to-right layout and at
    /// the right in a right-to-left one, and the look draws its corners
    /// accordingly.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The stepper's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// How a stepper looks and responds: the [`Chrome`] of each end, the size of
/// the ends and the spacing around them.
///
/// Each end's chrome draws that end's container, its icon (a custom part of
/// the chrome, drawn over the container and beneath the state layer) and its
/// interaction feedback. The two ends lie side by side, [`spacing`] apart,
/// at the stepper's trailing edge, each [`button`] in size and centred in
/// the row; a row shorter than the buttons caps their height and never
/// changes their width, so the controls are as wide as they were measured.
///
/// [`spacing`]: Self::spacing
/// [`button`]: Self::button
#[derive(Debug, Clone)]
pub struct StepperLook {
    decrement: Chrome,
    increment: Chrome,
    button: Size,
    spacing: f64,
    label_spacing: f64,
}

impl StepperLook {
    /// A stepper whose `decrement` and `increment` ends are each `button` in
    /// size, with no spacing between them and none to the label.
    #[must_use]
    pub const fn new(decrement: Chrome, increment: Chrome, button: Size) -> Self {
        Self {
            decrement,
            increment,
            button,
            spacing: 0.0,
            label_spacing: 0.0,
        }
    }

    /// Spaces the two ends `spacing` apart.
    #[must_use]
    pub const fn spacing(mut self, spacing: f64) -> Self {
        self.spacing = spacing;
        self
    }

    /// Spaces the ends `spacing` from the label.
    #[must_use]
    pub const fn label_spacing(mut self, spacing: f64) -> Self {
        self.label_spacing = spacing;
        self
    }

    /// The chrome of the decrement end: the pair's leading one.
    #[must_use]
    pub const fn decrement(&self) -> &Chrome {
        &self.decrement
    }

    /// The chrome of the increment end: the pair's trailing one.
    #[must_use]
    pub const fn increment(&self) -> &Chrome {
        &self.increment
    }

    /// The size of each end.
    #[must_use]
    pub const fn button(&self) -> Size {
        self.button
    }

    /// The space between the two ends.
    #[must_use]
    pub const fn get_spacing(&self) -> f64 {
        self.spacing
    }

    /// The space between the label and the ends.
    #[must_use]
    pub const fn get_label_spacing(&self) -> f64 {
        self.label_spacing
    }

    /// The size of the two ends together: what the stepper measures its
    /// controls at.
    #[must_use]
    pub const fn controls(&self) -> Size {
        Size::new(
            self.button.width.mul_add(2.0, self.spacing),
            self.button.height,
        )
    }
}
