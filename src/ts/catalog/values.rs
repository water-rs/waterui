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

use core::marker::PhantomData;
use std::collections::BTreeMap;

use nami::{Computed, Signal as _, SignalExt as _};
use suiteki::Str;
use waterui_core::layout::{Alignment, HorizontalAlignment, VerticalAlignment};
use waterui_core::{AnyView, Environment, flatten_signal};
use waterui_graphics::WorkingColor;
use waterui_graphics::color::{Color, Srgb, Working, working};
use waterui_layout::padding::EdgeInsets;
use waterui_locale::{Locale, TranslationCatalog};
use waterui_text::StyledStr;
use waterui_text::text::{IntoText, Text, TextConfig};
use waterui_ts::engine::{JsError, JsFunction, JsValue};
use waterui_ts::schema::{FieldSchema, NumberKind, StructSchema, TsType, TypeSchema};
use waterui_url::Url;

use waterui_ts::Bridge;
use waterui_ts::{FromJs, IntoJs, expected};

/// Named arguments carried by a localized [`TextContent`], folded into one
/// reactive map so updates to any signal argument re-interpolate in place.
#[derive(Debug, Clone)]
pub struct LocalizedArgs(Computed<BTreeMap<Str, Str>>);

impl LocalizedArgs {
    fn empty() -> Self {
        Self(Computed::constant(BTreeMap::new()))
    }

    fn snapshot(&self) -> BTreeMap<Str, Str> {
        self.0.snapshot()
    }
}

impl PartialEq for LocalizedArgs {
    fn eq(&self, other: &Self) -> bool {
        self.0.snapshot() == other.0.snapshot()
    }
}

impl Eq for LocalizedArgs {}

/// One interpolation argument value accepted inside `{ key, args }`.
#[derive(Debug, Clone)]
struct TextArg(Str);

impl IntoJs for TextArg {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::String(self.0.as_str().to_owned()))
    }
}

impl FromJs for TextArg {
    fn from_js(value: &JsValue, _bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::String(text) => Ok(Self(Str::from(text.clone()))),
            JsValue::Number(number) => Ok(Self(Str::from(number.to_string()))),
            JsValue::BigInt(_) => value
                .as_i64()
                .map(|integer| Self(Str::from(integer.to_string())))
                .or_else(|| {
                    value
                        .as_u64()
                        .map(|integer| Self(Str::from(integer.to_string())))
                })
                .ok_or_else(|| expected("a string or number argument", value)),
            other => Err(expected("a string or number argument", other)),
        }
    }
}

/// The schema of `{ key: string, args?: Record<string, string | number> }`.
const LOCALIZED_TEXT_SCHEMA: TypeSchema = TypeSchema::Struct(StructSchema::new(
    "LocalizedText",
    &[
        FieldSchema::new("key", TypeSchema::String),
        FieldSchema::new(
            "args",
            TypeSchema::Option(&TypeSchema::Map {
                key: &TypeSchema::String,
                value: &TypeSchema::Union(&[
                    TypeSchema::String,
                    TypeSchema::Number(NumberKind::F64),
                ]),
            }),
        ),
    ],
));

/// A resolved snapshot of one [`TextContent`] part, ready for locale lookup
/// and placeholder interpolation.
#[derive(Debug, Clone)]
pub enum ResolvedTextPart {
    Localized { key: Str, args: BTreeMap<Str, Str> },
    Verbatim(Str),
}

impl ResolvedTextPart {
    pub(crate) fn render(&self, env: &Environment, locale: &Locale) -> Str {
        match self {
            Self::Localized { key, args } => lookup_and_interpolate(env, locale, key, args),
            Self::Verbatim(text) => text.clone(),
        }
    }
}

impl IntoText for ResolvedTextPart {
    fn into_text(self) -> Text {
        match self {
            Self::Localized { key, args } => Text::localized_with(move |env, locale| {
                TextConfig::new(StyledStr::plain(lookup_and_interpolate(
                    env, locale, &key, &args,
                )))
            }),
            Self::Verbatim(text) => Text::verbatim(text),
        }
    }
}

/// Text as JSX wrote it: a verbatim runtime string or number, or an explicit
/// localized catalog entry (`{ key }` / `{ key, args }`).
///
/// Runtime JavaScript strings and numbers are verbatim so user data never
/// reaches [`TranslationCatalog`]. Only literal JSX text (which the JSX
/// transform emits as `{ key }` or `{ key, args }`) and explicit `t` calls
/// perform catalog lookups.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TextContent {
    /// A translation key with optional named arguments, looked up in the
    /// translation catalog for the current locale. A key the catalog does not
    /// carry falls back to the key itself before placeholder interpolation.
    Localized {
        /// The lookup key (and fallback template when absent from the catalog).
        key: Str,
        /// Named arguments interpolated into `{name}` placeholders.
        args: LocalizedArgs,
    },
    /// Text that is already the value it displays, and never looked up.
    Verbatim(Str),
}

impl TextContent {
    /// Whether `value` is a localized `{ key, args? }` object rather than a
    /// reactive signal or other JS object.
    #[must_use]
    pub fn is_localized_object(value: &JsValue) -> bool {
        value
            .as_object()
            .is_some_and(|entries| entries.iter().any(|(name, _)| name == "key"))
    }

    /// The raw key or verbatim text this value carries, before catalog lookup
    /// or argument interpolation.
    #[must_use]
    pub const fn as_str(&self) -> &Str {
        match self {
            Self::Localized { key, .. } | Self::Verbatim(key) => key,
        }
    }

    pub(crate) fn into_resolved_signal(self) -> Computed<ResolvedTextPart> {
        match self {
            Self::Localized { key, args } => args
                .0
                .map(move |args| ResolvedTextPart::Localized {
                    key: key.clone(),
                    args,
                })
                .computed(),
            Self::Verbatim(text) => Computed::constant(ResolvedTextPart::Verbatim(text)),
        }
    }

    pub(crate) fn flatten_computed(source: &Computed<Self>) -> Computed<ResolvedTextPart> {
        flatten_signal(source.map(Self::into_resolved_signal))
    }
}

impl TsType for TextContent {
    const SCHEMA: TypeSchema = TypeSchema::Union(&[
        TypeSchema::String,
        TypeSchema::Number(NumberKind::F64),
        LOCALIZED_TEXT_SCHEMA,
    ]);
}

impl FromJs for TextContent {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::String(text) => Ok(Self::Verbatim(Str::from(text.clone()))),
            JsValue::Number(number) => Ok(Self::Verbatim(Str::from(number.to_string()))),
            JsValue::BigInt(_) => value
                .as_i64()
                .map(|value| Self::Verbatim(Str::from(value.to_string())))
                .or_else(|| {
                    value
                        .as_u64()
                        .map(|value| Self::Verbatim(Str::from(value.to_string())))
                })
                .ok_or_else(|| {
                    expected("text, a number, or a localized { key, args } object", value)
                }),
            JsValue::Object(entries) => parse_localized_object(entries, bridge),
            other => Err(expected(
                "text, a number, or a localized { key, args } object",
                other,
            )),
        }
    }
}

fn parse_localized_object(
    entries: &[(String, JsValue)],
    bridge: &Bridge,
) -> Result<TextContent, JsError> {
    let mut key: Option<Str> = None;
    let mut args_value: Option<&JsValue> = None;
    for (name, value) in entries {
        match name.as_str() {
            "key" => {
                let text = value
                    .as_str()
                    .ok_or_else(|| expected("a string for `key`", value))?;
                key = Some(Str::from(text.to_owned()));
            }
            "args" => {
                args_value = Some(value);
            }
            unknown => {
                return Err(JsError::conversion(format!(
                    "localized text does not recognise property `{unknown}`; expected `key` and optional `args`"
                )));
            }
        }
    }
    let Some(key) = key else {
        return Err(JsError::conversion(
            "localized text object is missing required string property `key`",
        ));
    };
    let args = match args_value {
        None | Some(JsValue::Undefined | JsValue::Null) => LocalizedArgs::empty(),
        Some(JsValue::Object(arg_entries)) => materialize_args(arg_entries, bridge)?,
        Some(other) => return Err(expected("an arguments object for `args`", other)),
    };
    Ok(TextContent::Localized { key, args })
}

fn materialize_args(
    entries: &[(String, JsValue)],
    bridge: &Bridge,
) -> Result<LocalizedArgs, JsError> {
    if entries.is_empty() {
        return Ok(LocalizedArgs::empty());
    }
    let mut combined: Computed<BTreeMap<Str, Str>> = Computed::constant(BTreeMap::new());
    for (name, value) in entries {
        let arg_name = Str::from(name.clone());
        let arg_signal: Computed<TextArg> = bridge.materialize_computed(value)?;
        combined = combined
            .zip(&arg_signal)
            .map(move |(mut map, value)| {
                map.insert(arg_name.clone(), value.0);
                map
            })
            .computed();
    }
    Ok(LocalizedArgs(combined))
}

impl IntoJs for TextContent {
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        match self {
            Self::Verbatim(text) => Ok(JsValue::String(text.as_str().to_owned())),
            Self::Localized { key, args } => {
                let mut entries = vec![(
                    String::from("key"),
                    JsValue::String(key.as_str().to_owned()),
                )];
                let snapshot = args.snapshot();
                if !snapshot.is_empty() {
                    let arg_entries = snapshot
                        .into_iter()
                        .map(|(name, value)| {
                            (
                                name.as_str().to_owned(),
                                JsValue::String(value.as_str().to_owned()),
                            )
                        })
                        .collect();
                    entries.push((String::from("args"), JsValue::Object(arg_entries)));
                }
                Ok(JsValue::Object(entries))
            }
        }
    }
}

impl IntoText for TextContent {
    fn into_text(self) -> Text {
        match self {
            Self::Verbatim(text) => Text::verbatim(text),
            Self::Localized { key, args } => Text::new(
                args.0
                    .map(move |args| ResolvedTextPart::Localized {
                        key: key.clone(),
                        args,
                    })
                    .computed(),
            ),
        }
    }
}

/// One catalog lookup, with the key as its own fallback, followed by named
/// placeholder interpolation.
fn lookup_and_interpolate(
    env: &Environment,
    locale: &Locale,
    key: &Str,
    args: &BTreeMap<Str, Str>,
) -> Str {
    let template = env
        .get::<TranslationCatalog>()
        .and_then(|catalog| catalog.lookup_text(locale, key.as_str()))
        .unwrap_or_else(|| key.clone());
    if args.is_empty() && !template.as_str().contains(['{', '}']) {
        return template;
    }
    interpolate(template.as_str(), args)
}

/// Replaces `{name}` placeholders from `args` and unescapes `{{` / `}}`.
///
/// A `{name}` not present in `args`, or an unclosed `{`, is kept verbatim.
fn interpolate(template: &str, args: &BTreeMap<Str, Str>) -> Str {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '{' => {
                if chars.peek() == Some(&'{') {
                    chars.next();
                    out.push('{');
                    continue;
                }
                let mut name = String::new();
                let mut closed = false;
                for next in chars.by_ref() {
                    if next == '}' {
                        closed = true;
                        break;
                    }
                    name.push(next);
                }
                if closed {
                    if let Some(value) = args.get(name.as_str()) {
                        out.push_str(value.as_str());
                    } else {
                        out.push('{');
                        out.push_str(&name);
                        out.push('}');
                    }
                } else {
                    out.push('{');
                    out.push_str(&name);
                }
            }
            '}' => {
                if chars.peek() == Some(&'}') {
                    chars.next();
                }
                out.push('}');
            }
            other => out.push(other),
        }
    }
    Str::from(out)
}

/// A linear-light colour object, as `useTheme()` hands out and wide-gamut
/// callers write explicitly.
#[derive(Debug, Clone, Copy, PartialEq, TsType)]
struct LinearColor {
    /// Red, in linear light. Outside 0–1 for a wide-gamut colour.
    red: f32,
    /// Green, in linear light.
    green: f32,
    /// Blue, in linear light.
    blue: f32,
    /// The extended-range headroom; `1` for an ordinary colour.
    headroom: Option<f32>,
    /// Alpha; `1` when the colour is opaque.
    opacity: Option<f32>,
}

/// A colour value crossing the JSX boundary: either an sRGB hex string
/// (`#RGB`, `#RGBA`, `#RRGGBB`, `#RRGGBBAA`) or a linear-light object
/// `{ red, green, blue, headroom?, opacity? }` such as a `useTheme()` token.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ColorValue {
    /// Red, in linear light. Outside 0–1 for a wide-gamut colour.
    red: f32,
    /// Green, in linear light.
    green: f32,
    /// Blue, in linear light.
    blue: f32,
    /// The extended-range headroom; `1` for an ordinary colour.
    headroom: Option<f32>,
    /// Alpha; `1` when the colour is opaque.
    opacity: Option<f32>,
}

impl ColorValue {
    const fn from_linear(color: LinearColor) -> Self {
        Self {
            red: color.red,
            green: color.green,
            blue: color.blue,
            headroom: color.headroom,
            opacity: color.opacity,
        }
    }

    const fn into_linear(self) -> LinearColor {
        LinearColor {
            red: self.red,
            green: self.green,
            blue: self.blue,
            headroom: self.headroom,
            opacity: self.opacity,
        }
    }

    /// Parses an sRGB hex string (`#RGB`, `#RGBA`, `#RRGGBB`, or `#RRGGBBAA`)
    /// into a linear working [`ColorValue`].
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when `hex` does not start with `#`, has an
    /// unsupported digit count, or contains a non-hexadecimal character.
    pub fn from_srgb_hex(hex: &str) -> Result<Self, JsError> {
        let Some(digits) = hex.strip_prefix('#') else {
            return Err(JsError::conversion(format!(
                "invalid hex colour `{hex}`: expected #RGB, #RGBA, #RRGGBB, or #RRGGBBAA"
            )));
        };
        let bytes = digits.as_bytes();
        let nibble = |byte: u8| -> Result<u8, JsError> {
            match byte {
                b'0'..=b'9' => Ok(byte - b'0'),
                b'a'..=b'f' => Ok(byte - b'a' + 10),
                b'A'..=b'F' => Ok(byte - b'A' + 10),
                _ => Err(JsError::conversion(format!(
                    "invalid hex colour `{hex}`: expected hexadecimal digits"
                ))),
            }
        };
        let pair = |hi: u8, lo: u8| -> Result<u8, JsError> { Ok((nibble(hi)? << 4) | nibble(lo)?) };
        let (r, g, b, a) = match *bytes {
            [r, g, b] => (pair(r, r)?, pair(g, g)?, pair(b, b)?, 0xFF),
            [r, g, b, a] => (pair(r, r)?, pair(g, g)?, pair(b, b)?, pair(a, a)?),
            [r1, r2, g1, g2, b1, b2] => (pair(r1, r2)?, pair(g1, g2)?, pair(b1, b2)?, 0xFF),
            [r1, r2, g1, g2, b1, b2, a1, a2] => {
                (pair(r1, r2)?, pair(g1, g2)?, pair(b1, b2)?, pair(a1, a2)?)
            }
            _ => {
                return Err(JsError::conversion(format!(
                    "invalid hex colour `{hex}`: expected #RGB, #RGBA, #RRGGBB, or #RRGGBBAA"
                )));
            }
        };
        let [red, green, blue, _] = Srgb::new_u8(r, g, b).resolve().components;
        let opacity = f32::from(a) / 255.0;
        Ok(Self::from(WorkingColor::new([red, green, blue, opacity])))
    }

    /// Red, in linear light. Outside 0–1 for a wide-gamut colour.
    #[must_use]
    pub const fn red(&self) -> f32 {
        self.red
    }

    /// Green, in linear light.
    #[must_use]
    pub const fn green(&self) -> f32 {
        self.green
    }

    /// Blue, in linear light.
    #[must_use]
    pub const fn blue(&self) -> f32 {
        self.blue
    }

    /// The extended-range headroom; `1` for an ordinary colour.
    #[must_use]
    pub const fn headroom(&self) -> Option<f32> {
        self.headroom
    }

    /// Alpha; `1` when the colour is opaque.
    #[must_use]
    pub const fn opacity(&self) -> Option<f32> {
        self.opacity
    }
}

impl TsType for ColorValue {
    const SCHEMA: TypeSchema = TypeSchema::Union(&[TypeSchema::String, LinearColor::SCHEMA]);
}

impl FromJs for ColorValue {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::String(hex) => Self::from_srgb_hex(hex),
            JsValue::Object(_) => LinearColor::from_js(value, bridge).map(Self::from_linear),
            other => Err(expected("a hex colour string or a colour object", other)),
        }
    }
}

impl IntoJs for ColorValue {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        self.into_linear().into_js(bridge)
    }
}

impl From<ColorValue> for WorkingColor {
    /// The channels are the working colour itself: a theme token that echoes
    /// them back stays the same colour. A `headroom` past one on channels
    /// still inside the SDR range is the exposure the author asked for; once
    /// the channels reach past one the range is already theirs and `headroom`
    /// is descriptive, never applied twice.
    fn from(value: ColorValue) -> Self {
        let [red, green, blue] = [value.red, value.green, value.blue];
        let color = Self::new([red, green, blue, value.opacity.unwrap_or(1.0)]);
        match value.headroom.unwrap_or(1.0) {
            headroom if headroom > 1.0 && red.max(green).max(blue) <= 1.0 => {
                working::with_headroom(color, headroom - 1.0)
            }
            _ => color,
        }
    }
}

impl From<WorkingColor> for ColorValue {
    fn from(color: WorkingColor) -> Self {
        let [red, green, blue, opacity] = color.components;
        Self {
            red,
            green,
            blue,
            headroom: Some(red.max(green).max(blue).max(1.0)),
            opacity: Some(opacity),
        }
    }
}

/// The reactive colour a colour-valued attribute becomes.
#[must_use]
pub fn color_of(value: &Computed<ColorValue>) -> Color {
    Color::from_signal(value.map(|value| Color::new(Working(WorkingColor::from(value)))))
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
                EdgeInsets::new(top, leading, bottom, trailing)
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
#[non_exhaustive]
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
#[non_exhaustive]
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
/// Only the colour is reactive here: `ShadowValue`'s offset, blur radius and
/// corner radius are `f32` schema values for TypeScript.
///
/// [`Shadow`]: crate::appearance::style::Shadow
#[derive(Debug, Clone, PartialEq, TsType)]
#[non_exhaustive]
pub struct ShadowValue {
    /// The shadow's colour; black when it is left out, as `Shadow::splat` uses.
    color: Option<ColorValue>,
    /// The horizontal offset, in points.
    x: Option<f32>,
    /// The vertical offset, in points.
    y: Option<f32>,
    /// The blur radius, in points.
    radius: f32,
    /// The corner radius of the view casting the shadow, so the silhouette
    /// follows its shape.
    corner_radius: Option<f32>,
}

impl ShadowValue {
    /// The shadow's colour; black when it is left out, as `Shadow::splat` uses.
    #[must_use]
    pub const fn color(&self) -> Option<ColorValue> {
        self.color
    }

    /// The horizontal offset, in points.
    #[must_use]
    pub const fn x(&self) -> Option<f32> {
        self.x
    }

    /// The vertical offset, in points.
    #[must_use]
    pub const fn y(&self) -> Option<f32> {
        self.y
    }

    /// The blur radius, in points.
    #[must_use]
    pub const fn radius(&self) -> f32 {
        self.radius
    }

    /// The corner radius of the view casting the shadow, so the silhouette
    /// follows its shape.
    #[must_use]
    pub const fn corner_radius(&self) -> Option<f32> {
        self.corner_radius
    }
}

/// The border `border` draws, which is the Rust [`Border`] spelled as an
/// object.
///
/// [`Border`]: crate::appearance::border::Border
#[derive(Debug, Clone, PartialEq, TsType)]
#[non_exhaustive]
pub struct BorderValue {
    /// The border's colour.
    color: ColorValue,
    /// The stroke width, in points.
    width: f32,
    /// The corner radius, in points; square corners when left out.
    corner_radius: Option<f32>,
}

impl BorderValue {
    /// The border's colour.
    #[must_use]
    pub const fn color(&self) -> ColorValue {
        self.color
    }

    /// The stroke width, in points.
    #[must_use]
    pub const fn width(&self) -> f32 {
        self.width
    }

    /// The corner radius, in points; square corners when left out.
    #[must_use]
    pub const fn corner_radius(&self) -> Option<f32> {
        self.corner_radius
    }
}

/// A shape named by a string, for the shapes that carry no parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, TsType)]
#[non_exhaustive]
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
#[non_exhaustive]
pub struct RoundedShape {
    /// The corner radius, as a fraction of the shorter side: `0.5` is fully
    /// rounded, which is what [`ShapeName::Capsule`] says directly.
    corner_radius: f32,
}

impl RoundedShape {
    /// The corner radius, as a fraction of the shorter side: `0.5` is fully
    /// rounded, which is what [`ShapeName::Capsule`] says directly.
    #[must_use]
    pub const fn corner_radius(&self) -> f32 {
        self.corner_radius
    }
}

/// What `clip` accepts: a named shape, or a rounded rectangle's radius.
///
/// Rust's `clip` takes anything implementing `Shape`, which is an open set a
/// TypeScript type cannot name. What it can name is the shapes the framework
/// ships, so the attribute is a union of those — a string for the ones that
/// carry nothing, an object for the one that carries a radius — and a shape an
/// application defines is composed on the Rust side and slotted in as a view.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum ShapeValue {
    /// `clip="rectangle"`.
    Rectangle,
    /// `clip="circle"`.
    Circle,
    /// `clip="capsule"`.
    Capsule,
    /// `clip="ellipse"`.
    Ellipse,
    /// `clip={{ cornerRadius: 12 }}`.
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
#[non_exhaustive]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "closed alignment set mirroring the Rust horizontal alignment"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "closed alignment set mirroring the Rust vertical alignment"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "closed alignment set mirroring the Rust two-axis alignment"
)]
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
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: a scroll view scrolls vertically, horizontally, or both"
)]
pub enum ScrollAxis {
    /// Vertically, which is what a bare `<ScrollView>` does.
    #[default]
    Vertical,
    /// Horizontally.
    Horizontal,
    /// Both axes.
    Both,
}

/// What a [`JsAction`] is called with: the event's values as a tuple, in
/// declaration order.
///
/// This is the outbound mirror of the inbound conversion in
/// `waterui-ts`'s `convert::callback`, which reads a Rust closure's arguments
/// off a JavaScript call with [`FromJs`] in the order the schema's `Callback`
/// node lists them. Here the same list is written rather than read: the
/// tuple's elements *are* that node's argument types, and the values cross
/// with [`IntoJs`] in the same order. One list drives both, so the signature
/// an author reads in the generated `.d.ts` and the call their handler
/// receives cannot come apart.
///
/// Eight is the cap, as it is for every other callable spelling the schema
/// carries.
pub trait ActionArguments {
    /// Each argument's type, in declaration order.
    const TYPES: &'static [TypeSchema];

    /// Converts each argument, in declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`JsError`] when an argument cannot cross the engine seam.
    fn into_js_arguments(self, bridge: &Bridge) -> Result<Vec<JsValue>, JsError>;
}

impl ActionArguments for () {
    const TYPES: &'static [TypeSchema] = &[];

    fn into_js_arguments(self, _bridge: &Bridge) -> Result<Vec<JsValue>, JsError> {
        Ok(Vec::new())
    }
}

/// One tuple arity.
macro_rules! action_arguments {
    ($($parameter:ident: $value:ident),+) => {
        impl<$($parameter: IntoJs + TsType),+> ActionArguments for ($($parameter,)+) {
            const TYPES: &'static [TypeSchema] = &[$(<$parameter as TsType>::SCHEMA),+];

            fn into_js_arguments(self, bridge: &Bridge) -> Result<Vec<JsValue>, JsError> {
                let ($($value,)+) = self;
                Ok(vec![$($value.into_js(bridge)?),+])
            }
        }
    };
}

action_arguments!(A: a);
action_arguments!(A: a, B: b);
action_arguments!(A: a, B: b, C: c);
action_arguments!(A: a, B: b, C: c, D: d);
action_arguments!(A: a, B: b, C: c, D: d, E: e);
action_arguments!(A: a, B: b, C: c, D: d, E: e, F: f);
action_arguments!(A: a, B: b, C: c, D: d, E: e, F: f, G: g);
action_arguments!(A: a, B: b, C: c, D: d, E: e, F: f, G: g, H: h);

/// A JavaScript function the host calls: an event handler, never a value that
/// is read or subscribed to.
///
/// `Args` is what the event carries, so the type is the handler's signature:
/// `JsAction<(u32,)>` projects to `(arg0: number) => void` and is called with
/// that one number, while the bare `JsAction` projects to `() => void` and is
/// called with `()`. The arguments are one tuple rather than one type per
/// arity, which is why an attribute declaring a handler declares its whole
/// signature in one place.
///
/// The bridge is captured with the function, because calling back into
/// JavaScript needs the engine. Dropping the runtime while a view still holds
/// one is not a crash: the call reports that the runtime is gone.
pub struct JsAction<Args = ()> {
    function: JsFunction,
    bridge: waterui_ts::WeakBridge,
    /// The arguments the handler takes. They are written out at every call and
    /// never stored, which is what the `fn(Args)` spelling says.
    arguments: PhantomData<fn(Args)>,
}

impl<Args> Clone for JsAction<Args> {
    fn clone(&self) -> Self {
        Self {
            function: self.function.clone(),
            bridge: self.bridge.clone(),
            arguments: PhantomData,
        }
    }
}

impl<Args> core::fmt::Debug for JsAction<Args> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("JsAction").finish_non_exhaustive()
    }
}

impl<Args: ActionArguments> JsAction<Args> {
    /// Calls the handler with the values the event carried, logging a
    /// JavaScript exception rather than unwinding into the backend that
    /// delivered the event.
    pub fn call(&self, arguments: Args) {
        let Some(bridge) = self.bridge.upgrade() else {
            tracing::warn!(
                "a TypeScript event handler fired after its runtime was dropped, and did nothing"
            );
            return;
        };
        let arguments = match arguments.into_js_arguments(&bridge) {
            Ok(arguments) => arguments,
            Err(error) => {
                tracing::error!(%error, "a TypeScript event handler's arguments could not cross");
                return;
            }
        };
        if let Err(error) = bridge.call(&self.function, &arguments) {
            tracing::error!(%error, "a TypeScript event handler threw");
        }
    }
}

impl<Args: ActionArguments> TsType for JsAction<Args> {
    const SCHEMA: TypeSchema = TypeSchema::Callback(Args::TYPES);
}

impl<Args> FromJs for JsAction<Args> {
    fn from_js(value: &JsValue, bridge: &Bridge) -> Result<Self, JsError> {
        match value {
            JsValue::Function(function) => Ok(Self {
                function: function.clone(),
                bridge: bridge.downgrade(),
                arguments: PhantomData,
            }),
            other => Err(expected("a function", other)),
        }
    }
}

impl<Args> IntoJs for JsAction<Args> {
    /// The function itself: a handler that came from JavaScript goes back as
    /// what it was.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Function(self.function))
    }
}

/// One option of a `<Picker>`: the value it selects and the text it shows.
#[derive(Debug, Clone, PartialEq, Eq, TsType)]
#[non_exhaustive]
pub struct PickerOption {
    /// The value written into the picker's `value` signal when this option is
    /// chosen.
    value: Str,
    /// The option's label, localized like any other text.
    label: TextContent,
}

impl PickerOption {
    /// The value written into the picker's `value` signal when this option is
    /// chosen.
    #[must_use]
    pub const fn value(&self) -> &Str {
        &self.value
    }

    /// The option's label, localized like any other text.
    #[must_use]
    pub const fn label(&self) -> &TextContent {
        &self.label
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (Str, TextContent) {
        (self.value, self.label)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use suiteki::Str;
    use waterui_graphics::WorkingColor;
    use waterui_graphics::color::Srgb;
    use waterui_ts::schema::{TsType as _, TypeSchema};

    use super::{
        ColorValue, LOCALIZED_TEXT_SCHEMA, LinearColor, NumberKind, TextContent, interpolate,
    };

    #[test]
    fn text_content_schema_accepts_string_number_or_localized_text() {
        assert_eq!(
            TextContent::SCHEMA,
            TypeSchema::Union(&[
                TypeSchema::String,
                TypeSchema::Number(NumberKind::F64),
                LOCALIZED_TEXT_SCHEMA,
            ])
        );
    }

    #[test]
    fn interpolate_substitutes_named_arguments_and_unescapes_braces() {
        let mut args = BTreeMap::new();
        args.insert(Str::from_static("name"), Str::from_static("Ada"));
        args.insert(Str::from_static("id"), Str::from_static("42"));

        assert_eq!(
            interpolate("Hello, {name}!", &args),
            Str::from_static("Hello, Ada!")
        );
        assert_eq!(
            interpolate("Order #{id} for {{ {name} }}", &args),
            Str::from_static("Order #42 for { Ada }")
        );
        assert_eq!(
            interpolate("Keep {unknown} and {unclosed", &args),
            Str::from_static("Keep {unknown} and {unclosed")
        );
    }

    #[test]
    fn color_value_schema_accepts_hex_string_or_linear_object() {
        assert_eq!(
            ColorValue::SCHEMA,
            TypeSchema::Union(&[TypeSchema::String, LinearColor::SCHEMA])
        );
    }

    #[test]
    fn srgb_hex_formats_convert_to_working_color() {
        let rrggbb = ColorValue::from_srgb_hex("#FF3B30").expect("#RRGGBB must parse");
        let expected = Srgb::new_u8(0xFF, 0x3B, 0x30).resolve();
        let actual = WorkingColor::from(rrggbb);
        for (a, b) in actual.components.into_iter().zip(expected.components) {
            assert!((a - b).abs() < 1e-6, "component mismatch: {a} vs {b}");
        }

        let short_rgb = ColorValue::from_srgb_hex("#F0A").expect("#RGB must parse");
        let expanded_rgb = ColorValue::from_srgb_hex("#FF00AA").expect("#RRGGBB must parse");
        assert_eq!(short_rgb, expanded_rgb);

        let short_with_alpha = ColorValue::from_srgb_hex("#F0A8").expect("#RGBA must parse");
        let expanded_with_alpha =
            ColorValue::from_srgb_hex("#FF00AA88").expect("#RRGGBBAA must parse");
        assert_eq!(short_with_alpha, expanded_with_alpha);
        assert!((short_with_alpha.opacity().unwrap_or(0.0) - (136.0 / 255.0)).abs() < 1e-6);
    }

    #[test]
    fn invalid_srgb_hex_strings_are_rejected() {
        for bad in ["", "FF3B30", "#12", "#12345", "#1234567", "#GGGGGG"] {
            assert!(
                ColorValue::from_srgb_hex(bad).is_err(),
                "`{bad}` must be rejected as an invalid hex colour"
            );
        }
    }
}
