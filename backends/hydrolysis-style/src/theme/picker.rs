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
use waterui_core::Environment;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::color::Color;
use waterui_graphics::color::token::{
    Border, Foreground, MutedForeground, SelectionContainer, Surface,
};

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
    pub min_width: f32,
    /// Minimum field height.
    pub min_height: f32,
    /// Horizontal inset of the field's content.
    pub horizontal_inset: f32,
    /// Vertical inset of the field's content.
    pub vertical_inset: f32,
    /// Gap between the field's label and its value.
    pub label_spacing: f32,
    /// Space reserved for the dropdown indicator.
    pub indicator_space: f32,
    /// Vertical gap between the field and the popup it opens.
    pub popup_gap: f32,
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
    pub min_width: f32,
    /// Minimum group height.
    pub min_height: f32,
    /// Horizontal inset of the rows.
    pub horizontal_inset: f32,
    /// Vertical inset of the rows.
    pub vertical_inset: f32,
    /// Gap between the group's heading and its first row.
    pub label_spacing: f32,
    /// Gap between rows.
    pub row_spacing: f32,
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

/// The measures a wheel picker drum is laid out by, in points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelPickerLayout {
    /// Minimum wheel drum width.
    pub min_width: f32,
    /// Height of a single item row.
    pub row_height: f32,
    /// Number of visible rows in the wheel drum (odd so one row centers).
    pub visible_rows: usize,
    /// Horizontal inset of row labels inside the drum.
    pub horizontal_inset: f32,
    /// Vertical gap between a visible heading label and the wheel drum.
    pub label_spacing: f32,
    /// Corner radius of the wheel drum container.
    pub corner_radius: f32,
    /// Corner radius of the centered selection band.
    pub highlight_corner_radius: f32,
    /// Horizontal inset of the centered selection band inside the drum.
    pub highlight_inset: f32,
}

/// How a wheel picker looks: its layout metrics, container/highlight colours,
/// focus ring colour, and text styles for selected and off-center rows.
#[derive(Debug, Clone)]
pub struct WheelPickerLook {
    layout: WheelPickerLayout,
    surface: Color,
    border: Color,
    highlight: Color,
    focus_ring: Color,
    selected_label: Label,
    unselected_label: Label,
}

impl WheelPickerLook {
    /// A wheel picker laid out by `layout` with semantic theme colours.
    #[must_use]
    pub fn new(
        layout: WheelPickerLayout,
        surface: Color,
        border: Color,
        highlight: Color,
        focus_ring: Color,
    ) -> Self {
        Self {
            layout,
            surface,
            border,
            highlight,
            focus_ring,
            selected_label: Label::new().color(Color::new(Foreground)),
            unselected_label: Label::new().color(Color::new(MutedForeground)),
        }
    }

    /// Styles the centered selected row's label.
    #[must_use]
    pub fn selected_label(mut self, label: Label) -> Self {
        self.selected_label = label;
        self
    }

    /// Styles off-center unselected rows' labels.
    #[must_use]
    pub fn unselected_label(mut self, label: Label) -> Self {
        self.unselected_label = label;
        self
    }

    /// The wheel drum's layout measures.
    #[must_use]
    pub const fn layout(&self) -> WheelPickerLayout {
        self.layout
    }

    /// The drum container fill colour.
    #[must_use]
    pub const fn surface(&self) -> &Color {
        &self.surface
    }

    /// The drum container border colour.
    #[must_use]
    pub const fn border(&self) -> &Color {
        &self.border
    }

    /// The centered selection highlight band fill colour.
    #[must_use]
    pub const fn highlight(&self) -> &Color {
        &self.highlight
    }

    /// The keyboard focus ring colour.
    #[must_use]
    pub const fn focus_ring(&self) -> &Color {
        &self.focus_ring
    }

    /// The styling of the centered selected row's label.
    #[must_use]
    pub const fn get_selected_label(&self) -> &Label {
        &self.selected_label
    }

    /// The styling of off-center rows' labels.
    #[must_use]
    pub const fn get_unselected_label(&self) -> &Label {
        &self.unselected_label
    }
}

/// Default wheel picker look derived purely from the theme's colour tokens and
/// the picker's [`ControlSize`].
#[must_use]
pub fn default_wheel_picker_look(spec: &PickerSpec, _env: &Environment) -> WheelPickerLook {
    let (row_height, min_width) = match spec.size() {
        ControlSize::ExtraSmall => (24.0, 120.0),
        ControlSize::Small => (28.0, 144.0),
        ControlSize::Large => (36.0, 192.0),
        ControlSize::ExtraLarge => (40.0, 216.0),
        // `ControlSize::Medium` and any future size variants use the medium drum metrics.
        _ => (32.0, 168.0),
    };
    WheelPickerLook::new(
        WheelPickerLayout {
            min_width,
            row_height,
            visible_rows: 5,
            horizontal_inset: 16.0,
            label_spacing: 6.0,
            corner_radius: 10.0,
            highlight_corner_radius: 6.0,
            highlight_inset: 6.0,
        },
        Color::new(Surface),
        Color::new(Border),
        Color::new(SelectionContainer),
        spec.tint(),
    )
}
