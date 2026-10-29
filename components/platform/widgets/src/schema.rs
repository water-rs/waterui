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

use crate::authoring::WidgetContent;

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

impl Text {
    /// The text to display.
    #[must_use]
    pub const fn content(&self) -> &str {
        self.content.as_str()
    }

    /// The font to render with; `None` uses the widget default (`.body`).
    #[must_use]
    pub const fn get_font(&self) -> Option<&Font> {
        self.font.as_ref()
    }

    /// The foreground color; `None` uses the primary label color.
    #[must_use]
    pub const fn get_color(&self) -> Option<&Color> {
        self.color.as_ref()
    }

    /// A maximum line count, `SwiftUI`'s `lineLimit`.
    #[must_use]
    pub const fn get_line_limit(&self) -> Option<u32> {
        self.line_limit
    }
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

impl Image {
    /// Where the image's pixels come from.
    #[must_use]
    pub const fn source(&self) -> &ImageSource {
        &self.source
    }

    /// The foreground color for template (monochrome) sources such as SF
    /// Symbols; `None` uses the symbol's own rendering.
    #[must_use]
    pub const fn get_color(&self) -> Option<&Color> {
        self.color.as_ref()
    }
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

impl Stack {
    /// Which axis the children are laid out along.
    #[must_use]
    pub const fn axis(&self) -> &Axis {
        &self.axis
    }

    /// The children, in order.
    #[must_use]
    pub const fn children(&self) -> &[Node] {
        self.children.as_slice()
    }

    /// Spacing between children; `None` uses the platform default.
    #[must_use]
    pub const fn spacing(&self) -> Option<f32> {
        self.spacing
    }

    /// Cross-axis alignment of the children.
    #[must_use]
    pub const fn alignment(&self) -> Option<Alignment> {
        self.alignment
    }
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

impl Stack {
    /// A stack of `children` along `axis` with the platform spacing and the
    /// default alignment.
    #[must_use]
    pub const fn new(axis: Axis, children: Vec<Node>) -> Self {
        Self {
            axis,
            children,
            spacing: None,
            alignment: None,
        }
    }

    /// Sets the spacing between children; `None` uses the platform default.
    pub const fn set_spacing(&mut self, spacing: Option<f32>) {
        self.spacing = spacing;
    }

    /// Sets the cross-axis alignment of the children.
    pub const fn set_alignment(&mut self, alignment: Option<Alignment>) {
        self.alignment = alignment;
    }
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

impl Alignment {
    /// The horizontal component, when set.
    #[must_use]
    pub const fn horizontal(&self) -> Option<HorizontalAlignment> {
        self.horizontal
    }

    /// The vertical component, when set.
    #[must_use]
    pub const fn vertical(&self) -> Option<VerticalAlignment> {
        self.vertical
    }
}

impl Alignment {
    /// An alignment with the given components.
    #[must_use]
    pub const fn new(
        horizontal: Option<HorizontalAlignment>,
        vertical: Option<VerticalAlignment>,
    ) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }

    /// Sets the horizontal component.
    pub const fn set_horizontal(&mut self, horizontal: Option<HorizontalAlignment>) {
        self.horizontal = horizontal;
    }

    /// Sets the vertical component.
    pub const fn set_vertical(&mut self, vertical: Option<VerticalAlignment>) {
        self.vertical = vertical;
    }
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

impl Shape {
    /// The shape's geometry.
    #[must_use]
    pub const fn kind(&self) -> &ShapeKind {
        &self.kind
    }

    /// What fills it.
    #[must_use]
    pub const fn get_fill(&self) -> &Color {
        &self.fill
    }
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

impl Link {
    /// The URL opened on tap — a deep link into the app or a web URL.
    #[must_use]
    pub const fn url(&self) -> &str {
        self.url.as_str()
    }

    /// The rendered label.
    #[must_use]
    pub fn content(&self) -> &Node {
        &self.content
    }
}

/// A tappable region performing a declared intent.
///
/// `kind` is the `#[widget_intent]` declaration's kind; `payload` is the
/// action's serialized parameters — the extension decodes the concrete
/// `AppIntent` named by `kind` and calls its `perform()` with `payload`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Intent {
    kind: String,
    payload: String,
    label: Box<Node>,
}

impl Intent {
    /// A tappable region carrying `action_kind`/`action_payload`.
    pub(crate) fn from_action(action_kind: &str, action_payload: String, label: Node) -> Self {
        Self {
            kind: action_kind.to_string(),
            payload: action_payload,
            label: Box::new(label),
        }
    }

    /// The declared intent kind.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The action's serialized parameters.
    #[must_use]
    pub fn payload(&self) -> &str {
        &self.payload
    }

    /// The rendered label.
    #[must_use]
    pub fn label(&self) -> &Node {
        &self.label
    }
}

/// A user-declared widget action — `#[widget_intent]` on the action type.
///
/// The action is serialized to the `Intent` node's `payload`; the
/// extension decodes the concrete `AppIntent` named by the action's kind
/// and calls `perform()` with it. `perform` is a synchronous unit action —
/// the extension owns no application state, so the action is its own
/// context.
pub trait WidgetIntent: Serialize + serde::de::DeserializeOwned + 'static {
    /// The action's effect — run in the extension's process.
    fn perform(self);
}

/// The identity emitted by `#[widget_intent]` for a declared action.
#[doc(hidden)]
pub trait DeclaredWidgetIntent: WidgetIntent {
    /// The declared kind — unique per application.
    const KIND: &'static str;
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

impl TimelineEntry {
    /// When this entry becomes the widget's rendered content, in seconds
    /// since the Unix epoch.
    #[must_use]
    pub const fn date(&self) -> Date {
        self.date
    }

    /// The snapshot rendered for this entry.
    #[must_use]
    pub const fn content(&self) -> &Node {
        &self.content
    }
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

impl Timeline {
    /// The dated entries, in chronological order.
    #[must_use]
    pub const fn entries(&self) -> &[TimelineEntry] {
        self.entries.as_slice()
    }

    /// When `WidgetKit` should ask for a new timeline.
    #[must_use]
    pub const fn get_policy(&self) -> ReloadPolicy {
        self.policy
    }
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

impl WidgetContext {
    /// The widget family being rendered.
    #[must_use]
    pub const fn family(&self) -> WidgetFamily {
        self.family
    }

    /// The size the system proposes for the widget's content.
    #[must_use]
    pub const fn display_size(&self) -> Size {
        self.display_size
    }
}

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
