//! The widget snapshot wire schema.
//!
//! Every type here is a plain serializable value: it is what crosses the
//! boundary from the Rust timeline producer to the generated `WidgetKit`
//! extension (decoded by `swift/Schema.swift`) and the Android widget host
//! (decoded by `kotlin/WaterUIWidgetSchema.kt`). Nothing in this module is a
//! `View`, a signal, or an environment — a snapshot carries resolved values
//! only.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::authoring::{IntoLabelText, IntoText, WidgetContent};
use crate::store::WidgetStore;

/// One node of a widget snapshot.
///
/// The closed set the Apple and Android widget hosts understand. Every variant
/// name is serialized as the node's `type` tag; the variant's fields sit beside
/// it (`{"type":"text",…}`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum Node {
    /// A text leaf (localized key, verbatim string, or live host-formatted date).
    Text(Text),
    /// An image leaf.
    Image(Image),
    /// A stack container (`axis` selects `HStack`/`VStack`/`ZStack`).
    Stack(Stack),
    /// A filled shape.
    Shape(Shape),
    /// A color filling the space it is placed in.
    Color(Color),
    /// Insets a child node by explicit edge distances or platform default padding.
    Padding(Padding),
    /// Flexible empty space expanding along the enclosing stack's main axis.
    Spacer(Spacer),
    /// Size and alignment constraints around a child node.
    Frame(Frame),
    /// Paints a background node behind a content node.
    Background(Background),
    /// Attaches a screen-reader accessibility label to a child node.
    AccessibilityLabel(AccessibilityLabel),
    /// A tappable region that opens a URL.
    Link(Link),
    /// A tappable region that performs a declared intent.
    Intent(Intent),
}

/// A localizable or verbatim string used for accessibility labels and similar
/// text metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum LabelText {
    /// A localization key resolved by the host platform's string catalog.
    Localized {
        /// The localization key / default text.
        key: String,
    },
    /// Literal text displayed or spoken without catalog lookup.
    Verbatim {
        /// The literal string.
        text: String,
    },
}

impl LabelText {
    /// Creates a localized label resolved through the host platform's string catalog.
    #[must_use]
    pub fn localized(key: &'static str) -> Self {
        Self::Localized {
            key: key.to_string(),
        }
    }

    /// Creates a verbatim label that bypasses string-catalog lookup.
    pub fn verbatim(text: impl Into<String>) -> Self {
        Self::Verbatim { text: text.into() }
    }

    /// Returns the raw key or literal text carried by this label.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Localized { key } => key.as_str(),
            Self::Verbatim { text } => text.as_str(),
        }
    }
}

/// What a [`Text`] leaf displays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum TextContent {
    /// A localization key resolved by the host platform (`LocalizedStringKey`
    /// on Apple, string resource lookup with literal fallback on Android).
    Localized {
        /// The localization key / default text.
        key: String,
    },
    /// Verbatim text displayed as-is without catalog lookup.
    Verbatim {
        /// The literal text.
        text: String,
    },
    /// A live host-formatted date (such as `Text(date, style: .relative)` on
    /// Apple or `DateUtils` / `Chronometer` on Android).
    Date {
        /// The target instant, in seconds since the Unix epoch.
        date: Date,
        /// How the host formats the date.
        style: DateStyle,
    },
}

/// How a live date [`Text`] leaf is formatted by the host platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum DateStyle {
    /// Relative duration from now (`Text(date, style: .relative)`).
    Relative,
    /// Signed offset from now (`Text(date, style: .offset)`).
    Offset,
    /// Live counting timer (`Text(date, style: .timer)`).
    Timer,
    /// Formatted calendar date (`Text(date, style: .date)`).
    Date,
    /// Formatted time of day (`Text(date, style: .time)`).
    Time,
}

/// A text leaf.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Text {
    /// The text payload to display.
    content: TextContent,
    /// The font to render with; `None` uses the widget default (`.body`).
    #[serde(skip_serializing_if = "Option::is_none")]
    font: Option<Font>,
    /// Explicit bold toggle preserved across text styles.
    #[serde(skip_serializing_if = "Option::is_none")]
    bold: Option<bool>,
    /// The foreground color; `None` uses the primary label color.
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<Color>,
    /// A maximum line count, `SwiftUI`'s `lineLimit`.
    #[serde(skip_serializing_if = "Option::is_none")]
    line_limit: Option<u32>,
}

impl Text {
    /// Creates a text leaf using the [`IntoText`] conversion rules.
    pub fn new<T: IntoText<Output = Self>>(content: T) -> Self {
        content.into_text()
    }

    /// Creates localized text that resolves through the host's string catalog.
    #[must_use]
    pub fn localized(key: &'static str) -> Self {
        Self::from_content(TextContent::Localized {
            key: key.to_string(),
        })
    }

    /// Creates verbatim text that bypasses string-catalog lookup.
    pub fn verbatim(text: impl Into<String>) -> Self {
        Self::from_content(TextContent::Verbatim { text: text.into() })
    }

    /// Creates live date text formatted with `style`.
    #[must_use]
    pub const fn date(date: Date, style: DateStyle) -> Self {
        Self::from_content(TextContent::Date { date, style })
    }

    /// Creates live relative-time text (`Text(date, style: .relative)`).
    #[must_use]
    pub const fn relative(date: Date) -> Self {
        Self::date(date, DateStyle::Relative)
    }

    const fn from_content(content: TextContent) -> Self {
        Self {
            content,
            font: None,
            bold: None,
            color: None,
            line_limit: None,
        }
    }

    /// The text payload to display.
    #[must_use]
    pub const fn content(&self) -> &TextContent {
        &self.content
    }

    /// The font to render with; `None` uses the widget default (`.body`).
    #[must_use]
    pub const fn get_font(&self) -> Option<&Font> {
        self.font.as_ref()
    }

    /// Whether an explicit bold face was requested.
    #[must_use]
    pub const fn is_bold(&self) -> Option<bool> {
        self.bold
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

    /// Sets the font.
    #[must_use]
    pub const fn font(mut self, font: Font) -> Self {
        self.font = Some(font);
        self
    }

    /// Enables or disables the bold face on top of the current text style.
    #[must_use]
    pub const fn bold(mut self, bold: bool) -> Self {
        self.bold = Some(bold);
        self
    }

    /// Sets the foreground color, matching `ViewExt::foreground`.
    #[must_use]
    pub const fn foreground(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Sets the foreground color, matching `waterui_text::Text::color`.
    #[must_use]
    pub const fn color(self, color: Color) -> Self {
        self.foreground(color)
    }

    /// Caps the text at `limit` laid-out lines.
    #[must_use]
    pub const fn line_limit(mut self, limit: u32) -> Self {
        self.line_limit = Some(limit);
        self
    }

    /// Applies the `largeTitle` text style preset.
    #[must_use]
    pub const fn large_title(self) -> Self {
        self.font(Font::style(FontStyle::LargeTitle))
    }

    /// Applies the `title` text style preset.
    #[must_use]
    pub const fn title(self) -> Self {
        self.font(Font::style(FontStyle::Title))
    }

    /// Applies the `title2` text style preset.
    #[must_use]
    pub const fn title2(self) -> Self {
        self.font(Font::style(FontStyle::Title2))
    }

    /// Applies the `title3` text style preset.
    #[must_use]
    pub const fn title3(self) -> Self {
        self.font(Font::style(FontStyle::Title3))
    }

    /// Applies the `headline` text style preset.
    #[must_use]
    pub const fn headline(self) -> Self {
        self.font(Font::style(FontStyle::Headline))
    }

    /// Applies the `subheadline` text style preset.
    #[must_use]
    pub const fn subheadline(self) -> Self {
        self.font(Font::style(FontStyle::Subheadline))
    }

    /// Applies the `body` text style preset.
    #[must_use]
    pub const fn body(self) -> Self {
        self.font(Font::style(FontStyle::Body))
    }

    /// Applies the `callout` text style preset.
    #[must_use]
    pub const fn callout(self) -> Self {
        self.font(Font::style(FontStyle::Callout))
    }

    /// Applies the `footnote` text style preset.
    #[must_use]
    pub const fn footnote(self) -> Self {
        self.font(Font::style(FontStyle::Footnote))
    }

    /// Applies the `caption` text style preset.
    #[must_use]
    pub const fn caption(self) -> Self {
        self.font(Font::style(FontStyle::Caption))
    }

    /// Applies the `caption2` text style preset.
    #[must_use]
    pub const fn caption2(self) -> Self {
        self.font(Font::style(FontStyle::Caption2))
    }

    pub(crate) fn with_date_style(mut self, style: DateStyle) -> Self {
        if let TextContent::Date { date, .. } = self.content {
            self.content = TextContent::Date { date, style };
        }
        self
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
    /// A semantic platform color that resolves with appearance.
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
    source: ImageSource,
    /// The foreground color for template (monochrome) sources such as SF
    /// Symbols; `None` uses the symbol's own rendering.
    #[serde(skip_serializing_if = "Option::is_none")]
    color: Option<Color>,
}

impl Image {
    /// An SF Symbol (`waterui_apple::SfSymbol`) — an Apple-native primitive,
    /// supported on Apple widgets and resolved by drawable name on Android.
    pub fn sf_symbol(name: impl Into<String>) -> Self {
        Self {
            source: ImageSource::SfSymbol { name: name.into() },
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

    /// Tints a monochrome (template) source, matching `ViewExt::foreground`.
    #[must_use]
    pub const fn foreground(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Tints a monochrome (template) source.
    #[must_use]
    pub const fn color(self, color: Color) -> Self {
        self.foreground(color)
    }

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
    /// A system symbol name (Apple SF Symbol / Android drawable lookup).
    SfSymbol {
        /// The symbol name, e.g. `figure.walk`.
        name: String,
    },
    /// Inline encoded image bytes (PNG), decoded inside the host.
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
    axis: Axis,
    /// The children, in order.
    children: Vec<Node>,
    /// Spacing between children; `None` uses the platform default.
    #[serde(skip_serializing_if = "Option::is_none")]
    spacing: Option<f32>,
    /// Cross-axis alignment of the children.
    #[serde(skip_serializing_if = "Option::is_none")]
    alignment: Option<Alignment>,
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

    /// Sets the spacing between children; `None` uses the platform default.
    pub const fn set_spacing(&mut self, spacing: Option<f32>) {
        self.spacing = spacing;
    }

    /// Sets the cross-axis alignment of the children.
    pub const fn set_alignment(&mut self, alignment: Option<Alignment>) {
        self.alignment = alignment;
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

/// Horizontal alignment within a stack or frame.
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

/// Vertical alignment within a stack or frame.
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

/// Two-dimensional alignment for depth stacks and frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Alignment {
    /// The horizontal component, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    horizontal: Option<HorizontalAlignment>,
    /// The vertical component, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    vertical: Option<VerticalAlignment>,
}

impl Alignment {
    /// Centered horizontally and vertically.
    pub const CENTER: Self = Self::new(
        Some(HorizontalAlignment::Center),
        Some(VerticalAlignment::Center),
    );
    /// Leading horizontally, centered vertically.
    pub const LEADING: Self = Self::new(
        Some(HorizontalAlignment::Leading),
        Some(VerticalAlignment::Center),
    );
    /// Trailing horizontally, centered vertically.
    pub const TRAILING: Self = Self::new(
        Some(HorizontalAlignment::Trailing),
        Some(VerticalAlignment::Center),
    );
    /// Centered horizontally, top vertically.
    pub const TOP: Self = Self::new(
        Some(HorizontalAlignment::Center),
        Some(VerticalAlignment::Top),
    );
    /// Centered horizontally, bottom vertically.
    pub const BOTTOM: Self = Self::new(
        Some(HorizontalAlignment::Center),
        Some(VerticalAlignment::Bottom),
    );
    /// Leading and top.
    pub const TOP_LEADING: Self = Self::new(
        Some(HorizontalAlignment::Leading),
        Some(VerticalAlignment::Top),
    );
    /// Trailing and top.
    pub const TOP_TRAILING: Self = Self::new(
        Some(HorizontalAlignment::Trailing),
        Some(VerticalAlignment::Top),
    );
    /// Leading and bottom.
    pub const BOTTOM_LEADING: Self = Self::new(
        Some(HorizontalAlignment::Leading),
        Some(VerticalAlignment::Bottom),
    );
    /// Trailing and bottom.
    pub const BOTTOM_TRAILING: Self = Self::new(
        Some(HorizontalAlignment::Trailing),
        Some(VerticalAlignment::Bottom),
    );

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

    /// Sets the horizontal component.
    pub const fn set_horizontal(&mut self, horizontal: Option<HorizontalAlignment>) {
        self.horizontal = horizontal;
    }

    /// Sets the vertical component.
    pub const fn set_vertical(&mut self, vertical: Option<VerticalAlignment>) {
        self.vertical = vertical;
    }
}

impl From<HorizontalAlignment> for Alignment {
    fn from(horizontal: HorizontalAlignment) -> Self {
        Self::new(Some(horizontal), Some(VerticalAlignment::Center))
    }
}

impl From<VerticalAlignment> for Alignment {
    fn from(vertical: VerticalAlignment) -> Self {
        Self::new(Some(HorizontalAlignment::Center), Some(vertical))
    }
}

/// Edge insets in points / density-independent pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EdgeInsets {
    /// Top inset.
    pub top: f32,
    /// Bottom inset.
    pub bottom: f32,
    /// Leading inset (left in LTR, right in RTL).
    pub leading: f32,
    /// Trailing inset (right in LTR, left in RTL).
    pub trailing: f32,
}

impl EdgeInsets {
    /// Uniform insets on every edge.
    #[must_use]
    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            bottom: value,
            leading: value,
            trailing: value,
        }
    }

    /// Symmetric vertical and horizontal insets.
    #[must_use]
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            bottom: vertical,
            leading: horizontal,
            trailing: horizontal,
        }
    }

    /// Vertical (top and bottom) insets with zero horizontal insets.
    #[must_use]
    pub const fn vertical(value: f32) -> Self {
        Self::symmetric(value, 0.0)
    }

    /// Horizontal (leading and trailing) insets with zero vertical insets.
    #[must_use]
    pub const fn horizontal(value: f32) -> Self {
        Self::symmetric(0.0, value)
    }

    /// Returns a copy with top and bottom set to `value`.
    #[must_use]
    pub const fn with_vertical(mut self, value: f32) -> Self {
        self.top = value;
        self.bottom = value;
        self
    }

    /// Returns a copy with leading and trailing set to `value`.
    #[must_use]
    pub const fn with_horizontal(mut self, value: f32) -> Self {
        self.leading = value;
        self.trailing = value;
        self
    }

    /// Explicit insets for each edge in `top, leading, bottom, trailing` order.
    #[must_use]
    pub const fn new(top: f32, leading: f32, bottom: f32, trailing: f32) -> Self {
        Self {
            top,
            bottom,
            leading,
            trailing,
        }
    }
}

impl From<f32> for EdgeInsets {
    fn from(value: f32) -> Self {
        Self::all(value)
    }
}

impl From<[f32; 4]> for EdgeInsets {
    fn from([top, leading, bottom, trailing]: [f32; 4]) -> Self {
        Self::new(top, leading, bottom, trailing)
    }
}

/// Insets a child node by explicit edge distances or platform default padding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Padding {
    /// Explicit edge insets; `None` uses the host platform's default padding.
    #[serde(skip_serializing_if = "Option::is_none")]
    insets: Option<EdgeInsets>,
    /// The padded child node.
    content: Box<Node>,
}

impl Padding {
    /// Wraps `content` in the host platform's default padding.
    pub fn default_insets(content: impl WidgetContent) -> Self {
        Self {
            insets: None,
            content: Box::new(content.into_node()),
        }
    }

    /// Wraps `content` in explicit `insets`.
    pub fn new(insets: impl Into<EdgeInsets>, content: impl WidgetContent) -> Self {
        Self {
            insets: Some(insets.into()),
            content: Box::new(content.into_node()),
        }
    }

    /// Explicit edge insets, or `None` for the platform default.
    #[must_use]
    pub const fn insets(&self) -> Option<EdgeInsets> {
        self.insets
    }

    /// The padded child node.
    #[must_use]
    pub fn content(&self) -> &Node {
        &self.content
    }
}

/// Flexible empty space expanding along the enclosing stack's main axis.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Spacer {
    /// Minimum length in points/dp, when set.
    #[serde(skip_serializing_if = "Option::is_none")]
    min_length: Option<f32>,
}

impl Spacer {
    /// A spacer with no minimum length.
    #[must_use]
    pub const fn new() -> Self {
        Self { min_length: None }
    }

    /// A spacer with at least `min_length` points/dp along the stack axis.
    #[must_use]
    pub const fn with_min_length(min_length: f32) -> Self {
        Self {
            min_length: Some(min_length),
        }
    }

    /// Minimum length in points/dp, when set.
    #[must_use]
    pub const fn min_length(&self) -> Option<f32> {
        self.min_length
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new()
    }
}

/// An upper-bound length on a [`Frame`], serializable in JSON even when
/// unbounded (`f32::INFINITY`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum MaxLength {
    /// A finite maximum extent in points/dp.
    Fixed {
        /// Maximum extent in points/dp.
        value: f32,
    },
    /// Unbounded expansion (`CGFloat.infinity` / `MATCH_PARENT`).
    Infinity,
}

impl From<f32> for MaxLength {
    fn from(value: f32) -> Self {
        if value.is_infinite() && value.is_sign_positive() {
            Self::Infinity
        } else {
            Self::Fixed { value }
        }
    }
}

/// Size and alignment constraints around a child node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Frame {
    /// Exact width in points/dp.
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<f32>,
    /// Exact height in points/dp.
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<f32>,
    /// Minimum width in points/dp.
    #[serde(skip_serializing_if = "Option::is_none")]
    min_width: Option<f32>,
    /// Maximum width in points/dp, or unbounded expansion.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_width: Option<MaxLength>,
    /// Minimum height in points/dp.
    #[serde(skip_serializing_if = "Option::is_none")]
    min_height: Option<f32>,
    /// Maximum height in points/dp, or unbounded expansion.
    #[serde(skip_serializing_if = "Option::is_none")]
    max_height: Option<MaxLength>,
    /// Alignment of `content` inside the frame.
    #[serde(skip_serializing_if = "Option::is_none")]
    alignment: Option<Alignment>,
    /// The framed child node.
    content: Box<Node>,
}

impl Frame {
    /// Wraps `content` in an unconstrained frame ready for constraint methods.
    pub fn new(content: impl WidgetContent) -> Self {
        Self {
            width: None,
            height: None,
            min_width: None,
            max_width: None,
            min_height: None,
            max_height: None,
            alignment: None,
            content: Box::new(content.into_node()),
        }
    }

    /// Fixes the frame's width.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// Fixes the frame's height.
    #[must_use]
    pub const fn height(mut self, height: f32) -> Self {
        self.height = Some(height);
        self
    }

    /// Fixes both width and height.
    #[must_use]
    pub const fn size(mut self, width: f32, height: f32) -> Self {
        self.width = Some(width);
        self.height = Some(height);
        self
    }

    /// Sets the minimum width.
    #[must_use]
    pub const fn min_width(mut self, width: f32) -> Self {
        self.min_width = Some(width);
        self
    }

    /// Sets the maximum width (`f32::INFINITY` expands to fill available space).
    #[must_use]
    pub fn max_width(mut self, width: impl Into<MaxLength>) -> Self {
        self.max_width = Some(width.into());
        self
    }

    /// Sets the minimum height.
    #[must_use]
    pub const fn min_height(mut self, height: f32) -> Self {
        self.min_height = Some(height);
        self
    }

    /// Sets the maximum height (`f32::INFINITY` expands to fill available space).
    #[must_use]
    pub fn max_height(mut self, height: impl Into<MaxLength>) -> Self {
        self.max_height = Some(height.into());
        self
    }

    /// Sets both minimum width and minimum height.
    #[must_use]
    pub const fn min_size(mut self, width: f32, height: f32) -> Self {
        self.min_width = Some(width);
        self.min_height = Some(height);
        self
    }

    /// Sets both maximum width and maximum height.
    #[must_use]
    pub fn max_size(mut self, width: impl Into<MaxLength>, height: impl Into<MaxLength>) -> Self {
        self.max_width = Some(width.into());
        self.max_height = Some(height.into());
        self
    }

    /// Sets the child alignment inside the frame.
    #[must_use]
    pub fn alignment(mut self, alignment: impl Into<Alignment>) -> Self {
        self.alignment = Some(alignment.into());
        self
    }

    /// Exact width in points/dp.
    #[must_use]
    pub const fn get_width(&self) -> Option<f32> {
        self.width
    }

    /// Exact height in points/dp.
    #[must_use]
    pub const fn get_height(&self) -> Option<f32> {
        self.height
    }

    /// Minimum width in points/dp.
    #[must_use]
    pub const fn get_min_width(&self) -> Option<f32> {
        self.min_width
    }

    /// Maximum width in points/dp, or unbounded expansion.
    #[must_use]
    pub const fn get_max_width(&self) -> Option<MaxLength> {
        self.max_width
    }

    /// Minimum height in points/dp.
    #[must_use]
    pub const fn get_min_height(&self) -> Option<f32> {
        self.min_height
    }

    /// Maximum height in points/dp, or unbounded expansion.
    #[must_use]
    pub const fn get_max_height(&self) -> Option<MaxLength> {
        self.max_height
    }

    /// Alignment of `content` inside the frame.
    #[must_use]
    pub const fn get_alignment(&self) -> Option<Alignment> {
        self.alignment
    }

    /// The framed child node.
    #[must_use]
    pub fn content(&self) -> &Node {
        &self.content
    }
}

/// Paints `background` behind `content`.
///
/// When placed at the root of a widget timeline entry, the Apple `WidgetKit`
/// host lifts `background` into `.containerBackground(for: .widget)` so the
/// system bleeds it to the widget tile edge and adapts it to `StandBy` and
/// tinted modes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Background {
    /// The foreground content node.
    content: Box<Node>,
    /// The background node placed behind `content`.
    background: Box<Node>,
}

impl Background {
    /// Layers `background` behind `content`.
    pub fn new(content: impl WidgetContent, background: impl WidgetContent) -> Self {
        Self {
            content: Box::new(content.into_node()),
            background: Box::new(background.into_node()),
        }
    }

    /// The foreground content node.
    #[must_use]
    pub fn content(&self) -> &Node {
        &self.content
    }

    /// The background node placed behind `content`.
    #[must_use]
    pub fn background(&self) -> &Node {
        &self.background
    }
}

/// Attaches a screen-reader accessibility label to a child node.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct AccessibilityLabel {
    /// The label announced by assistive technologies.
    label: LabelText,
    /// The labeled child node.
    content: Box<Node>,
}

impl AccessibilityLabel {
    /// Wraps `content` with the accessibility `label`.
    pub fn new(label: impl IntoLabelText, content: impl WidgetContent) -> Self {
        Self {
            label: label.into_label_text(),
            content: Box::new(content.into_node()),
        }
    }

    /// The label announced by assistive technologies.
    #[must_use]
    pub const fn label(&self) -> &LabelText {
        &self.label
    }

    /// The labeled child node.
    #[must_use]
    pub fn content(&self) -> &Node {
        &self.content
    }
}

/// A shape filled with a color.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct Shape {
    /// The shape's geometry.
    kind: ShapeKind,
    /// What fills it.
    fill: Color,
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
    url: String,
    /// The rendered label.
    content: Box<Node>,
}

impl Link {
    /// A tappable region that opens `url` — a deep link into the app or a
    /// web URL.
    pub fn new(url: impl Into<String>, content: impl WidgetContent) -> Self {
        Self {
            url: url.into(),
            content: Box::new(content.into_node()),
        }
    }

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
/// and calls `perform` with the widget's shared [`WidgetStore`] so the
/// action can mutate state that the next timeline reads.
pub trait WidgetIntent: Serialize + DeserializeOwned + 'static {
    /// The action's effect — run in the extension's process with access to
    /// the shared widget store.
    fn perform(self, store: &WidgetStore);
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
    date: Date,
    /// The snapshot rendered for this entry.
    content: Node,
}

impl TimelineEntry {
    /// A snapshot that becomes current at `date`.
    pub fn new(date: impl Into<Date>, content: impl WidgetContent) -> Self {
        Self {
            date: date.into(),
            content: content.into_node(),
        }
    }

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
    entries: Vec<TimelineEntry>,
    /// When `WidgetKit` should ask for a new timeline.
    policy: ReloadPolicy,
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

    /// The dated entries, in chronological order.
    #[must_use]
    pub const fn entries(&self) -> &[TimelineEntry] {
        self.entries.as_slice()
    }

    /// When `WidgetKit` should ask for the next timeline.
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

/// The context handed to a widget timeline producer.
///
/// Carries the widget family being rendered, the proposed display size, and
/// the shared [`WidgetStore`] container provisioned for the app and its
/// widgets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct WidgetContext {
    /// The widget family being rendered.
    family: WidgetFamily,
    /// The size the system proposes for the widget's content.
    display_size: Size,
    /// The shared widget store backing cross-process state with the app.
    #[serde(rename = "storePath")]
    shared_store: WidgetStore,
}

impl WidgetContext {
    /// Creates a widget context for tests and host bridges.
    #[must_use]
    pub const fn new(family: WidgetFamily, display_size: Size, shared_store: WidgetStore) -> Self {
        Self {
            family,
            display_size,
            shared_store,
        }
    }

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

    /// The shared key-value store provisioned for the application and its
    /// widget extension.
    #[must_use]
    pub const fn shared_store(&self) -> &WidgetStore {
        &self.shared_store
    }
}
