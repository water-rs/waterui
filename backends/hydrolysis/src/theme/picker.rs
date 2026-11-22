//! The picker looks of the theme contract (water-rs/waterui#1434): what a
//! theme returns for a menu picker ([`MenuPickerLook`]) and a radio group
//! ([`RadioGroupLook`]). A segmented picker's look is
//! [`SegmentedLook`](super::SegmentedLook).
//!
//! A menu picker is a field whose [`Chrome`] draws its container, the
//! dropdown indicator and its interaction feedback; the popup it opens is
//! the [`MenuLook`](super::MenuLook). A radio group draws one
//! [`ToggleControl`] indicator per item, keyed by the item: each item keeps
//! its own live indicator over whether it is the selected one, so a
//! selection change retargets the operands of the two rows it touches and
//! records nothing.
//!
//! Every look draws left to right; Hydrolysis mirrors it in a right-to-left
//! layout.

use waterui_controls::ControlSize;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;

use super::look::{Label, SpecTint};
use super::{Chrome, ToggleControl};

/// What Hydrolysis asks a theme's [`menu_picker`](super::WidgetTheme::menu_picker)
/// or [`radio_group`](super::WidgetTheme::radio_group) look for: everything
/// structural about one picker.
///
/// A spec change requests the look again; everything else reaches the look
/// through its [`Driven`](super::Driven) values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickerSpec {
    size: ControlSize,
    direction: LayoutDirection,
    tint: SpecTint,
}

impl PickerSpec {
    /// The spec of a picker of `size`, laid out `direction`, drawn with
    /// `tint`.
    #[must_use]
    pub const fn new(size: ControlSize, direction: LayoutDirection, tint: SpecTint) -> Self {
        Self {
            size,
            direction,
            tint,
        }
    }

    /// The picker's control size.
    #[must_use]
    pub const fn size(&self) -> ControlSize {
        self.size
    }

    /// The layout direction the picker is laid out in.
    #[must_use]
    pub const fn direction(&self) -> LayoutDirection {
        self.direction
    }

    /// The picker's tint: its subtree's nearest `.tint(...)`, else the
    /// theme's `Accent` token. The colour is live.
    #[must_use]
    pub fn tint(&self) -> Color {
        self.tint.color()
    }
}

/// The measures a menu picker's field is laid out by, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuPickerLayout {
    /// Minimum field width.
    pub min_width: f64,
    /// Minimum field height.
    pub min_height: f64,
    /// Horizontal inset of the field's content.
    pub horizontal_inset: f64,
    /// Vertical inset of the field's content.
    pub vertical_inset: f64,
    /// Gap between the field's label and its value.
    pub label_spacing: f64,
    /// Space reserved for the dropdown indicator.
    pub indicator_space: f64,
    /// Vertical gap between the field and the popup it opens.
    pub popup_gap: f64,
}

/// How a menu picker looks: the [`Chrome`] of its field, the measures it is
/// laid out by and the styling of the selected value's text.
///
/// The chrome draws the field's container, its dropdown indicator (a custom
/// part of the chrome) and its interaction feedback.
#[derive(Debug, Clone)]
pub struct MenuPickerLook {
    chrome: Chrome,
    layout: MenuPickerLayout,
    value: Label,
}

impl MenuPickerLook {
    /// A menu picker drawn as `chrome`, laid out by `layout`, with its
    /// value in its own font and colour.
    #[must_use]
    pub const fn new(chrome: Chrome, layout: MenuPickerLayout) -> Self {
        Self {
            chrome,
            layout,
            value: Label::new(),
        }
    }

    /// Styles the selected value's text.
    #[must_use]
    pub fn value(mut self, value: Label) -> Self {
        self.value = value;
        self
    }

    /// The field's chrome.
    #[must_use]
    pub const fn chrome(&self) -> &Chrome {
        &self.chrome
    }

    /// The field's measures.
    #[must_use]
    pub const fn layout(&self) -> MenuPickerLayout {
        self.layout
    }

    /// The styling of the selected value's text.
    #[must_use]
    pub const fn get_value(&self) -> &Label {
        &self.value
    }
}

/// The measures a radio group is laid out by, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RadioLayout {
    /// Minimum group width.
    pub min_width: f64,
    /// Minimum group height.
    pub min_height: f64,
    /// Horizontal inset of the rows.
    pub horizontal_inset: f64,
    /// Vertical inset of the rows.
    pub vertical_inset: f64,
    /// Gap between the group's heading and its first row.
    pub label_spacing: f64,
    /// Gap between rows.
    pub row_spacing: f64,
}

/// How a radio group looks: the [`ToggleControl`] each row draws as its
/// indicator, over whether the row's item is selected, and the measures the
/// group is laid out by.
///
/// The control's size is the indicator's, and its label spacing the gap
/// between the indicator and its row's label.
#[derive(Debug, Clone)]
pub struct RadioGroupLook {
    indicator: ToggleControl,
    layout: RadioLayout,
    label: Label,
}

impl RadioGroupLook {
    /// A radio group whose rows draw `indicator`, laid out by `layout`.
    #[must_use]
    pub const fn new(indicator: ToggleControl, layout: RadioLayout) -> Self {
        Self {
            indicator,
            layout,
            label: Label::new(),
        }
    }

    /// Styles the rows' labels.
    #[must_use]
    pub fn label(mut self, label: Label) -> Self {
        self.label = label;
        self
    }

    /// The indicator each row draws.
    #[must_use]
    pub const fn indicator(&self) -> &ToggleControl {
        &self.indicator
    }

    /// The group's measures.
    #[must_use]
    pub const fn layout(&self) -> RadioLayout {
        self.layout
    }

    /// The styling of the rows' labels.
    #[must_use]
    pub const fn get_label(&self) -> &Label {
        &self.label
    }
}
