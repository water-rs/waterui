//! The widget snapshot wire schema.
//!
//! Every type here is a plain serializable value: it is what crosses the
//! boundary from the Rust timeline producer to the generated `WidgetKit`
//! extension, decoded by the Swift mirror in `swift/Schema.swift`. Nothing in
//! this module is a `View`, a signal, or an environment — a snapshot carries
//! resolved values only.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

/// One node of a widget snapshot.
///
/// The closed set `WidgetKit`'s renderer understands. Every variant name is
/// serialized as the node's `type` tag; the variant's fields sit beside it
/// (`{"type":"text","content":"…"}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum Node {
    /// A text leaf.
    Text(Text),
    /// An image leaf.
    Image(Image),
    /// A stack container (`axis` selects `HStack`/`VStack`/`ZStack`).
    Stack(Stack),
    /// A filled shape.
    Shape(Shape),
    /// A color filling the space it is placed in.
    Color(Color),
    /// A tappable region that opens a URL.
    Link(Link),
    /// A tappable region that performs a declared intent.
    Intent(Intent),
}

/// A text leaf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Text {
    /// The text to display.
    pub content: String,
    /// The font to render with; `None` uses the widget default (`.body`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font: Option<Font>,
    /// The foreground color; `None` uses the primary label color.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
    /// A maximum line count, `SwiftUI`'s `lineLimit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_limit: Option<u32>,
}

/// A font a text leaf renders with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum Font {
    /// A `SwiftUI` text style (`.title`, `.body`, …).
    Named {
        /// The text style.
        style: FontStyle,
    },
    /// A system font at an explicit size and weight.
    System {
        /// Point size.
        size: f64,
        /// The weight.
        weight: FontWeight,
    },
}

/// The `SwiftUI` text styles a widget text can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FontStyle {
    /// `Font.largeTitle`.
    LargeTitle,
    /// `Font.title`.
    Title,
    /// `Font.title2`.
    Title2,
    /// `Font.title3`.
    Title3,
    /// `Font.headline`.
    Headline,
    /// `Font.subheadline`.
    Subheadline,
    /// `Font.body`.
    Body,
    /// `Font.callout`.
    Callout,
    /// `Font.footnote`.
    Footnote,
    /// `Font.caption`.
    Caption,
    /// `Font.caption2`.
    Caption2,
}

/// The `SwiftUI` font weight vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FontWeight {
    /// `Font.Weight.ultraLight`.
    UltraLight,
    /// `Font.Weight.thin`.
    Thin,
    /// `Font.Weight.light`.
    Light,
    /// `Font.Weight.regular`.
    Regular,
    /// `Font.Weight.medium`.
    Medium,
    /// `Font.Weight.semibold`.
    Semibold,
    /// `Font.Weight.bold`.
    Bold,
    /// `Font.Weight.heavy`.
    Heavy,
    /// `Font.Weight.black`.
    Black,
}

/// A color value in a snapshot: either explicit sRGB or a semantic system
/// color, so widget content follows the user's appearance.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum Color {
    /// An explicit sRGB color.
    Srgb {
        /// Red channel, 0.0–1.0.
        red: f32,
        /// Green channel, 0.0–1.0.
        green: f32,
        /// Blue channel, 0.0–1.0.
        blue: f32,
        /// Opacity, 0.0–1.0.
        opacity: f32,
    },
    /// A semantic `SwiftUI` color that resolves with appearance.
    Semantic {
        /// Which semantic color.
        semantic: SemanticColor,
    },
}

/// The semantic colors a widget can name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SemanticColor {
    /// `Color.primary`.
    Primary,
    /// `Color.secondary`.
    Secondary,
    /// `Color.accentColor`.
    Accent,
}

/// An image leaf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Image {
    /// Where the image's pixels come from.
    pub source: ImageSource,
    /// The foreground color for template (monochrome) sources such as SF
    /// Symbols; `None` uses the symbol's own rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

/// Where an image's pixels come from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ImageSource {
    /// An SF Symbol by name — an Apple-only primitive (repository principle:
    /// asymmetric primitives are documented, not faked).
    System {
        /// The symbol name, e.g. `figure.walk`.
        name: String,
    },
    /// Inline encoded image bytes (PNG), decoded inside the extension.
    Data {
        /// The encoded image.
        data: Vec<u8>,
    },
}

/// A stack container.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Stack {
    /// Which axis the children are laid out along.
    pub axis: Axis,
    /// The children, in order.
    pub children: Vec<Node>,
    /// Spacing between children; `None` uses the platform default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f32>,
    /// Cross-axis alignment of the children.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alignment: Option<Alignment>,
}

/// The axis a stack lays out along.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum Axis {
    /// `HStack` — children in a row.
    Horizontal,
    /// `VStack` — children in a column.
    Vertical,
    /// `ZStack` — children layered back to front.
    Depth,
}

/// Horizontal alignment within a stack's cross axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum HorizontalAlignment {
    /// Leading edge.
    Leading,
    /// Centered.
    Center,
    /// Trailing edge.
    Trailing,
}

/// Vertical alignment within a stack's cross axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum VerticalAlignment {
    /// Top edge.
    Top,
    /// Centered.
    Center,
    /// Bottom edge.
    Bottom,
}

/// The alignment a stack applies: `horizontal` for a vertical stack,
/// `vertical` for a horizontal stack, both for a depth stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Alignment {
    /// The horizontal component, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub horizontal: Option<HorizontalAlignment>,
    /// The vertical component, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical: Option<VerticalAlignment>,
}

/// A shape filled with a color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Shape {
    /// The shape's geometry.
    pub kind: ShapeKind,
    /// What fills it.
    pub fill: Color,
}

/// The shape vocabulary the widget renderer draws.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ShapeKind {
    /// `Rectangle`.
    Rectangle,
    /// `RoundedRectangle` with a corner radius.
    RoundedRectangle {
        /// The corner radius in points.
        corner_radius: f32,
    },
    /// `Capsule`.
    Capsule,
    /// `Circle`.
    Circle,
    /// `Ellipse`.
    Ellipse,
}

/// A tappable region opening a URL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Link {
    /// The URL opened on tap — a deep link into the app or a web URL.
    pub url: String,
    /// The rendered label.
    pub content: Box<Node>,
}

/// A tappable region performing a declared intent.
///
/// The `name` identifies the intent the extension performs; the generated
/// extension bridges it to an `AppIntent` that opens the application with
/// the declared name as a `waterui-intent://` URL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Intent {
    /// The declared intent name.
    pub name: String,
    /// The rendered label.
    pub label: Box<Node>,
}

/// One dated entry of a widget timeline: an immutable snapshot and the
/// instant it becomes current.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct TimelineEntry {
    /// When this entry becomes the widget's rendered content, in seconds
    /// since the Unix epoch.
    pub date: Date,
    /// The snapshot rendered for this entry.
    pub content: Node,
}

/// A `WidgetKit` timeline: the dated entries plus the reload policy the
/// provider reports to the system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Timeline {
    /// The dated entries, in chronological order.
    pub entries: Vec<TimelineEntry>,
    /// When `WidgetKit` should ask for a new timeline.
    pub policy: ReloadPolicy,
}

/// When the extension asks for the next timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum ReloadPolicy {
    /// Reload after the last entry's date passes.
    AtEnd,
    /// Never reload; the timeline stands.
    Never,
    /// Reload at a specific instant.
    After {
        /// The instant to reload, in seconds since the Unix epoch.
        date: Date,
    },
}

/// An instant on the `WidgetKit` timeline, serialized as seconds since the
/// Unix epoch (`Swift.Date(timeIntervalSince1970:)`).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Date(pub(crate) f64);

/// The widget family the extension is rendering for, mirroring
/// `WidgetKit.WidgetFamily` case names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WidgetFamily {
    /// `WidgetFamily.systemSmall`.
    SystemSmall,
    /// `WidgetFamily.systemMedium`.
    SystemMedium,
    /// `WidgetFamily.systemLarge`.
    SystemLarge,
    /// `WidgetFamily.systemExtraLarge`.
    SystemExtraLarge,
    /// `WidgetFamily.accessoryCircular` — lock-screen / corner circular.
    AccessoryCircular,
    /// `WidgetFamily.accessoryRectangular` — lock-screen rectangular.
    AccessoryRectangular,
    /// `WidgetFamily.accessoryInline` — lock-screen inline text.
    AccessoryInline,
}

/// A pixel-size box, in points.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Size {
    /// Width in points.
    pub width: f64,
    /// Height in points.
    pub height: f64,
}

/// The context the extension hands the Rust timeline producer for each
/// timeline request — the family being rendered and the display size the
/// system proposed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WidgetContext {
    /// The widget family being rendered.
    pub family: WidgetFamily,
    /// The size the system proposes for the widget's content.
    pub display_size: Size,
}
