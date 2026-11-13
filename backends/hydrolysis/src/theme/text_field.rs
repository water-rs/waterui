//! The text field look of the theme contract (water-rs/waterui#1434): what
//! a theme returns for a text field and for a secure field
//! ([`TextFieldLook`]).
//!
//! A field is a container with an inside label, the text it edits, a caret
//! and a selection. The look declares each of them: the container as a
//! [`Chrome`] (its focus indicator included), the active indicator along its
//! bottom edge as a [`Driven`] over the interaction state, the placeholder
//! and the floating label as [`Label`]s, and the caret and the selection as
//! paints. The label floats on a motion of its own, and the caret blinks on
//! one.
//!
//! Hydrolysis measures, places and records the field from this one look, so
//! the geometry it measures is the geometry it draws, and a hover, a focus
//! change or the label floating moves live operands and records nothing.

use cherenkov_record::Paint;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;

use super::look::SpecTint;
use super::{Border, Chrome, Driven, InputFieldMetrics, Label, TextCaretMotion, ValueMotion};

/// What Hydrolysis asks a theme's [`text_field`](super::WidgetTheme::text_field)
/// look for: everything structural about one field.
///
/// A text field and a secure field ask with the same spec and share the
/// look. A spec change requests the look again; everything else reaches the
/// look through its [`Driven`] values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextFieldSpec {
    direction: LayoutDirection,
    tint: SpecTint,
}

impl TextFieldSpec {
    /// The spec of a field laid out `direction`, drawn with `tint`.
    #[must_use]
    pub const fn new(direction: LayoutDirection, tint: SpecTint) -> Self {
        Self { direction, tint }
    }

    /// The layout direction the field is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The field's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live: a tint signal's change
    /// reaches the look's drawing without requesting the look again.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// The inside label of a field: how it is styled, how far it floats and the
/// motion it floats by.
///
/// The label rests centred in the container and floats to the top, scaled
/// down, while the field is focused or holds content. Its [`Label`] styles
/// the label's font and colour; the colour follows the field's interaction
/// state.
#[derive(Debug, Clone)]
pub struct FloatingLabel {
    label: Label,
    float: ValueMotion,
    scale: f64,
    top: f64,
}

impl FloatingLabel {
    /// A label styled by `label` that floats by `float`: the motion of the
    /// label moving between resting and floating. It floats at full size
    /// ([`Self::scale`]) along the container's top edge ([`Self::top`]).
    #[must_use]
    pub const fn new(label: Label, float: ValueMotion) -> Self {
        Self {
            label,
            float,
            scale: 1.0,
            top: 0.0,
        }
    }

    /// Scales the floating label to `scale` of its resting size.
    #[must_use]
    pub const fn scale(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }

    /// Floats the label's band `top` below the container's top edge.
    #[must_use]
    pub const fn top(mut self, top: f64) -> Self {
        self.top = top;
        self
    }

    /// How the label is styled.
    #[must_use]
    pub const fn label(&self) -> &Label {
        &self.label
    }

    /// The motion of the label floating and resting.
    #[must_use]
    pub const fn get_float(&self) -> &ValueMotion {
        &self.float
    }

    /// The floating label's size relative to its resting size.
    #[must_use]
    pub const fn get_scale(&self) -> f64 {
        self.scale
    }

    /// The distance from the container's top edge to the floating label's
    /// band.
    #[must_use]
    pub const fn get_top(&self) -> f64 {
        self.top
    }
}

/// The caret of a focused field: its paint and its blink.
#[derive(Debug, Clone)]
pub struct Caret {
    paint: Driven<Paint>,
    blink: TextCaretMotion,
}

impl Caret {
    /// A caret painted with `paint` that blinks by `blink`. The paint is
    /// drawn at the blink's opacity.
    #[must_use]
    pub fn new(paint: impl Into<Driven<Paint>>, blink: TextCaretMotion) -> Self {
        Self {
            paint: paint.into(),
            blink,
        }
    }

    /// The caret's paint.
    #[must_use]
    pub const fn paint(&self) -> &Driven<Paint> {
        &self.paint
    }

    /// The caret's blink.
    #[must_use]
    pub const fn blink(&self) -> TextCaretMotion {
        self.blink
    }
}

/// How a text field looks and responds: its container, active indicator,
/// labels, caret and selection, and the metrics it lays out by.
///
/// The container's [`Chrome`] draws the field's surface, its state layer and
/// its focus indicator. The active indicator is a line along the container's
/// bottom edge drawn over the surface, beneath the state layer; its
/// [`Border`] gives the line's width and paint for each state, so a field
/// can thicken and recolour it on focus.
#[derive(Debug, Clone)]
pub struct TextFieldLook {
    container: Chrome,
    metrics: InputFieldMetrics,
    active_indicator: Option<Driven<Border>>,
    placeholder: Label,
    value: Option<Label>,
    label: FloatingLabel,
    caret: Caret,
    selection: Driven<Paint>,
}

impl TextFieldLook {
    /// A field whose `container` lies behind the text, laid out by
    /// `metrics`, with an inside `label`, a `caret` and a `selection`
    /// painted with the given paint. It has no active indicator and keeps
    /// its placeholder's own styling.
    #[must_use]
    pub fn new(
        container: Chrome,
        metrics: InputFieldMetrics,
        label: FloatingLabel,
        caret: Caret,
        selection: impl Into<Driven<Paint>>,
    ) -> Self {
        Self {
            container,
            metrics,
            active_indicator: None,
            placeholder: Label::new(),
            value: None,
            label,
            caret,
            selection: selection.into(),
        }
    }

    /// Draws a line along the container's bottom edge with the border
    /// `indicator` gives for each state.
    #[must_use]
    pub fn active_indicator(mut self, indicator: impl Into<Driven<Border>>) -> Self {
        self.active_indicator = Some(indicator.into());
        self
    }

    /// Styles the placeholder with `placeholder`.
    #[must_use]
    pub fn placeholder(mut self, placeholder: Label) -> Self {
        self.placeholder = placeholder;
        self
    }

    /// Styles the value the user typed, a secure field's masked text
    /// included, with `value`: its font and its colour for each interaction
    /// state. A look without it draws the value as plain text in the default
    /// foreground.
    #[must_use]
    pub fn value(mut self, value: Label) -> Self {
        self.value = Some(value);
        self
    }

    /// The container's chrome.
    #[must_use]
    pub const fn container(&self) -> &Chrome {
        &self.container
    }

    /// The metrics the field is laid out by.
    #[must_use]
    pub const fn metrics(&self) -> InputFieldMetrics {
        self.metrics
    }

    /// The active indicator, when the look has one.
    #[must_use]
    pub const fn get_active_indicator(&self) -> Option<&Driven<Border>> {
        self.active_indicator.as_ref()
    }

    /// How the placeholder is styled.
    #[must_use]
    pub const fn get_placeholder(&self) -> &Label {
        &self.placeholder
    }

    /// How the value is styled, when the look styles it.
    #[must_use]
    pub const fn get_value(&self) -> Option<&Label> {
        self.value.as_ref()
    }

    /// The inside label.
    #[must_use]
    pub const fn floating_label(&self) -> &FloatingLabel {
        &self.label
    }

    /// The caret.
    #[must_use]
    pub const fn caret(&self) -> &Caret {
        &self.caret
    }

    /// The selection's paint.
    #[must_use]
    pub const fn selection(&self) -> &Driven<Paint> {
        &self.selection
    }
}
