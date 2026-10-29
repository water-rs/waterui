//! One struct per component: the attributes it configures, and the conversion
//! that reads them.
//!
//! These are the declarations the catalog is assembled from. Each struct is
//! both halves of one contract — `#[derive(TsType)]` produces the schema the
//! catalog carries and the `.d.ts` is generated from, and the same derive
//! produces the [`FromJs`](waterui_ts::FromJs) conversion the host reads a
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
use waterui_ts::schema::TsType;

use waterui_ts::JsViewBuilder;

#[cfg(feature = "media")]
use super::values::UrlValue;
use super::values::{
    BackgroundValue, BorderValue, ColorValue, HorizontalAlign, JsAction, PaddingValue,
    PickerOption, ProgressStyleValue, ScrollAxis, ShadowValue, ShapeValue, StackAlign, TextContent,
    VerticalAlign,
};

/// `<VStack>`: children stacked top to bottom.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct VStackAttributes {
    /// The gap between children, in points.
    pub spacing: Option<Computed<f32>>,
    /// Where children sit on the horizontal axis.
    pub alignment: Option<HorizontalAlign>,
}

impl VStackAttributes {
    /// The gap between children, in points.
    #[must_use]
    pub const fn spacing(&self) -> Option<&Computed<f32>> {
        self.spacing.as_ref()
    }

    /// Where children sit on the horizontal axis.
    #[must_use]
    pub const fn alignment(&self) -> Option<HorizontalAlign> {
        self.alignment
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Option<Computed<f32>>, Option<HorizontalAlign>) {
        (self.spacing, self.alignment)
    }
}

/// `<HStack>`: children stacked leading to trailing.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct HStackAttributes {
    /// The gap between children, in points.
    pub spacing: Option<Computed<f32>>,
    /// Where children sit on the vertical axis.
    pub alignment: Option<VerticalAlign>,
}

impl HStackAttributes {
    /// The gap between children, in points.
    #[must_use]
    pub const fn spacing(&self) -> Option<&Computed<f32>> {
        self.spacing.as_ref()
    }

    /// Where children sit on the vertical axis.
    #[must_use]
    pub const fn alignment(&self) -> Option<VerticalAlign> {
        self.alignment
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Option<Computed<f32>>, Option<VerticalAlign>) {
        (self.spacing, self.alignment)
    }
}

/// `<ZStack>`: children stacked front to back.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ZStackAttributes {
    /// Where children sit in both axes.
    pub alignment: Option<StackAlign>,
}

impl ZStackAttributes {
    /// Where children sit in both axes.
    #[must_use]
    pub const fn alignment(&self) -> Option<StackAlign> {
        self.alignment
    }
}

/// `<ScrollView>`: a scrollable surface around its children.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ScrollViewAttributes {
    /// Which way the surface scrolls.
    pub axis: Option<ScrollAxis>,
}

impl ScrollViewAttributes {
    /// Which way the surface scrolls.
    #[must_use]
    pub const fn axis(&self) -> Option<ScrollAxis> {
        self.axis
    }
}

/// `<Spacer>`: a flexible gap.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct SpacerAttributes {
    /// The gap's minimum length, in points.
    #[ts(rename = "minLength")]
    pub min_length: Option<f32>,
}

impl SpacerAttributes {
    /// The gap's minimum length, in points.
    #[must_use]
    pub const fn min_length(&self) -> Option<f32> {
        self.min_length
    }
}

/// `<Divider>`: a hairline across the stack's cross axis.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct DividerAttributes {}

/// `<Text>`: semantic text.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct TextAttributes {}

/// `<Label>`: text with an optional icon, and the explicit form of every
/// control's label slot.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct LabelAttributes {}

/// `<Button>`: a control that performs an action.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ButtonAttributes {
    /// What the button does when it is activated — by a tap, by the keyboard,
    /// or by an assistive technology, which is why this is the button's own
    /// attribute and not the `onTapGesture` modifier.
    #[ts(rename = "onTap")]
    pub on_tap: Option<JsAction>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

impl ButtonAttributes {
    /// What the button does when it is activated — by a tap, by the keyboard,
    /// or by an assistive technology, which is why this is the button's own
    /// attribute and not the `onTapGesture` modifier.
    #[must_use]
    pub const fn on_tap(&self) -> Option<&JsAction> {
        self.on_tap.as_ref()
    }

    /// The label, when it is not the children.
    #[must_use]
    pub const fn label(&self) -> Option<&AnyView> {
        self.label.as_ref()
    }

    /// Takes the label out, leaving none behind.
    pub const fn take_label(&mut self) -> Option<AnyView> {
        self.label.take()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Option<JsAction>, Option<AnyView>) {
        (self.on_tap, self.label)
    }
}

/// `<Toggle>`: a two-state switch.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ToggleAttributes {
    /// The state the switch reads and writes.
    pub value: Binding<bool>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

impl ToggleAttributes {
    /// The state the switch reads and writes.
    #[must_use]
    pub const fn value(&self) -> &Binding<bool> {
        &self.value
    }

    /// The label, when it is not the children.
    #[must_use]
    pub const fn label(&self) -> Option<&AnyView> {
        self.label.as_ref()
    }

    /// Takes the label out, leaving none behind.
    pub const fn take_label(&mut self) -> Option<AnyView> {
        self.label.take()
    }
}

/// `<Slider>`: a control for a value in a range.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct SliderAttributes {
    /// The value the slider reads and writes.
    pub value: Binding<f64>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

impl SliderAttributes {
    /// The value the slider reads and writes.
    #[must_use]
    pub const fn value(&self) -> &Binding<f64> {
        &self.value
    }

    /// The label, when it is not the children.
    #[must_use]
    pub const fn label(&self) -> Option<&AnyView> {
        self.label.as_ref()
    }

    /// Takes the label out, leaving none behind.
    pub const fn take_label(&mut self) -> Option<AnyView> {
        self.label.take()
    }
}

/// `<Stepper>`: a control that increments and decrements a whole number.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct StepperAttributes {
    /// The value the stepper reads and writes.
    pub value: Binding<i32>,
    /// How much one step changes the value.
    pub step: Option<Computed<i32>>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

impl StepperAttributes {
    /// The value the stepper reads and writes.
    #[must_use]
    pub const fn value(&self) -> &Binding<i32> {
        &self.value
    }

    /// How much one step changes the value.
    #[must_use]
    pub const fn step(&self) -> Option<&Computed<i32>> {
        self.step.as_ref()
    }

    /// The label, when it is not the children.
    #[must_use]
    pub const fn label(&self) -> Option<&AnyView> {
        self.label.as_ref()
    }

    /// Takes the label out, leaving none behind.
    pub const fn take_label(&mut self) -> Option<AnyView> {
        self.label.take()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Binding<i32>, Option<Computed<i32>>, Option<AnyView>) {
        (self.value, self.step, self.label)
    }
}

/// `<TextField>`: a single-line text entry field.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct TextFieldAttributes {
    /// The text the field reads and writes.
    pub value: Binding<Str>,
    /// The placeholder shown while the field is empty.
    pub prompt: Option<Computed<TextContent>>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

impl TextFieldAttributes {
    /// The text the field reads and writes.
    #[must_use]
    pub const fn value(&self) -> &Binding<Str> {
        &self.value
    }

    /// The placeholder shown while the field is empty.
    #[must_use]
    pub const fn prompt(&self) -> Option<&Computed<TextContent>> {
        self.prompt.as_ref()
    }

    /// The label, when it is not the children.
    #[must_use]
    pub const fn label(&self) -> Option<&AnyView> {
        self.label.as_ref()
    }

    /// Takes the label out, leaving none behind.
    pub const fn take_label(&mut self) -> Option<AnyView> {
        self.label.take()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Binding<Str>, Option<Computed<TextContent>>, Option<AnyView>) {
        (self.value, self.prompt, self.label)
    }
}

/// `<Picker>`: a control that selects one of a set of options.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct PickerAttributes {
    /// The selected option's value.
    pub value: Binding<Str>,
    /// The options to choose from.
    pub options: Computed<Vec<PickerOption>>,
    /// The label, when it is not the children.
    pub label: Option<AnyView>,
}

impl PickerAttributes {
    /// The selected option's value.
    #[must_use]
    pub const fn value(&self) -> &Binding<Str> {
        &self.value
    }

    /// The options to choose from.
    #[must_use]
    pub const fn options(&self) -> &Computed<Vec<PickerOption>> {
        &self.options
    }

    /// The label, when it is not the children.
    #[must_use]
    pub const fn label(&self) -> Option<&AnyView> {
        self.label.as_ref()
    }

    /// Takes the label out, leaving none behind.
    pub const fn take_label(&mut self) -> Option<AnyView> {
        self.label.take()
    }
}

/// `<List>`: a native list of rows.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ListAttributes {
    /// Whether the list is in edit mode, which shows the controls for the
    /// handlers below: a row has a delete control when the list can delete,
    /// and a reorder handle when it can move.
    pub editing: Option<Computed<bool>>,
    /// Runs when a row's delete control is used, with the index of the row
    /// that was deleted. A list with no handler shows no delete control,
    /// because the control would do nothing.
    #[ts(rename = "onDelete")]
    pub on_delete: Option<JsAction<(u32,)>>,
    /// Runs when a row is moved, with the index it left and the index it
    /// arrived at. A list with no handler shows no reorder handle.
    #[ts(rename = "onMove")]
    pub on_move: Option<JsAction<(u32, u32)>>,
}

/// The fields of [`ListAttributes`] in declaration order: `editing`,
/// `on_delete` and `on_move`.
pub type ListAttributesParts = (
    Option<Computed<bool>>,
    Option<JsAction<(u32,)>>,
    Option<JsAction<(u32, u32)>>,
);

impl ListAttributes {
    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> ListAttributesParts {
        (self.editing, self.on_delete, self.on_move)
    }

    /// Creates a `ListAttributes` from its parts.
    #[must_use]
    pub const fn new(
        editing: Option<Computed<bool>>,
        on_delete: Option<JsAction<(u32,)>>,
        on_move: Option<JsAction<(u32, u32)>>,
    ) -> Self {
        Self {
            editing,
            on_delete,
            on_move,
        }
    }

    /// Whether the list is in edit mode, which shows the controls for the
    /// handlers below: a row has a delete control when the list can delete,
    /// and a reorder handle when it can move.
    #[must_use]
    pub const fn editing(&self) -> Option<&Computed<bool>> {
        self.editing.as_ref()
    }

    /// Runs when a row's delete control is used, with the index of the row
    /// that was deleted. A list with no handler shows no delete control,
    /// because the control would do nothing.
    #[must_use]
    pub const fn on_delete(&self) -> Option<&JsAction<(u32,)>> {
        self.on_delete.as_ref()
    }

    /// Runs when a row is moved, with the index it left and the index it
    /// arrived at. A list with no handler shows no reorder handle.
    #[must_use]
    pub const fn on_move(&self) -> Option<&JsAction<(u32, u32)>> {
        self.on_move.as_ref()
    }
}

/// `<Progress>`: determinate or indeterminate progress.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ProgressAttributes {
    /// How far along, `0` through `total`. Left out, the progress is
    /// indeterminate — what Rust spells `Progress::infinity()`.
    pub value: Option<Computed<f64>>,
    /// The value that counts as complete; `1` when left out.
    pub total: Option<Computed<f64>>,
    /// Which shape the progress takes.
    pub style: Option<ProgressStyleValue>,
}

impl ProgressAttributes {
    /// How far along, `0` through `total`. Left out, the progress is
    /// indeterminate — what Rust spells `Progress::infinity()`.
    #[must_use]
    pub const fn value(&self) -> Option<&Computed<f64>> {
        self.value.as_ref()
    }

    /// The value that counts as complete; `1` when left out.
    #[must_use]
    pub const fn total(&self) -> Option<&Computed<f64>> {
        self.total.as_ref()
    }

    /// Which shape the progress takes.
    #[must_use]
    pub const fn style(&self) -> Option<ProgressStyleValue> {
        self.style
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        Option<Computed<f64>>,
        Option<Computed<f64>>,
        Option<ProgressStyleValue>,
    ) {
        (self.value, self.total, self.style)
    }
}

/// `<Link>`: a label that opens a URL.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct LinkAttributes {
    /// The URL the link opens.
    pub url: Computed<Str>,
}

impl LinkAttributes {
    /// The URL the link opens.
    #[must_use]
    pub const fn url(&self) -> &Computed<Str> {
        &self.url
    }

    /// Takes the `url` out of the value.
    #[must_use]
    pub fn into_url(self) -> Computed<Str> {
        self.url
    }
}

/// `<Card>`: a titled surface around its content.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct CardAttributes {
    /// The card's title.
    pub title: Option<TextContent>,
    /// The card's subtitle, under the title.
    pub subtitle: Option<TextContent>,
}

impl CardAttributes {
    /// The card's title.
    #[must_use]
    pub const fn title(&self) -> Option<&TextContent> {
        self.title.as_ref()
    }

    /// The card's subtitle, under the title.
    #[must_use]
    pub const fn subtitle(&self) -> Option<&TextContent> {
        self.subtitle.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Option<TextContent>, Option<TextContent>) {
        (self.title, self.subtitle)
    }
}

/// `<Badge>`: a count over another view.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct BadgeAttributes {
    /// The count the badge shows.
    pub value: Computed<i32>,
    /// What the badge is attached to. A builder rather than a view, because
    /// `Badge` may build its content again.
    pub content: JsViewBuilder,
}

impl BadgeAttributes {
    /// The count the badge shows.
    #[must_use]
    pub const fn value(&self) -> &Computed<i32> {
        &self.value
    }

    /// What the badge is attached to. A builder rather than a view, because
    /// `Badge` may build its content again.
    #[must_use]
    pub const fn content(&self) -> &JsViewBuilder {
        &self.content
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Computed<i32>, JsViewBuilder) {
        (self.value, self.content)
    }
}

/// `<Table>`: columns of text.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct TableAttributes {}

/// `<Column>`: one column of a `<Table>`.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ColumnAttributes {
    /// The column's heading.
    pub label: TextContent,
}

impl ColumnAttributes {
    /// The column's heading.
    #[must_use]
    pub const fn label(&self) -> &TextContent {
        &self.label
    }

    /// Takes the `label` out of the value.
    #[must_use]
    pub fn into_label(self) -> TextContent {
        self.label
    }
}

/// `<Accordion>`: a header that expands to reveal its content.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct AccordionAttributes {
    /// The content revealed when the accordion is expanded. A builder, because
    /// the content is built when it is revealed and again after it is hidden.
    pub content: JsViewBuilder,
    /// Whether the accordion is expanded, when the application owns that state.
    pub expanded: Option<Binding<bool>>,
}

impl AccordionAttributes {
    /// The content revealed when the accordion is expanded. A builder, because
    /// the content is built when it is revealed and again after it is hidden.
    #[must_use]
    pub const fn content(&self) -> &JsViewBuilder {
        &self.content
    }

    /// Whether the accordion is expanded, when the application owns that state.
    #[must_use]
    pub const fn expanded(&self) -> Option<&Binding<bool>> {
        self.expanded.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (JsViewBuilder, Option<Binding<bool>>) {
        (self.content, self.expanded)
    }
}

/// `<Avatar>`: a person's picture, or their monogram.
///
/// The picture is fetched and decoded by `waterui-media`, so the attribute
/// that names it exists only with the facade's `media` feature — the same
/// gate `<Image>` sits behind, for the same reason: a build without the
/// media graph declares no attribute it cannot honour.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct AvatarAttributes {
    /// Who the avatar is of, which is also its accessibility label.
    pub name: TextContent,
    /// The picture's URL. Without one the avatar shows the monogram.
    #[cfg(feature = "media")]
    pub image: Option<Computed<UrlValue>>,
}

impl AvatarAttributes {
    /// Who the avatar is of, which is also its accessibility label.
    #[must_use]
    pub const fn name(&self) -> &TextContent {
        &self.name
    }

    /// The picture's URL. Without one the avatar shows the monogram.
    #[cfg(feature = "media")]
    #[must_use]
    pub const fn image(&self) -> Option<&Computed<UrlValue>> {
        self.image.as_ref()
    }

    /// Takes the `name` out of the value.
    #[must_use]
    pub fn into_name(self) -> TextContent {
        self.name
    }

    /// Takes the value apart into its fields, in declaration order.
    #[cfg(feature = "media")]
    #[must_use]
    pub fn into_parts(self) -> (TextContent, Option<Computed<UrlValue>>) {
        (self.name, self.image)
    }
}

/// `<NavigationStack>`: a stack whose children are its root.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct NavigationStackAttributes {
    /// The root's navigation title.
    pub title: Option<TextContent>,
}

impl NavigationStackAttributes {
    /// The root's navigation title.
    #[must_use]
    pub const fn title(&self) -> Option<&TextContent> {
        self.title.as_ref()
    }

    /// Takes the `title` out of the value.
    #[must_use]
    pub fn into_title(self) -> Option<TextContent> {
        self.title
    }
}

/// `<NavigationLink>`: a label that pushes a destination.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct NavigationLinkAttributes {
    /// The destination, built afresh every time the link is followed.
    pub destination: JsViewBuilder,
    /// The destination's navigation title.
    pub title: Option<TextContent>,
}

impl NavigationLinkAttributes {
    /// The destination, built afresh every time the link is followed.
    #[must_use]
    pub const fn destination(&self) -> &JsViewBuilder {
        &self.destination
    }

    /// The destination's navigation title.
    #[must_use]
    pub const fn title(&self) -> Option<&TextContent> {
        self.title.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (JsViewBuilder, Option<TextContent>) {
        (self.destination, self.title)
    }
}

/// `<Tabs>`: a native tab container.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct TabsAttributes {
    /// Which tab is showing.
    pub value: Binding<Str>,
}

impl TabsAttributes {
    /// Which tab is showing.
    #[must_use]
    pub const fn value(&self) -> &Binding<Str> {
        &self.value
    }
}

/// `<Tab>`: one page of a `<Tabs>`, whose children are its label.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct TabAttributes {
    /// The value written into the container's `value` when this tab is chosen.
    pub value: Str,
    /// The tab's root, built afresh every time the tab is shown.
    pub content: JsViewBuilder,
    /// The root's navigation title.
    pub title: Option<TextContent>,
}

impl TabAttributes {
    /// The value written into the container's `value` when this tab is chosen.
    #[must_use]
    pub const fn value(&self) -> &Str {
        &self.value
    }

    /// The tab's root, built afresh every time the tab is shown.
    #[must_use]
    pub const fn content(&self) -> &JsViewBuilder {
        &self.content
    }

    /// The root's navigation title.
    #[must_use]
    pub const fn title(&self) -> Option<&TextContent> {
        self.title.as_ref()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Str, JsViewBuilder, Option<TextContent>) {
        (self.value, self.content, self.title)
    }
}

/// `<Image>`: a picture fetched from a URL.
#[cfg(feature = "media")]
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct ImageAttributes {
    /// Where the picture comes from.
    pub src: Computed<UrlValue>,
    /// Whether the decoded picture may stretch to the bounds it is offered.
    pub resizable: Option<bool>,
}

#[cfg(feature = "media")]
impl ImageAttributes {
    /// Where the picture comes from.
    #[must_use]
    pub const fn src(&self) -> &Computed<UrlValue> {
        &self.src
    }

    /// Whether the decoded picture may stretch to the bounds it is offered.
    #[must_use]
    pub const fn resizable(&self) -> Option<bool> {
        self.resizable
    }
}

/// The modifiers, which apply to every component.
///
/// `ViewExt` is implemented for every `View`, so one declaration carries the
/// whole table: the names `installHost` hands JavaScript for attribute
/// classification, the value shapes the catalog publishes, and — through the
/// same derive — the conversions the host reads a modifier value with.
///
/// The struct is never constructed. Its fields are read one at a time, in the
/// order the attributes were written, because that order is the modifier chain
/// and the host applies it exactly as Rust would.
#[derive(Debug, TsType)]
#[non_exhaustive]
pub struct Modifiers {
    /// Insets the view: `true` for the default inset, a number for every edge,
    /// or an object per edge.
    pub padding: PaddingValue,
    /// Paints a colour, or a view, behind the view.
    pub background: BackgroundValue,
    /// Tints the view's text and symbols.
    pub foreground: Computed<ColorValue>,
    /// Makes the view translucent, `0` through `1`.
    pub opacity: Computed<f32>,
    /// Casts a shadow behind the view.
    pub shadow: ShadowValue,
    /// Strokes a border around the view.
    pub border: BorderValue,
    /// Clips the view to a shape.
    pub clip: ShapeValue,
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

impl Modifiers {
    /// Insets the view: `true` for the default inset, a number for every edge,
    /// or an object per edge.
    #[must_use]
    pub const fn padding(&self) -> &PaddingValue {
        &self.padding
    }

    /// Paints a colour, or a view, behind the view.
    #[must_use]
    pub const fn background(&self) -> &BackgroundValue {
        &self.background
    }

    /// Tints the view's text and symbols.
    #[must_use]
    pub const fn foreground(&self) -> &Computed<ColorValue> {
        &self.foreground
    }

    /// Makes the view translucent, `0` through `1`.
    #[must_use]
    pub const fn opacity(&self) -> &Computed<f32> {
        &self.opacity
    }

    /// Casts a shadow behind the view.
    #[must_use]
    pub const fn shadow(&self) -> &ShadowValue {
        &self.shadow
    }

    /// Strokes a border around the view.
    #[must_use]
    pub const fn border(&self) -> &BorderValue {
        &self.border
    }

    /// Clips the view to a shape.
    #[must_use]
    pub const fn clip(&self) -> ShapeValue {
        self.clip
    }

    /// Fixes the view's width, in points.
    #[must_use]
    pub const fn width(&self) -> &Computed<f32> {
        &self.width
    }

    /// Fixes the view's height, in points.
    #[must_use]
    pub const fn height(&self) -> &Computed<f32> {
        &self.height
    }

    /// The width the view will not go below.
    #[must_use]
    pub const fn min_width(&self) -> &Computed<f32> {
        &self.min_width
    }

    /// The width the view will not exceed.
    #[must_use]
    pub const fn max_width(&self) -> &Computed<f32> {
        &self.max_width
    }

    /// The height the view will not go below.
    #[must_use]
    pub const fn min_height(&self) -> &Computed<f32> {
        &self.min_height
    }

    /// The height the view will not exceed.
    #[must_use]
    pub const fn max_height(&self) -> &Computed<f32> {
        &self.max_height
    }

    /// Disables the view and everything below it.
    #[must_use]
    pub const fn disabled(&self) -> &Computed<bool> {
        &self.disabled
    }

    /// Runs a handler when the view is tapped. A control's own activation is
    /// its own attribute — `<Button onTap>` — because a native control is
    /// activated by more than a tap.
    #[must_use]
    pub const fn on_tap_gesture(&self) -> &JsAction {
        &self.on_tap_gesture
    }

    /// The view's accessibility label, for a view whose own content does not
    /// name it.
    #[must_use]
    pub const fn a11y_label(&self) -> &Computed<TextContent> {
        &self.a11y_label
    }

    /// The view's accessibility value.
    #[must_use]
    pub const fn a11y_value(&self) -> &Computed<TextContent> {
        &self.a11y_value
    }

    /// A stable identifier for tests and tooling, never shown to anyone.
    #[must_use]
    pub const fn a11y_id(&self) -> &Str {
        &self.a11y_id
    }

    /// Hides the view from assistive technology without hiding it visually.
    #[must_use]
    pub const fn a11y_hidden(&self) -> bool {
        self.a11y_hidden
    }
}
