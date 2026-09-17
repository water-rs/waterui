//! The value shapes an attribute accepts, and what each becomes in Rust.
//!
//! Every type here is three things at once: the [`TypeSchema`] the catalog
//! declares, the [`FromJs`] conversion that reads the value a JSX attribute
//! carried, and the conversion into the `WaterUI` value the constructor takes.
//! Keeping them together is what makes the catalog a contract rather than a
//! description — a shape the schema promises and the conversion refuses could
//! not survive the tests below the module.
//!
//! Most of them derive their schema. The exceptions are the genuine unions —
//! `padding` takes `true`, a number or an edge-insets object — which no Rust
//! type projects to, because a Rust field has one type. Those write their own
//! [`TsType`] impl directly beside the conversion that reads the same shapes.

use nami::{Computed, SignalExt as _};
use suiteki::Str;
use waterui_core::AnyView;
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_graphics::ResolvedColor;
use waterui_graphics::color::Color;
use waterui_layout::padding::EdgeInsets;
use waterui_locale::{Locale, TranslationCatalog};
use waterui_text::text::{IntoText, Text, TextConfig};
use waterui_ts::engine::{JsError, JsFunction, JsValue};
use waterui_ts::schema::{NumberKind, TsType, TypeSchema};
use waterui_url::Url;

use waterui_ts::Bridge;
use waterui_ts::{FromJs, IntoJs, expected};

/// Text as JSX wrote it: a string, or a number lifted into one.
///
/// A string participates in the same [`TranslationCatalog`] lookup as Rust's
/// `text("…")`, so a TypeScript view is localized exactly like a Rust view and
/// one catalog serves both. A number is the value itself — `<Text>{count()}</Text>`
/// is `text!("{count}")`, a formatted value rather than a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextContent {
    /// A string, looked up in the translation catalog for the current locale.
    /// A key the catalog does not carry reads as the key itself, which is what
    /// `Text::localized` does.
    Key(Str),
    /// Text that is already the value it displays, and never looked up.
    Verbatim(Str),
}

impl TextContent {
    /// The text this value carries, before any lookup.
    #[must_use]
    pub const fn as_str(&self) -> &Str {
        match self {
            Self::Key(text) | Self::Verbatim(text) => text,
        }
    }
}

impl TsType for TextContent {
    const SCHEMA: TypeSchema =
        TypeSchema::Union(&[TypeSchema::String, TypeSchema::Number(NumberKind::F64)]);
}

impl FromJs for TextContent {
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::String(text) => Ok(Self::Key(Str::from(text.clone()))),
            JsValue::Number(number) => Ok(Self::Verbatim(Str::from(number.to_string()))),
            JsValue::BigInt(_) => value
                .as_i64()
                .map(|value| Self::Verbatim(Str::from(value.to_string())))
                .ok_or_else(|| expected("text or a number", value)),
            other => Err(expected("text or a number", other)),
        }
    }
}

impl IntoJs for TextContent {
    /// The projection is `string`, so a value travelling back out is the text
    /// itself — a key is what JavaScript wrote, and a number was already
    /// lifted into its digits when it arrived.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::String(self.as_str().as_str().to_owned()))
    }
}

impl IntoText for TextContent {
    fn into_text(self) -> Text {
        match self {
            Self::Key(key) => Text::localized_with(move |env, locale| translate(env, locale, &key)),
            Self::Verbatim(text) => Text::verbatim(text),
        }
    }
}

/// One catalog lookup, with the key as its own fallback.
///
/// `Text::localized` takes a `&'static str` because a Rust literal is one. A
/// string that arrived from JavaScript is owned, so the lookup is spelled out
/// here over the same catalog and the same locale.
fn translate(env: &waterui_core::Environment, locale: &Locale, key: &Str) -> TextConfig {
    let translated = env
        .get::<TranslationCatalog>()
        .and_then(|catalog| catalog.lookup_text(locale, key.as_str()))
        .unwrap_or_else(|| key.clone());
    TextConfig::new(waterui_text::StyledStr::plain(translated))
}

/// A colour as `useTheme()` hands one out: five channels in linear light.
///
/// A theme token crossing into TypeScript and back is the same value, which is
/// what makes `<VStack background={theme().surface}>` the ordinary way to
/// paint with the theme.
#[derive(Debug, Clone, Copy, PartialEq, TsType)]
pub struct ColorValue {
    /// Red, in linear light. Outside 0–1 for a wide-gamut colour.
    pub red: f32,
    /// Green, in linear light.
    pub green: f32,
    /// Blue, in linear light.
    pub blue: f32,
    /// The extended-range headroom; `1` for an ordinary colour.
    pub headroom: Option<f32>,
    /// Alpha; `1` when the colour is opaque.
    pub opacity: Option<f32>,
}

impl From<ColorValue> for ResolvedColor {
    fn from(value: ColorValue) -> Self {
        Self {
            red: value.red,
            green: value.green,
            blue: value.blue,
            headroom: value.headroom.unwrap_or(1.0),
            opacity: value.opacity.unwrap_or(1.0),
        }
    }
}

impl From<ResolvedColor> for ColorValue {
    fn from(color: ResolvedColor) -> Self {
        Self {
            red: color.red,
            green: color.green,
            blue: color.blue,
            headroom: Some(color.headroom),
            opacity: Some(color.opacity),
        }
    }
}

/// The reactive colour a colour-valued attribute becomes.
#[must_use]
pub fn color_of(value: &Computed<ColorValue>) -> Color {
    waterui_graphics::color::signal_color(
        value
            .map(|value| Color::new(ResolvedColor::from(value)))
            .computed(),
    )
}

/// The insets an `EdgeInsets`-shaped object carries.
///
/// Every edge is optional and the two axes are shorthands, mirroring the Rust
/// `EdgeInsets` constructors: an edge named twice takes the more specific
/// value, so `{ horizontal: 8, leading: 16 }` insets the leading edge by 16.
///
/// Every position is a reactive input of its own, so `{ top: gap }` follows
/// `gap` without the object being rebuilt — the same thing a Rust author
/// composes by hand when one edge of a `Computed<EdgeInsets>` moves.
#[derive(Debug, Clone, TsType)]
pub struct EdgeValues {
    /// The top inset.
    pub top: Option<Computed<f32>>,
    /// The bottom inset.
    pub bottom: Option<Computed<f32>>,
    /// The leading inset.
    pub leading: Option<Computed<f32>>,
    /// The trailing inset.
    pub trailing: Option<Computed<f32>>,
    /// Both the leading and the trailing inset.
    pub horizontal: Option<Computed<f32>>,
    /// Both the top and the bottom inset.
    pub vertical: Option<Computed<f32>>,
}

impl EdgeValues {
    /// The insets these positions compose to, following every one of them.
    fn insets(&self) -> Computed<EdgeInsets> {
        let edge = |specific: &Option<Computed<f32>>, axis: &Option<Computed<f32>>| {
            specific
                .clone()
                .or_else(|| axis.clone())
                .unwrap_or_else(|| Computed::constant(0.0))
        };
        let top = edge(&self.top, &self.vertical);
        let bottom = edge(&self.bottom, &self.vertical);
        let leading = edge(&self.leading, &self.horizontal);
        let trailing = edge(&self.trailing, &self.horizontal);
        top.zip(&bottom)
            .zip(&leading)
            .zip(&trailing)
            .map(|(((top, bottom), leading), trailing)| {
                EdgeInsets::new(top, bottom, leading, trailing)
            })
            .computed()
    }
}

/// What `padding` accepts: `true`, a number, or an object per edge.
///
/// The spellings are the Rust ones — `.padding()`, `.padding_with(16.0)`,
/// `.padding_with(EdgeInsets…)` — and no Rust type is their union, so the
/// schema is written here beside the conversion. Every numeric position is a
/// reactive input, whether it is the one number or one edge of the object.
#[derive(Debug, Clone)]
pub enum PaddingValue {
    /// `padding` or `padding={true}`: the framework's default inset.
    Default,
    /// `padding={false}`: no inset at all, which is how a computed padding
    /// switches itself off.
    None,
    /// `padding={12}`: the same inset on every edge.
    Uniform(Computed<f32>),
    /// `padding={{ horizontal: 16, top: gap }}`: insets per edge.
    Edges(EdgeValues),
}

impl PaddingValue {
    /// The reactive insets this value composes to.
    pub fn insets(&self) -> Computed<EdgeInsets> {
        match self {
            Self::Default => Computed::constant(EdgeInsets::all(crate::view::DEFAULT_PADDING)),
            Self::None => Computed::constant(EdgeInsets::all(0.0)),
            Self::Uniform(inset) => inset.map(EdgeInsets::all).computed(),
            Self::Edges(edges) => edges.insets(),
        }
    }
}

impl TsType for PaddingValue {
    const SCHEMA: TypeSchema = TypeSchema::Union(&[
        TypeSchema::Bool,
        <Computed<f32> as TsType>::SCHEMA,
        EdgeValues::SCHEMA,
    ]);
}

impl FromJs for PaddingValue {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Bool(true) => Ok(Self::Default),
            JsValue::Bool(false) => Ok(Self::None),
            JsValue::Object(_) => EdgeValues::from_js(value, bridge).map(Self::Edges),
            // A number, or any reactive input of one: `padding={12}` and
            // `padding={gap}` are one inset on every edge.
            other => Computed::<f32>::from_js(other, bridge).map(Self::Uniform),
        }
    }
}

impl IntoJs for PaddingValue {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        match self {
            Self::Default => Ok(JsValue::Bool(true)),
            Self::None => Ok(JsValue::Bool(false)),
            Self::Uniform(inset) => inset.into_js(bridge),
            Self::Edges(edges) => edges.into_js(bridge),
        }
    }
}

/// What `background` paints behind a view.
///
/// Rust's `background` takes anything that is a view, and a colour is one, so
/// the modifier here takes the same two shapes: a colour object, reactive like
/// any other attribute value, or a view built elsewhere in the same module.
/// The distinction is made before materializing, because a view crosses once
/// and a colour is a value that can keep changing.
#[derive(Debug)]
pub enum BackgroundValue {
    /// `background={{ red: 1, green: 0, blue: 0 }}`: a colour, which may be a
    /// signal.
    Color(Computed<ColorValue>),
    /// `background={<Gradient />}`: a view painted behind the content.
    View(AnyView),
}

impl TsType for BackgroundValue {
    const SCHEMA: TypeSchema = TypeSchema::Union(&[
        <Computed<ColorValue> as TsType>::SCHEMA,
        <AnyView as TsType>::SCHEMA,
    ]);
}

impl FromJs for BackgroundValue {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Opaque(_) => AnyView::from_js(value, bridge).map(Self::View),
            _ => Computed::<ColorValue>::from_js(value, bridge).map(Self::Color),
        }
    }
}

impl IntoJs for BackgroundValue {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        match self {
            Self::Color(color) => color.into_js(bridge),
            Self::View(view) => view.into_js(bridge),
        }
    }
}

/// The shadow `shadow` casts, which is the Rust [`Shadow`] spelled as an
/// object.
///
/// Only the colour is reactive, because only the colour is reactive in Rust:
/// `Shadow`'s offset, blur radius and corner radius are `f32` fields, and a
/// signal declared where the framework takes a number would promise an update
/// nothing delivers.
///
/// [`Shadow`]: crate::appearance::style::Shadow
#[derive(Debug, Clone, PartialEq, TsType)]
pub struct ShadowValue {
    /// The shadow's colour; black when it is left out, as `Shadow::splat` uses.
    pub color: Option<ColorValue>,
    /// The horizontal offset, in points.
    pub x: Option<f32>,
    /// The vertical offset, in points.
    pub y: Option<f32>,
    /// The blur radius, in points.
    pub radius: f32,
    /// The corner radius of the view casting the shadow, so the silhouette
    /// follows its shape.
    #[ts(rename = "cornerRadius")]
    pub corner_radius: Option<f32>,
}

/// The border `border` draws, which is the Rust [`Border`] spelled as an
/// object.
///
/// [`Border`]: crate::appearance::border::Border
#[derive(Debug, Clone, PartialEq, TsType)]
pub struct BorderValue {
    /// The border's colour.
    pub color: ColorValue,
    /// The stroke width, in points.
    pub width: f32,
    /// The corner radius, in points; square corners when left out.
    #[ts(rename = "cornerRadius")]
    pub corner_radius: Option<f32>,
}

/// A shape named by a string, for the shapes that carry no parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TsType)]
pub enum ShapeName {
    /// The view's own rectangle.
    Rectangle,
    /// A circle inscribed in the view's shorter side.
    Circle,
    /// A rectangle whose ends are fully rounded, at whatever size the view is.
    Capsule,
    /// An ellipse filling the view's bounds.
    Ellipse,
}

/// A rectangle with a uniform corner radius.
#[derive(Debug, Clone, Copy, PartialEq, TsType)]
pub struct RoundedShape {
    /// The corner radius, as a fraction of the shorter side: `0.5` is fully
    /// rounded, which is what [`ShapeName::Capsule`] says directly.
    #[ts(rename = "cornerRadius")]
    pub corner_radius: f32,
}

/// What `clip` accepts: a named shape, or a rounded rectangle's radius.
///
/// Rust's `clip` takes anything implementing `Shape`, which is an open set a
/// TypeScript type cannot name. What it can name is the shapes the framework
/// ships, so the attribute is a union of those — a string for the ones that
/// carry nothing, an object for the one that carries a radius — and a shape an
/// application defines is composed on the Rust side and slotted in as a view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeValue {
    /// `clip="Rectangle"`.
    Rectangle,
    /// `clip="Circle"`.
    Circle,
    /// `clip="Capsule"`.
    Capsule,
    /// `clip="Ellipse"`.
    Ellipse,
    /// `clip={{ cornerRadius: 0.25 }}`.
    Rounded(f32),
}

impl TsType for ShapeValue {
    const SCHEMA: TypeSchema = TypeSchema::Union(&[ShapeName::SCHEMA, RoundedShape::SCHEMA]);
}

impl FromJs for ShapeValue {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::String(_) => Ok(match ShapeName::from_js(value, bridge)? {
                ShapeName::Rectangle => Self::Rectangle,
                ShapeName::Circle => Self::Circle,
                ShapeName::Capsule => Self::Capsule,
                ShapeName::Ellipse => Self::Ellipse,
            }),
            JsValue::Object(_) => {
                RoundedShape::from_js(value, bridge).map(|shape| Self::Rounded(shape.corner_radius))
            }
            other => Err(expected(
                "a shape name, or an object carrying a corner radius",
                other,
            )),
        }
    }
}

impl IntoJs for ShapeValue {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        match self {
            Self::Rectangle => ShapeName::Rectangle.into_js(bridge),
            Self::Circle => ShapeName::Circle.into_js(bridge),
            Self::Capsule => ShapeName::Capsule.into_js(bridge),
            Self::Ellipse => ShapeName::Ellipse.into_js(bridge),
            Self::Rounded(corner_radius) => RoundedShape { corner_radius }.into_js(bridge),
        }
    }
}

/// A URL, which crosses as the string it is written as.
///
/// A newtype rather than `Url` itself, because both the conversion traits and
/// `Url` come from other crates. Parsing happens on the way in, so an address
/// a `WaterUI` view could not fetch is a typed error naming it rather than a
/// picture that silently never appears.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlValue(pub Url);

impl TsType for UrlValue {
    const SCHEMA: TypeSchema = TypeSchema::String;
}

impl FromJs for UrlValue {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        let text = Str::from_js(value, bridge)?;
        Url::parse(&text)
            .map(Self)
            .ok_or_else(|| JsError::conversion(format!("`{text}` is not a URL WaterUI can fetch")))
    }
}

impl IntoJs for UrlValue {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        Str::from(self.0.to_string()).into_js(bridge)
    }
}

/// Which shape a `<Progress>` takes, mirroring Rust's `ProgressStyle`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, TsType)]
pub enum ProgressStyleValue {
    /// A bar, which is what a determinate progress uses by default.
    #[default]
    Linear,
    /// A ring.
    Circular,
    /// An indeterminate spinner.
    Loading,
}

/// Where a vertical stack's children sit on the horizontal axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, TsType)]
pub enum HorizontalAlign {
    /// The leading edge, which follows the layout direction.
    Leading,
    /// The centre, which is what a bare `<VStack>` uses.
    #[default]
    Center,
    /// The trailing edge.
    Trailing,
}

impl From<HorizontalAlign> for HorizontalAlignment {
    fn from(value: HorizontalAlign) -> Self {
        match value {
            HorizontalAlign::Leading => Self::Leading,
            HorizontalAlign::Center => Self::Center,
            HorizontalAlign::Trailing => Self::Trailing,
        }
    }
}

/// Where a horizontal stack's children sit on the vertical axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, TsType)]
pub enum VerticalAlign {
    /// The top edge.
    Top,
    /// The centre, which is what a bare `<HStack>` uses.
    #[default]
    Center,
    /// The bottom edge.
    Bottom,
    /// The first text baseline.
    FirstBaseline,
    /// The last text baseline.
    LastBaseline,
}

impl From<VerticalAlign> for VerticalAlignment {
    fn from(value: VerticalAlign) -> Self {
        match value {
            VerticalAlign::Top => Self::Top,
            VerticalAlign::Center => Self::Center,
            VerticalAlign::Bottom => Self::Bottom,
            VerticalAlign::FirstBaseline => Self::FirstBaseline,
            VerticalAlign::LastBaseline => Self::LastBaseline,
        }
    }
}

/// Where a depth stack's children sit in both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, TsType)]
pub enum StackAlign {
    /// The top leading corner.
    TopLeading,
    /// The top edge, centred horizontally.
    Top,
    /// The top trailing corner.
    TopTrailing,
    /// The leading edge, centred vertically.
    Leading,
    /// Both centres, which is what a bare `<ZStack>` uses.
    #[default]
    Center,
    /// The trailing edge, centred vertically.
    Trailing,
    /// The bottom leading corner.
    BottomLeading,
    /// The bottom edge, centred horizontally.
    Bottom,
    /// The bottom trailing corner.
    BottomTrailing,
}

impl From<StackAlign> for Alignment {
    fn from(value: StackAlign) -> Self {
        match value {
            StackAlign::TopLeading => Self::TopLeading,
            StackAlign::Top => Self::Top,
            StackAlign::TopTrailing => Self::TopTrailing,
            StackAlign::Leading => Self::Leading,
            StackAlign::Center => Self::Center,
            StackAlign::Trailing => Self::Trailing,
            StackAlign::BottomLeading => Self::BottomLeading,
            StackAlign::Bottom => Self::Bottom,
            StackAlign::BottomTrailing => Self::BottomTrailing,
        }
    }
}

/// Which way a scroll view scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, TsType)]
pub enum ScrollAxis {
    /// Vertically, which is what a bare `<ScrollView>` does.
    #[default]
    Vertical,
    /// Horizontally.
    Horizontal,
    /// Both axes.
    Both,
}

/// A JavaScript function the host calls: an event handler, never a value that
/// is read or subscribed to.
///
/// The bridge is captured with the function, because calling back into
/// JavaScript needs the engine. Dropping the runtime while a view still holds
/// one is not a crash: the call reports that the runtime is gone.
#[derive(Clone)]
pub struct JsAction {
    function: JsFunction,
    bridge: waterui_ts::WeakBridge,
}

impl core::fmt::Debug for JsAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsAction").finish_non_exhaustive()
    }
}

impl JsAction {
    /// Calls the handler, logging a JavaScript exception rather than
    /// unwinding into the backend that delivered the event.
    pub fn call(&self) {
        let Some(bridge) = self.bridge.upgrade() else {
            tracing::warn!(
                "a TypeScript event handler fired after its runtime was dropped, and did nothing"
            );
            return;
        };
        if let Err(error) = bridge.call(&self.function, &[]) {
            tracing::error!(%error, "a TypeScript event handler threw");
        }
    }
}

impl TsType for JsAction {
    const SCHEMA: TypeSchema = TypeSchema::Callback(&[]);
}

impl FromJs for JsAction {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Function(function) => Ok(Self {
                function: function.clone(),
                bridge: bridge.downgrade(),
            }),
            other => Err(expected("a function", other)),
        }
    }
}

impl IntoJs for JsAction {
    /// The function itself: a handler that came from JavaScript goes back as
    /// what it was.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Function(self.function))
    }
}

/// One option of a `<Picker>`: the value it selects and the text it shows.
#[derive(Debug, Clone, PartialEq, Eq, TsType)]
pub struct PickerOption {
    /// The value written into the picker's `value` signal when this option is
    /// chosen.
    pub value: Str,
    /// The option's label, localized like any other text.
    pub label: TextContent,
}
