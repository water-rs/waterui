//! The restricted authoring surface.
//!
//! Only types implementing [`WidgetContent`] can appear inside a widget, and
//! only [`WidgetChildren`] implementations can nest inside a stack. Neither
//! trait is implemented by `waterui::View` or any `WaterUI` component — a
//! normal view in widget position fails to compile, which is the mechanism
//! the design calls "unsupported components are a compile-time error".

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use core::ops::{Add, Sub};
use core::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use suiteki::Str;
use waterui_contract::contract_violation;

use crate::schema::{
    AccessibilityLabel, Alignment, Axis, Background, Color, Date, DateStyle, DeclaredWidgetIntent,
    EdgeInsets, Font, FontStyle, FontWeight, Frame, HorizontalAlignment, Image, Intent, LabelText,
    Link, MaxLength, Node, Padding, SemanticColor, Shape, Spacer, Stack, Text, Timeline,
    TimelineEntry, VerticalAlignment,
};

/// A value that can occupy a widget snapshot.
///
/// Implemented by every schema node type and by the string/date types that
/// convert to [`Text`]. `View`s, signals, environments and `AnyView`s do not
/// implement it — nothing reactive or platform-bound can cross into a
/// snapshot.
///
/// The closed surface is what makes an unsupported component a compile-time
/// error rather than a runtime drop: a `WaterUI` view in widget position
/// does not compile —
///
/// ```compile_fail
/// use waterui_widgets::prelude::*;
/// let _ = vstack((waterui_text::text("inside an app"),));
/// ```
///
/// — and neither does a value with no schema representation:
///
/// ```compile_fail
/// use waterui_widgets::prelude::*;
/// let _ = vstack((42,));
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot appear in a widget",
    label = "expected a widget node",
    note = "the widget schema admits text, image, stack, shape, color, padding, spacer, frame, background, accessibility label, link and intent nodes only. `WaterUI` views (`Text`, `Button`, `VStack`, …) are not widget content; use `waterui::widgets::prelude::*` constructors instead."
)]
pub trait WidgetContent {
    /// Convert into the wire node.
    fn into_node(self) -> Node;
}

/// Modifier methods shared by every widget node, mirroring `waterui::ViewExt`.
pub trait WidgetExt: WidgetContent + Sized {
    /// Adds platform-default padding around this node.
    #[must_use]
    fn padding(self) -> Padding {
        Padding::default_insets(self)
    }

    /// Adds explicit edge-inset padding around this node.
    #[must_use]
    fn padding_with(self, insets: impl Into<EdgeInsets>) -> Padding {
        Padding::new(insets, self)
    }

    /// Adds leading and trailing padding around this node.
    #[must_use]
    fn padding_horizontal(self, value: f32) -> Padding {
        Padding::new(EdgeInsets::symmetric(0.0, value), self)
    }

    /// Adds top and bottom padding around this node.
    #[must_use]
    fn padding_vertical(self, value: f32) -> Padding {
        Padding::new(EdgeInsets::symmetric(value, 0.0), self)
    }

    /// Fixes this node's width.
    #[must_use]
    fn width(self, width: f32) -> Frame {
        Frame::new(self).width(width)
    }

    /// Fixes this node's height.
    #[must_use]
    fn height(self, height: f32) -> Frame {
        Frame::new(self).height(height)
    }

    /// Fixes both width and height simultaneously.
    #[must_use]
    fn size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).size(width, height)
    }

    /// Applies a minimum width constraint.
    #[must_use]
    fn min_width(self, width: f32) -> Frame {
        Frame::new(self).min_width(width)
    }

    /// Applies a maximum width constraint (`f32::INFINITY` expands to fill).
    #[must_use]
    fn max_width(self, width: impl Into<MaxLength>) -> Frame {
        Frame::new(self).max_width(width)
    }

    /// Applies a minimum height constraint.
    #[must_use]
    fn min_height(self, height: f32) -> Frame {
        Frame::new(self).min_height(height)
    }

    /// Applies a maximum height constraint (`f32::INFINITY` expands to fill).
    #[must_use]
    fn max_height(self, height: impl Into<MaxLength>) -> Frame {
        Frame::new(self).max_height(height)
    }

    /// Applies minimum constraints on both axes.
    #[must_use]
    fn min_size(self, width: f32, height: f32) -> Frame {
        Frame::new(self).min_size(width, height)
    }

    /// Applies maximum constraints on both axes.
    #[must_use]
    fn max_size(self, width: impl Into<MaxLength>, height: impl Into<MaxLength>) -> Frame {
        Frame::new(self).max_size(width, height)
    }

    /// Aligns this node within a frame.
    #[must_use]
    fn alignment(self, alignment: impl Into<Alignment>) -> Frame {
        Frame::new(self).alignment(alignment)
    }

    /// Paints `background` behind this node.
    ///
    /// At the root of a widget entry, the Apple `WidgetKit` host lifts this
    /// background into `.containerBackground(for: .widget)`.
    #[must_use]
    fn background(self, background: impl WidgetContent) -> Background {
        Background::new(self, background)
    }

    /// Attaches a screen-reader accessibility label to this node.
    #[must_use]
    fn a11y_label(self, label: impl IntoLabelText) -> AccessibilityLabel {
        AccessibilityLabel::new(label, self)
    }
}

impl<T: WidgetContent + Sized> WidgetExt for T {}

/// A value usable as the root of a timeline-producing function.
///
/// Any [`WidgetContent`] produces a single-entry timeline dated "now" with
/// the default [`ReloadPolicy::AtEnd`](crate::ReloadPolicy::AtEnd); an explicit [`TimelineEntry`] or
/// [`Timeline`] sets the date and reload behavior itself.
pub trait IntoTimeline {
    /// Convert into the timeline the extension reports.
    fn into_timeline(self) -> Timeline;
}

impl IntoTimeline for Timeline {
    fn into_timeline(self) -> Timeline {
        self
    }
}

impl IntoTimeline for TimelineEntry {
    fn into_timeline(self) -> Timeline {
        Timeline::new([self])
    }
}

impl<T: WidgetContent> IntoTimeline for T {
    fn into_timeline(self) -> Timeline {
        Timeline::new([TimelineEntry::new(Date::now(), self)])
    }
}

/// Children a stack can hold: tuples of [`WidgetContent`], or homogeneous
/// collections for generated content.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be widget children",
    label = "expected a tuple of widget nodes",
    note = "stack children are tuples of `WidgetContent` values — `(text(\"a\"), spacer(), fill(color))` — or `Vec`/`Option`/`[_; N]` of `WidgetContent`. `WaterUI` views and tuples containing them do not qualify."
)]
pub trait WidgetChildren {
    /// Convert into the stack's ordered child nodes.
    fn into_nodes(self) -> Vec<Node>;
}

impl<T: WidgetContent> WidgetChildren for Vec<T> {
    fn into_nodes(self) -> Vec<Node> {
        self.into_iter().map(WidgetContent::into_node).collect()
    }
}

impl<T: WidgetContent, const N: usize> WidgetChildren for [T; N] {
    fn into_nodes(self) -> Vec<Node> {
        self.into_iter().map(WidgetContent::into_node).collect()
    }
}

/// `None` contributes no child; `Some` contributes one — conditional
/// children without leaving the snapshot value world.
impl<T: WidgetContent> WidgetChildren for Option<T> {
    fn into_nodes(self) -> Vec<Node> {
        self.map_or_else(Vec::new, |node| vec![node.into_node()])
    }
}

/// Tuple arity like the core `tuples!` helper, private to this crate since
/// the widget surface must not pull `waterui-core` in.
macro_rules! tuples {
    ($mac:ident) => {
        $mac!(T1);
        $mac!(T1, T2);
        $mac!(T1, T2, T3);
        $mac!(T1, T2, T3, T4);
        $mac!(T1, T2, T3, T4, T5);
        $mac!(T1, T2, T3, T4, T5, T6);
        $mac!(T1, T2, T3, T4, T5, T6, T7);
        $mac!(T1, T2, T3, T4, T5, T6, T7, T8);
        $mac!(T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $mac!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $mac!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $mac!(T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
    };
}

macro_rules! impl_widget_children {
    ($($ty:ident),*) => {
        #[expect(
            non_snake_case,
            reason = "tuple destructuring reuses type parameter identifiers"
        )]
        impl<$($ty: WidgetContent),*> WidgetChildren for ($($ty,)*) {
            fn into_nodes(self) -> Vec<Node> {
                let ($($ty,)*) = self;
                vec![$($ty.into_node()),*]
            }
        }
    };
}

tuples!(impl_widget_children);

impl WidgetContent for Node {
    fn into_node(self) -> Node {
        self
    }
}

macro_rules! impl_node {
    ($($ty:ident => $variant:ident),*) => {
        $(
            impl WidgetContent for $ty {
                fn into_node(self) -> Node {
                    Node::$variant(self)
                }
            }
        )*
    };
}

impl_node!(
    Text => Text,
    Image => Image,
    Stack => Stack,
    Shape => Shape,
    Color => Color,
    Padding => Padding,
    Spacer => Spacer,
    Frame => Frame,
    Background => Background,
    AccessibilityLabel => AccessibilityLabel,
    Link => Link,
    Intent => Intent
);

impl WidgetContent for &'static str {
    fn into_node(self) -> Node {
        Node::Text(Text::localized(self))
    }
}

impl WidgetContent for String {
    fn into_node(self) -> Node {
        Node::Text(Text::verbatim(self))
    }
}

impl WidgetContent for Str {
    fn into_node(self) -> Node {
        Node::Text(Text::verbatim(String::from(self)))
    }
}

impl WidgetContent for Date {
    fn into_node(self) -> Node {
        DateText::new(self).into_node()
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Text & label conversion traits
// ══════════════════════════════════════════════════════════════════════════

/// Conversion trait for widget text construction, mirroring
/// `waterui_text::IntoText` and additionally accepting [`Date`] for live
/// host-formatted dates (`text(Date::now()).relative()`).
///
/// - `&'static str` produces localized text (resolved by the host platform's
///   string catalog).
/// - `String` and [`Str`] produce verbatim text.
/// - [`Date`] produces a [`DateText`] leaf whose formatting style can be
///   refined with `.relative()`, `.offset()`, `.timer()`, `.date()`, or
///   `.time()`.
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as widget text",
    label = "expected widget text or a `Date`",
    note = "`IntoText` is implemented by `&'static str` (localized), `String` / `Str` (verbatim), `Date` (live formatted date), `Text`, and `DateText`."
)]
pub trait IntoText {
    /// The text node produced by [`text`].
    type Output: WidgetContent;

    /// Converts this value into a widget text leaf.
    fn into_text(self) -> Self::Output;
}

impl IntoText for Text {
    type Output = Self;

    fn into_text(self) -> Self {
        self
    }
}

impl IntoText for DateText {
    type Output = Self;

    fn into_text(self) -> Self {
        self
    }
}

impl IntoText for &'static str {
    type Output = Text;

    fn into_text(self) -> Text {
        Text::localized(self)
    }
}

impl IntoText for String {
    type Output = Text;

    fn into_text(self) -> Text {
        Text::verbatim(self)
    }
}

impl IntoText for Str {
    type Output = Text;

    fn into_text(self) -> Text {
        Text::verbatim(String::from(self))
    }
}

impl IntoText for Date {
    type Output = DateText;

    fn into_text(self) -> DateText {
        DateText::new(self)
    }
}

/// A live date [`Text`] leaf produced by `text(Date::now())`.
///
/// Defaults to [`DateStyle::Date`] if placed directly in a widget; call
/// [`.relative()`](Self::relative), [`.offset()`](Self::offset),
/// [`.timer()`](Self::timer), [`.date()`](Self::date), or
/// [`.time()`](Self::time) to choose the live formatting style.
///
/// Calling `.relative()` on string text is a compile-time error:
///
/// ```compile_fail
/// use waterui_widgets::prelude::*;
/// let _ = text("Steps").relative();
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct DateText {
    inner: Text,
}

impl DateText {
    /// Creates a date text leaf defaulting to [`DateStyle::Date`].
    #[must_use]
    pub const fn new(date: Date) -> Self {
        Self {
            inner: Text::date(date, DateStyle::Date),
        }
    }

    /// Formats the date relative to the current time (`Text(date, style: .relative)`).
    #[must_use]
    pub fn relative(self) -> Text {
        self.inner.with_date_style(DateStyle::Relative)
    }

    /// Formats the date as a signed offset from the current time.
    #[must_use]
    pub fn offset(self) -> Text {
        self.inner.with_date_style(DateStyle::Offset)
    }

    /// Formats the date as a live counting timer.
    #[must_use]
    pub fn timer(self) -> Text {
        self.inner.with_date_style(DateStyle::Timer)
    }

    /// Formats the date as a calendar date.
    #[must_use]
    pub fn date(self) -> Text {
        self.inner.with_date_style(DateStyle::Date)
    }

    /// Formats the date as a time of day.
    #[must_use]
    pub fn time(self) -> Text {
        self.inner.with_date_style(DateStyle::Time)
    }

    /// Sets the font.
    #[must_use]
    pub fn font(mut self, font: Font) -> Self {
        self.inner = self.inner.font(font);
        self
    }

    /// Enables or disables the bold face on top of the current text style.
    #[must_use]
    pub fn bold(mut self, bold: bool) -> Self {
        self.inner = self.inner.bold(bold);
        self
    }

    /// Sets the foreground color.
    #[must_use]
    pub fn foreground(mut self, color: Color) -> Self {
        self.inner = self.inner.foreground(color);
        self
    }

    /// Sets the foreground color.
    #[must_use]
    pub fn color(self, color: Color) -> Self {
        self.foreground(color)
    }

    /// Caps the text at `limit` laid-out lines.
    #[must_use]
    pub fn line_limit(mut self, limit: u32) -> Self {
        self.inner = self.inner.line_limit(limit);
        self
    }

    /// Applies the `largeTitle` text style preset.
    #[must_use]
    pub fn large_title(mut self) -> Self {
        self.inner = self.inner.large_title();
        self
    }

    /// Applies the `title` text style preset.
    #[must_use]
    pub fn title(mut self) -> Self {
        self.inner = self.inner.title();
        self
    }

    /// Applies the `title2` text style preset.
    #[must_use]
    pub fn title2(mut self) -> Self {
        self.inner = self.inner.title2();
        self
    }

    /// Applies the `title3` text style preset.
    #[must_use]
    pub fn title3(mut self) -> Self {
        self.inner = self.inner.title3();
        self
    }

    /// Applies the `headline` text style preset.
    #[must_use]
    pub fn headline(mut self) -> Self {
        self.inner = self.inner.headline();
        self
    }

    /// Applies the `subheadline` text style preset.
    #[must_use]
    pub fn subheadline(mut self) -> Self {
        self.inner = self.inner.subheadline();
        self
    }

    /// Applies the `body` text style preset.
    #[must_use]
    pub fn body(mut self) -> Self {
        self.inner = self.inner.body();
        self
    }

    /// Applies the `callout` text style preset.
    #[must_use]
    pub fn callout(mut self) -> Self {
        self.inner = self.inner.callout();
        self
    }

    /// Applies the `footnote` text style preset.
    #[must_use]
    pub fn footnote(mut self) -> Self {
        self.inner = self.inner.footnote();
        self
    }

    /// Applies the `caption` text style preset.
    #[must_use]
    pub fn caption(mut self) -> Self {
        self.inner = self.inner.caption();
        self
    }

    /// Applies the `caption2` text style preset.
    #[must_use]
    pub fn caption2(mut self) -> Self {
        self.inner = self.inner.caption2();
        self
    }
}

impl WidgetContent for DateText {
    fn into_node(self) -> Node {
        Node::Text(self.inner)
    }
}

impl From<DateText> for Text {
    fn from(date_text: DateText) -> Self {
        date_text.inner
    }
}

/// Conversion trait for localizable accessibility labels.
pub trait IntoLabelText {
    /// Converts this value into a [`LabelText`].
    fn into_label_text(self) -> LabelText;
}

impl IntoLabelText for LabelText {
    fn into_label_text(self) -> LabelText {
        self
    }
}

impl IntoLabelText for &'static str {
    fn into_label_text(self) -> LabelText {
        LabelText::localized(self)
    }
}

impl IntoLabelText for String {
    fn into_label_text(self) -> LabelText {
        LabelText::verbatim(self)
    }
}

impl IntoLabelText for Str {
    fn into_label_text(self) -> LabelText {
        LabelText::verbatim(String::from(self))
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Leaf constructors
// ══════════════════════════════════════════════════════════════════════════

/// Creates a widget text leaf using [`IntoText`] rules.
///
/// Pass a `&'static str` for localized text, a `String`/`Str` for verbatim
/// text, or a [`Date`] for live host-formatted date text
/// (`text(Date::now()).relative()`).
pub fn text<T: IntoText>(content: T) -> T::Output {
    content.into_text()
}

/// Creates flexible empty space along the enclosing stack's main axis.
#[must_use]
pub const fn spacer() -> Spacer {
    Spacer::new()
}

/// Creates flexible empty space with a minimum length in points/dp.
#[must_use]
pub const fn spacer_min(min_length: f32) -> Spacer {
    Spacer::with_min_length(min_length)
}

/// Creates a system symbol image (SF Symbol on Apple, matching drawable on Android).
pub fn image_system(name: impl Into<String>) -> Image {
    Image::sf_symbol(name)
}

/// Creates an image from inline encoded image bytes (PNG).
pub fn image_data(data: impl Into<Vec<u8>>) -> Image {
    Image::data(data)
}

impl Font {
    /// A `SwiftUI` text style.
    #[must_use]
    pub const fn style(style: FontStyle) -> Self {
        Self::Named { style }
    }

    /// A system font at an explicit size and weight.
    #[must_use]
    pub const fn system(size: f64, weight: FontWeight) -> Self {
        Self::System { size, weight }
    }
}

impl Color {
    /// An sRGB color with full opacity.
    #[must_use]
    pub const fn srgb(red: f32, green: f32, blue: f32) -> Self {
        Self::Srgb {
            red,
            green,
            blue,
            opacity: 1.0,
        }
    }

    /// An sRGB color from 0–255 channels with full opacity.
    #[must_use]
    pub const fn srgb_u8(red: u8, green: u8, blue: u8) -> Self {
        Self::Srgb {
            red: red as f32 / 255.0,
            green: green as f32 / 255.0,
            blue: blue as f32 / 255.0,
            opacity: 1.0,
        }
    }

    /// A semantic system color.
    #[must_use]
    pub const fn semantic(semantic: SemanticColor) -> Self {
        Self::Semantic { semantic }
    }

    /// `Color.primary`.
    pub const PRIMARY: Self = Self::Semantic {
        semantic: SemanticColor::Primary,
    };
    /// `Color.secondary`.
    pub const SECONDARY: Self = Self::Semantic {
        semantic: SemanticColor::Secondary,
    };
    /// `Color.accentColor`.
    pub const ACCENT: Self = Self::Semantic {
        semantic: SemanticColor::Accent,
    };

    /// Re-expresses the color at the given opacity. No-op for semantic
    /// colors, which own their alpha.
    #[must_use]
    pub const fn with_opacity(self, opacity: f32) -> Self {
        match self {
            Self::Srgb {
                red, green, blue, ..
            } => Self::Srgb {
                red,
                green,
                blue,
                opacity,
            },
            semantic => semantic,
        }
    }
}

/// A standalone fill — place a [`Color`] where a node goes to paint the
/// space it occupies.
#[must_use]
pub const fn fill(color: Color) -> Color {
    color
}

/// A tappable region that opens `url`.
pub fn link(url: impl Into<String>, content: impl WidgetContent) -> Link {
    Link::new(url, content)
}

impl Intent {
    /// A tappable region that performs `action`.
    ///
    /// `action` is a `#[widget_intent]` declaration — the extension decodes
    /// the concrete `AppIntent` named by the action's kind and calls its
    /// `perform()` with the action's serialized parameters.
    ///
    /// # Contract violation
    ///
    /// When `action` cannot be serialized as JSON — the action's own
    /// `Serialize` implementation failed. A debug build panics; a release
    /// build logs it and the region carries an empty payload, which the
    /// extension cannot decode into an action.
    pub fn new<I: DeclaredWidgetIntent>(action: &I, label: impl WidgetContent) -> Self {
        let payload = serde_json::to_string(action).unwrap_or_else(|error| {
            contract_violation!(
                "the WidgetIntent action `{}` must serialize as JSON: {error}; \
                 the region carries an empty payload",
                <I as DeclaredWidgetIntent>::KIND
            );
            String::new()
        });
        Self::from_action(
            <I as DeclaredWidgetIntent>::KIND,
            payload,
            label.into_node(),
        )
    }
}

/// A tappable region that performs `action`.
///
/// A string cannot substitute for a declared action:
///
/// ```compile_fail
/// use waterui_widgets::prelude::*;
/// let _ = intent(&"undeclared", text("Run"));
/// ```
///
/// Implementing the action alone does not declare its native handler:
///
/// ```compile_fail
/// use waterui_widgets::prelude::*;
///
/// #[derive(serde::Serialize, serde::Deserialize)]
/// struct Undeclared;
///
/// impl WidgetIntent for Undeclared {
///     fn perform(self, _store: &WidgetStore) {}
/// }
///
/// let _ = intent(&Undeclared, text("Run"));
/// ```
pub fn intent<I: DeclaredWidgetIntent>(action: &I, label: impl WidgetContent) -> Intent {
    Intent::new(action, label)
}

// ══════════════════════════════════════════════════════════════════════════
// Stacks
// ══════════════════════════════════════════════════════════════════════════

/// A vertical stack (`VStack`) of widget children.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct VStack {
    stack: Stack,
}

/// A horizontal stack (`HStack`) of widget children.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct HStack {
    stack: Stack,
}

/// A depth stack (`ZStack`) of widget children.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ZStack {
    stack: Stack,
}

macro_rules! stack_impls {
    ($ty:ident, $ctor:ident, $axis:ident) => {
        impl $ty {
            /// Sets the spacing between children in points.
            #[must_use]
            pub const fn spacing(mut self, spacing: f32) -> Self {
                self.stack.set_spacing(Some(spacing));
                self
            }
        }

        impl WidgetContent for $ty {
            fn into_node(self) -> Node {
                Node::Stack(self.stack)
            }
        }

        /// Creates a stack with default alignment and platform spacing.
        pub fn $ctor<C: WidgetChildren>(children: C) -> $ty {
            $ty {
                stack: Stack::new(Axis::$axis, children.into_nodes()),
            }
        }
    };
}

/// Applies `change` to the stack's alignment, creating an empty one first.
fn align(stack: &mut Stack, change: impl FnOnce(&mut Alignment)) {
    let mut alignment = Stack::alignment(stack).unwrap_or_else(|| Alignment::new(None, None));
    change(&mut alignment);
    stack.set_alignment(Some(alignment));
}

stack_impls!(VStack, vstack, Vertical);
stack_impls!(HStack, hstack, Horizontal);
stack_impls!(ZStack, zstack, Depth);

impl VStack {
    /// Sets the horizontal alignment of the stack's children.
    #[must_use]
    pub fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        align(&mut self.stack, |aligned| {
            aligned.set_horizontal(Some(alignment));
        });
        self
    }
}

impl HStack {
    /// Sets the vertical alignment of the stack's children.
    #[must_use]
    pub fn alignment(mut self, alignment: VerticalAlignment) -> Self {
        align(&mut self.stack, |aligned| {
            aligned.set_vertical(Some(alignment));
        });
        self
    }
}

impl ZStack {
    /// Sets the horizontal alignment of the stack's children.
    #[must_use]
    pub fn horizontal_alignment(mut self, alignment: HorizontalAlignment) -> Self {
        align(&mut self.stack, |aligned| {
            aligned.set_horizontal(Some(alignment));
        });
        self
    }

    /// Sets the vertical alignment of the stack's children.
    #[must_use]
    pub fn vertical_alignment(mut self, alignment: VerticalAlignment) -> Self {
        align(&mut self.stack, |aligned| {
            aligned.set_vertical(Some(alignment));
        });
        self
    }
}

impl From<VStack> for Stack {
    fn from(stack: VStack) -> Self {
        stack.stack
    }
}
impl From<HStack> for Stack {
    fn from(stack: HStack) -> Self {
        stack.stack
    }
}
impl From<ZStack> for Stack {
    fn from(stack: ZStack) -> Self {
        stack.stack
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Timeline
// ══════════════════════════════════════════════════════════════════════════

impl Date {
    /// The current instant.
    ///
    /// A system clock set before the Unix epoch gives a negative offset.
    #[must_use]
    pub fn now() -> Self {
        Self::from(SystemTime::now())
    }

    /// An instant `seconds` after the Unix epoch.
    #[must_use]
    pub const fn unix_timestamp(seconds: f64) -> Self {
        Self(seconds)
    }

    /// The instant as seconds since the Unix epoch.
    #[must_use]
    pub const fn unix_seconds(self) -> f64 {
        self.0
    }
}

impl From<SystemTime> for Date {
    fn from(time: SystemTime) -> Self {
        // An instant before the epoch is a negative offset from it.
        Self(match time.duration_since(UNIX_EPOCH) {
            Ok(after) => after.as_secs_f64(),
            Err(before) => -before.duration().as_secs_f64(),
        })
    }
}

impl Add<Duration> for Date {
    type Output = Self;
    fn add(self, duration: Duration) -> Self {
        Self(self.0 + duration.as_secs_f64())
    }
}

impl Sub<Duration> for Date {
    type Output = Self;
    fn sub(self, duration: Duration) -> Self {
        Self(self.0 - duration.as_secs_f64())
    }
}

impl From<f64> for Date {
    fn from(seconds: f64) -> Self {
        Self::unix_timestamp(seconds)
    }
}
