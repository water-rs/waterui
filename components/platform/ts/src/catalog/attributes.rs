//! One struct per component: the attributes it configures, and the conversion
//! that reads them.
//!
//! These are the declarations the catalog is assembled from. Each struct is
//! both halves of one contract — `#[derive(TsType)]` produces the schema the
//! catalog carries and the `.d.ts` is generated from, and the same derive
//! produces the [`FromJs`](crate::FromJs) conversion the host reads a
//! configuration object with — so an attribute cannot be declared in one place
//! and read in another.
//!
//! A two-way attribute is a [`Binding<T>`]: it is typed `Signal<T>` on the
//! TypeScript side and materialized from the JavaScript signal that was
//! written, so `<Toggle value={done}>` binds the signal itself rather than
//! round-tripping a value through a change handler. A read-only reactive
//! attribute is a [`Computed<T>`], which accepts a signal, a thunk or a
//! constant alike. Everything materializes inside the host call, so an
//! attribute no native view uses creates nothing on the Rust side.
//!
//! [`Binding<T>`]: nami::Binding
//! [`Computed<T>`]: nami::Computed

use nami::{Binding, Computed};
use suiteki::Str;
use waterui_core::AnyView;
use waterui_ts_schema::TsType;

use super::values::{
    BackgroundValue, HorizontalAlign, JsAction, PaddingValue, PickerOption, ScrollAxis, StackAlign,
    TextContent, VerticalAlign,
};

/// `<VStack>`: children stacked top to bottom.
#[derive(Debug, TsType)]
pub struct VStackAttributes {
    /// The gap between children, in points.
    pub spacing: Option<Computed<f32>>,
    /// Where children sit on the horizontal axis.
    pub alignment: Option<HorizontalAlign>,
}

/// `<HStack>`: children stacked leading to trailing.
#[derive(Debug, TsType)]
pub struct HStackAttributes {
    /// The gap between children, in points.
    pub spacing: Option<Computed<f32>>,
    /// Where children sit on the vertical axis.
    pub alignment: Option<VerticalAlign>,
}

/// `<ZStack>`: children stacked front to back.
#[derive(Debug, TsType)]
pub struct ZStackAttributes {
    /// Where children sit in both axes.
    pub alignment: Option<StackAlign>,
}

/// `<ScrollView>`: a scrollable surface around its children.
#[derive(Debug, TsType)]
pub struct ScrollViewAttributes {
    /// Which way the surface scrolls.
    pub axis: Option<ScrollAxis>,
}

/// `<Spacer>`: a flexible gap.
#[derive(Debug, TsType)]
pub struct SpacerAttributes {
    /// The gap's minimum length, in points.
    #[ts(rename = "minLength")]
    pub min_length: Option<f32>,
}

/// `<Divider>`: a hairline across the stack's cross axis.
#[derive(Debug, TsType)]
pub struct DividerAttributes {}

/// `<Text>`: semantic text.
#[derive(Debug, TsType)]
pub struct TextAttributes {}

/// `<Label>`: text with an optional icon, and the explicit form of every
/// control's label slot.
#[derive(Debug, TsType)]
pub struct LabelAttributes {}

/// `<Button>`: a control that performs an action.
#[derive(Debug, TsType)]
pub struct ButtonAttributes {
    /// What the button does when it is activated — by a tap, by the keyboard,
    /// or by an assistive technology, which is why this is the button's own
    /// attribute and not the `onTapGesture` modifier.
    #[ts(rename = "onTap")]
    pub on_tap: Option<JsAction>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

/// `<Toggle>`: a two-state switch.
#[derive(Debug, TsType)]
pub struct ToggleAttributes {
    /// The state the switch reads and writes.
    pub value: Binding<bool>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

/// `<Slider>`: a control for a value in a range.
#[derive(Debug, TsType)]
pub struct SliderAttributes {
    /// The value the slider reads and writes.
    pub value: Binding<f64>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

/// `<Stepper>`: a control that increments and decrements a whole number.
#[derive(Debug, TsType)]
pub struct StepperAttributes {
    /// The value the stepper reads and writes.
    pub value: Binding<i32>,
    /// How much one step changes the value.
    pub step: Option<Computed<i32>>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

/// `<TextField>`: a single-line text entry field.
#[derive(Debug, TsType)]
pub struct TextFieldAttributes {
    /// The text the field reads and writes.
    pub value: Binding<Str>,
    /// The placeholder shown while the field is empty.
    pub prompt: Option<Computed<TextContent>>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

/// `<Picker>`: a control that selects one of a set of options.
#[derive(Debug, TsType)]
pub struct PickerAttributes {
    /// The selected option's value.
    pub value: Binding<Str>,
    /// The options to choose from.
    pub options: Computed<Vec<PickerOption>>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

/// Every view modifier, and the value each one takes.
///
/// A modifier applies to every component, because `ViewExt` is implemented for
/// every `View`, so one declaration carries the whole table: the names
/// `installHost` hands JavaScript for attribute classification, the value
/// shapes the catalog publishes, and — through the same derive — the
/// conversions the host reads a modifier value with.
///
/// The struct is never constructed. Its fields are read one at a time, in the
/// order the attributes were written, because that order is the modifier chain
/// and the host applies it exactly as Rust would.
#[derive(Debug, TsType)]
pub struct Modifiers {
    /// Insets the view: `true` for the default inset, a number for every edge,
    /// or an object per edge.
    pub padding: Computed<PaddingValue>,
    /// Paints a colour, or a view, behind the view.
    pub background: BackgroundValue,
    /// Fixes the view's width, in points.
    pub width: Computed<f32>,
    /// Fixes the view's height, in points.
    pub height: Computed<f32>,
    /// The width the view will not go below.
    #[ts(rename = "minWidth")]
    pub min_width: Computed<f32>,
    /// The width the view will not exceed.
    #[ts(rename = "maxWidth")]
    pub max_width: Computed<f32>,
    /// The height the view will not go below.
    #[ts(rename = "minHeight")]
    pub min_height: Computed<f32>,
    /// The height the view will not exceed.
    #[ts(rename = "maxHeight")]
    pub max_height: Computed<f32>,
    /// Disables the view and everything below it.
    pub disabled: Computed<bool>,
    /// Runs a handler when the view is tapped. A control's own activation is
    /// its own attribute — `<Button onTap>` — because a native control is
    /// activated by more than a tap.
    #[ts(rename = "onTapGesture")]
    pub on_tap_gesture: JsAction,
    /// The view's accessibility label, for a view whose own content does not
    /// name it.
    #[ts(rename = "a11yLabel")]
    pub a11y_label: Computed<TextContent>,
    /// The view's accessibility value.
    #[ts(rename = "a11yValue")]
    pub a11y_value: Computed<TextContent>,
    /// A stable identifier for tests and tooling, never shown to anyone.
    #[ts(rename = "a11yId")]
    pub a11y_id: Str,
    /// Hides the view from assistive technology without hiding it visually.
    #[ts(rename = "a11yHidden")]
    pub a11y_hidden: bool,
}
