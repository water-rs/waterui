//! The restricted authoring surface.
//!
//! Only types implementing [`WidgetContent`] can appear inside a widget, and
//! only [`WidgetChildren`] implementations can nest inside a stack. Neither
//! trait is implemented by `waterui::View` or any `WaterUI` component — a
//! normal view in widget position fails to compile, which is the mechanism
//! the design calls "unsupported components are a compile-time error".

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;

use suiteki::Str;

use crate::schema::{
    Alignment, Axis, Color, Date, Font, FontStyle, FontWeight, HorizontalAlignment, Image,
    ImageSource, Intent, Link, Node, ReloadPolicy, SemanticColor, Shape, ShapeKind, Stack, Text,
    Timeline, TimelineEntry, VerticalAlignment,
};

/// A value that can occupy a widget snapshot.
///
/// Implemented by every schema node type and by the string types that
/// convert to [`Text`]. `View`s, signals, environments and `AnyView`s do not
/// implement it — nothing reactive or platform-bound can cross into a
/// snapshot.
///
/// The closed surface is what makes an unsupported component a compile-time
/// error rather than a runtime drop: a `WaterUI` view in widget position
/// does not compile —
///
/// ```compile_fail
/// use waterui_widgets::vstack;
/// let _ = vstack((waterui_text::text("inside an app"),));
/// ```
///
/// — and neither does a value with no schema representation:
///
/// ```compile_fail
/// use waterui_widgets::vstack;
/// let _ = vstack((42,));
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot appear in a widget",
    label = "expected a widget node",
    note = "the widget schema admits text, image, stack, shape, color, link and intent nodes only. `WaterUI` views (`Text`, `Button`, `VStack`, …) are not widget content; use the `waterui::widgets` constructors instead."
)]
pub trait WidgetContent {
    /// Convert into the wire node.
    fn into_node(self) -> Node;
}

/// A value usable as the root of a timeline-producing function.
///
/// Any [`WidgetContent`] produces a single-entry timeline dated "now" with
/// the default [`ReloadPolicy::AtEnd`]; an explicit [`TimelineEntry`] or
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
    note = "stack children are tuples of `WidgetContent` values — `(text(\"a\"), fill(color))` — or `Vec`/`Option`/`[_; N]` of `WidgetContent`. `WaterUI` views and tuples containing them do not qualify."
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
        self.map_or_else(Vec::new, |node| alloc::vec![node.into_node()])
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
        #[allow(non_snake_case)]
        impl<$($ty: WidgetContent),*> WidgetChildren for ($($ty,)*) {
            fn into_nodes(self) -> Vec<Node> {
                let ($($ty,)*) = self;
                alloc::vec![$($ty.into_node()),*]
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
    Link => Link,
    Intent => Intent
);

impl WidgetContent for &'static str {
    fn into_node(self) -> Node {
        Node::Text(Text::new(self))
    }
}

impl WidgetContent for String {
    fn into_node(self) -> Node {
        Node::Text(Text::new(self))
    }
}

impl WidgetContent for Str {
    fn into_node(self) -> Node {
        Node::Text(Text::new(String::from(self)))
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Leaf constructors
// ══════════════════════════════════════════════════════════════════════════

impl Text {
    /// Creates a text leaf with default styling.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            font: None,
            color: None,
            line_limit: None,
        }
    }

    /// Sets the font.
    #[must_use]
    pub const fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// Sets the foreground color.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Caps the text at `limit` laid-out lines.
    #[must_use]
    pub const fn line_limit(mut self, limit: u32) -> Self {
        self.line_limit = Some(limit);
        self
    }
}

/// Creates a text leaf — the ergonomic entry point.
pub fn text(content: impl Into<String>) -> Text {
    Text::new(content)
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

impl Image {
    /// An SF Symbol — an Apple-native primitive, supported on Apple widgets.
    pub fn system(name: impl Into<String>) -> Self {
        Self {
            source: ImageSource::System { name: name.into() },
            color: None,
        }
    }

    /// Inline encoded image bytes (PNG).
    pub fn data(data: impl Into<Vec<u8>>) -> Self {
        Self {
            source: ImageSource::Data { data: data.into() },
            color: None,
        }
    }

    /// Tints a monochrome (template) source.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }
}

impl Shape {
    /// `Rectangle`, filled with the primary semantic color.
    #[must_use]
    pub const fn rectangle() -> Self {
        Self::of_kind(ShapeKind::Rectangle)
    }

    /// `RoundedRectangle` with a corner radius.
    #[must_use]
    pub const fn rounded_rectangle(corner_radius: f32) -> Self {
        Self::of_kind(ShapeKind::RoundedRectangle { corner_radius })
    }

    /// `Capsule`.
    #[must_use]
    pub const fn capsule() -> Self {
        Self::of_kind(ShapeKind::Capsule)
    }

    /// `Circle`.
    #[must_use]
    pub const fn circle() -> Self {
        Self::of_kind(ShapeKind::Circle)
    }

    /// `Ellipse`.
    #[must_use]
    pub const fn ellipse() -> Self {
        Self::of_kind(ShapeKind::Ellipse)
    }

    const fn of_kind(kind: ShapeKind) -> Self {
        Self {
            kind,
            fill: Color::Semantic {
                semantic: SemanticColor::Primary,
            },
        }
    }

    /// Sets the fill color.
    #[must_use]
    pub const fn fill(mut self, fill: Color) -> Self {
        self.fill = fill;
        self
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

impl Link {
    /// A tappable region that opens `url` — a deep link into the app or a
    /// web URL.
    pub fn new(url: impl Into<String>, content: impl WidgetContent) -> Self {
        Self {
            url: url.into(),
            content: alloc::boxed::Box::new(content.into_node()),
        }
    }
}

/// A tappable region that opens `url`.
pub fn link(url: impl Into<String>, content: impl WidgetContent) -> Link {
    Link::new(url, content)
}

impl Intent {
    /// A tappable region that performs the declared intent `name`.
    ///
    /// The generated extension bridges the name to an `AppIntent` that opens
    /// the application with a `waterui-intent://<name>` URL.
    pub fn new(name: impl Into<String>, label: impl WidgetContent) -> Self {
        Self {
            name: name.into(),
            label: alloc::boxed::Box::new(label.into_node()),
        }
    }
}

/// A tappable region that performs the declared intent `name`.
pub fn intent(name: impl Into<String>, label: impl WidgetContent) -> Intent {
    Intent::new(name, label)
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
                self.stack.spacing = Some(spacing);
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
                stack: Stack {
                    axis: Axis::$axis,
                    children: children.into_nodes(),
                    spacing: None,
                    alignment: None,
                },
            }
        }
    };
}

stack_impls!(VStack, vstack, Vertical);
stack_impls!(HStack, hstack, Horizontal);
stack_impls!(ZStack, zstack, Depth);

impl VStack {
    /// Sets the horizontal alignment of the stack's children.
    #[must_use]
    pub fn alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.stack
            .alignment
            .get_or_insert(Alignment {
                horizontal: None,
                vertical: None,
            })
            .horizontal = Some(alignment);
        self
    }
}

impl HStack {
    /// Sets the vertical alignment of the stack's children.
    #[must_use]
    pub fn alignment(mut self, alignment: VerticalAlignment) -> Self {
        self.stack
            .alignment
            .get_or_insert(Alignment {
                horizontal: None,
                vertical: None,
            })
            .vertical = Some(alignment);
        self
    }
}

impl ZStack {
    /// Sets the horizontal alignment of the stack's children.
    #[must_use]
    pub fn horizontal_alignment(mut self, alignment: HorizontalAlignment) -> Self {
        self.stack
            .alignment
            .get_or_insert(Alignment {
                horizontal: None,
                vertical: None,
            })
            .horizontal = Some(alignment);
        self
    }

    /// Sets the vertical alignment of the stack's children.
    #[must_use]
    pub fn vertical_alignment(mut self, alignment: VerticalAlignment) -> Self {
        self.stack
            .alignment
            .get_or_insert(Alignment {
                horizontal: None,
                vertical: None,
            })
            .vertical = Some(alignment);
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
    /// # Panics
    /// Panics if the system clock is set before the Unix epoch.
    #[must_use]
    pub fn now() -> Self {
        Self(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock is before the Unix epoch")
                .as_secs_f64(),
        )
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

impl From<std::time::SystemTime> for Date {
    fn from(time: std::time::SystemTime) -> Self {
        Self(
            time.duration_since(std::time::UNIX_EPOCH)
                .expect("a widget date before the Unix epoch")
                .as_secs_f64(),
        )
    }
}

impl core::ops::Add<Duration> for Date {
    type Output = Self;
    fn add(self, duration: Duration) -> Self {
        Self(self.0 + duration.as_secs_f64())
    }
}

impl core::ops::Sub<Duration> for Date {
    type Output = Self;
    fn sub(self, duration: Duration) -> Self {
        Self(self.0 - duration.as_secs_f64())
    }
}

impl Timeline {
    /// A timeline of `entries`; the default policy reloads [`ReloadPolicy::AtEnd`].
    pub fn new(entries: impl Into<Vec<TimelineEntry>>) -> Self {
        Self {
            entries: entries.into(),
            policy: ReloadPolicy::AtEnd,
        }
    }

    /// Sets when the extension asks for the next timeline.
    #[must_use]
    pub const fn policy(mut self, policy: ReloadPolicy) -> Self {
        self.policy = policy;
        self
    }
}

impl TimelineEntry {
    /// A snapshot that becomes current at `date`.
    pub fn new(date: impl Into<Date>, content: impl WidgetContent) -> Self {
        Self {
            date: date.into(),
            content: content.into_node(),
        }
    }
}

impl From<f64> for Date {
    fn from(seconds: f64) -> Self {
        Self::unix_timestamp(seconds)
    }
}
