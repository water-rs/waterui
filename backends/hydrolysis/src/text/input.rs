//! The engine-neutral shaping input.
//!
//! A [`ResolvedTextLayoutInput`] is a fully-resolved, self-contained text
//! layout description: every reactive input (signal values, [`waterui_core::Str`]-backed
//! font families) has been read out and projected into owned data, so shaping
//! it is a pure function of it — which is what makes the content-keyed layout
//! cache correct. Resolving reads the [`Environment`] and reactive signals,
//! so it must run on the main thread; the resolved value is `Send` and can be
//! shaped on any thread.

use core::hash::{Hash, Hasher};
use core::ops::Range;
use std::sync::Arc;

use nami::Signal as _;
use rustc_hash::FxHasher;
use waterui::theme;
use waterui_core::Environment;
use waterui_core::layout::{HorizontalAlignment, layout_direction};
use waterui_graphics::color::Color;
use waterui_text::font::FontWeight as TextFontWeight;
use waterui_text::styled::{Style as TextStyle, StyledStr};

use crate::renderer::working_color_to_rgba8;

/// A fully-resolved, self-contained text layout description.
///
/// Every reactive input (signal values, [`waterui_core::Str`]-backed font families) has been
/// read out and projected into owned data, so shaping this input is a pure
/// function of it — which is what makes the content-keyed cache correct.
pub struct ResolvedTextLayoutInput {
    pub(in crate::text) plain: String,
    pub(in crate::text) spans: Vec<(Range<usize>, ResolvedTextStyleSpec)>,
    pub(in crate::text) default_font: ResolvedFontSpec,
    pub(in crate::text) default_brush: [u8; 4],
    pub(in crate::text) locale: String,
    pub(in crate::text) alignment: HorizontalAlignment,
    pub(in crate::text) right_to_left: bool,
    /// This input's width-independent cache identity, built with the input.
    pub(in crate::text) identity: Arc<TextLayoutIdentity>,
}

impl ResolvedTextLayoutInput {
    /// `true` when the text is empty — an empty input shapes to the engine's
    /// empty layout without touching the cache.
    pub(in crate::text) const fn is_empty(&self) -> bool {
        self.plain.is_empty()
    }

    pub(in crate::text) fn cache_key(&self, max_width: Option<f32>) -> TextLayoutCacheKey {
        TextLayoutCacheKey {
            identity: Arc::clone(&self.identity),
            max_width: max_width.map(f32::to_bits),
        }
    }

    /// The spans that paint a background, as `(range, colour)` pairs — the
    /// background contract a layout carries out of the input it was shaped
    /// from.
    pub(in crate::text) fn background_spans(&self) -> Box<[(Range<usize>, [u8; 4])]> {
        self.spans
            .iter()
            .filter_map(|(range, style)| style.background.map(|colour| (range.clone(), colour)))
            .collect()
    }

    /// The same shaping defaults respelled over different text — how the
    /// ellipsis probe and each truncated re-shape mint their inputs.
    pub(in crate::text) fn respell(
        &self,
        plain: String,
        spans: Vec<(Range<usize>, ResolvedTextStyleSpec)>,
    ) -> Self {
        let alignment_id = self.alignment.stable_id();
        let identity = Arc::new(TextLayoutIdentity::new(
            plain.clone(),
            spans
                .iter()
                .map(|(range, style)| span_cache_key(range, style))
                .collect(),
            text_layout_font_cache_key(&self.default_font),
            self.default_brush,
            self.locale.clone(),
            TextLayoutAlignmentCacheKey {
                low: alignment_id.low(),
                high: alignment_id.high(),
                right_to_left: self.right_to_left,
            },
        ));
        Self {
            plain,
            spans,
            default_font: self.default_font.clone(),
            default_brush: self.default_brush,
            locale: self.locale.clone(),
            alignment: self.alignment,
            right_to_left: self.right_to_left,
            identity,
        }
    }
}

/// `Send` projection of a resolved font (the `!Send` [`waterui_core::Str`] family is copied
/// into an owned [`String`]).
#[derive(Clone)]
pub(in crate::text) struct ResolvedFontSpec {
    pub(in crate::text) size: f32,
    pub(in crate::text) weight: TextFontWeight,
    pub(in crate::text) line_height: Option<f32>,
    pub(in crate::text) letter_spacing: f32,
    pub(in crate::text) family: Option<String>,
}

/// `Send` projection of a resolved text run style.
#[derive(Clone)]
pub(in crate::text) struct ResolvedTextStyleSpec {
    pub(in crate::text) font: ResolvedFontSpec,
    pub(in crate::text) foreground: Option<[u8; 4]>,
    pub(in crate::text) background: Option<[u8; 4]>,
    pub(in crate::text) italic: bool,
    pub(in crate::text) underline: bool,
    pub(in crate::text) strikethrough: bool,
}

/// Resolve a [`StyledStr`] into a `Send` [`ResolvedTextLayoutInput`].
///
/// Reads the environment and reactive signals, so it must run on the main
/// thread. The returned value can then be shaped on any thread.
pub fn resolve_text_layout_input(
    styled: &StyledStr,
    alignment: HorizontalAlignment,
    env: &Environment,
) -> ResolvedTextLayoutInput {
    let mut plain = String::new();
    let mut spans = Vec::with_capacity(styled.chunks().len());
    for (chunk, style) in styled.chunks() {
        let start = plain.len();
        plain.push_str(chunk.as_str());
        let end = plain.len();
        spans.push((start..end, resolve_text_style(style, env)));
    }

    let default_font = font_spec(&waterui_text::font::Font::default().resolve(env).snapshot());
    let default_brush = default_text_brush(env);
    let locale = text_layout_locale(env);
    let right_to_left = layout_direction(env).snapshot().is_right_to_left();
    let alignment_id = alignment.stable_id();
    let identity = Arc::new(TextLayoutIdentity::new(
        plain.clone(),
        spans
            .iter()
            .map(|(range, style)| span_cache_key(range, style))
            .collect(),
        text_layout_font_cache_key(&default_font),
        default_brush,
        locale.clone(),
        TextLayoutAlignmentCacheKey {
            low: alignment_id.low(),
            high: alignment_id.high(),
            right_to_left,
        },
    ));
    ResolvedTextLayoutInput {
        plain,
        spans,
        default_font,
        default_brush,
        locale,
        alignment,
        right_to_left,
        identity,
    }
}

pub(in crate::text) fn span_cache_key(
    range: &Range<usize>,
    style: &ResolvedTextStyleSpec,
) -> TextLayoutSpanCacheKey {
    TextLayoutSpanCacheKey {
        start: range.start,
        end: range.end,
        font: text_layout_font_cache_key(&style.font),
        foreground: style.foreground,
        background: style.background,
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
    }
}

pub(in crate::text) fn font_spec(font: &waterui_text::font::ResolvedFont) -> ResolvedFontSpec {
    ResolvedFontSpec {
        size: font.size,
        weight: font.weight,
        line_height: font.line_height,
        letter_spacing: font.letter_spacing,
        family: font.family.as_deref().map(String::from),
    }
}

fn resolve_text_style(style: &TextStyle, env: &Environment) -> ResolvedTextStyleSpec {
    ResolvedTextStyleSpec {
        font: font_spec(&style.font.resolve(env).snapshot()),
        foreground: style
            .foreground
            .clone()
            .map(|color| working_color_to_rgba8(color.resolve(env).snapshot())),
        background: style
            .background
            .clone()
            .map(|color| working_color_to_rgba8(color.resolve(env).snapshot())),
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
    }
}

fn default_text_brush(env: &Environment) -> [u8; 4] {
    let color = theme::installed_color_signal::<theme::color::Foreground>(env).map_or_else(
        || Color::srgb(0, 0, 0).resolve(env).snapshot(),
        |signal| signal.snapshot(),
    );
    working_color_to_rgba8(color)
}

/// The BCP 47 tag the environment's locale resolves to — the language shaping
/// runs under.
pub(in crate::text) fn text_layout_locale(env: &Environment) -> String {
    waterui_locale::locale_binding(env)
        .snapshot()
        .canonical_tag()
}

fn text_layout_font_cache_key(font: &ResolvedFontSpec) -> TextLayoutFontCacheKey {
    TextLayoutFontCacheKey {
        size: font.size.to_bits(),
        weight: text_font_weight_cache_key(font.weight),
        line_height: font.line_height.map(f32::to_bits),
        letter_spacing: font.letter_spacing.to_bits(),
        family: font.family.clone(),
    }
}

const fn text_font_weight_cache_key(weight: TextFontWeight) -> u16 {
    match weight {
        TextFontWeight::Thin => 100,
        TextFontWeight::UltraLight => 200,
        TextFontWeight::Light => 300,
        TextFontWeight::Normal => 400,
        TextFontWeight::Medium => 500,
        TextFontWeight::SemiBold => 600,
        TextFontWeight::Bold => 700,
        TextFontWeight::UltraBold => 800,
        TextFontWeight::Black => 900,
    }
}

/// Everything that identifies a shaped layout except the width it is fitted to.
///
/// Built once per [`ResolvedTextLayoutInput`] and shared by every width probe of
/// that text: a container measuring one leaf against several proposals then
/// allocates this once instead of once per probe. All fields are owned and
/// `Send`, so the cache can be shared across threads.
#[derive(Debug, Eq)]
pub(in crate::text) struct TextLayoutIdentity {
    text: String,
    spans: Vec<TextLayoutSpanCacheKey>,
    default_font: TextLayoutFontCacheKey,
    default_brush: [u8; 4],
    locale: String,
    alignment: TextLayoutAlignmentCacheKey,
    /// Hash of every field above, computed once at construction. [`Hash`] writes
    /// only this, so a cache probe never re-walks the text and its spans;
    /// equality still compares the fields, so a collision cannot return the
    /// wrong layout.
    hash: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::text) struct TextLayoutAlignmentCacheKey {
    pub(in crate::text) low: u64,
    pub(in crate::text) high: u64,
    pub(in crate::text) right_to_left: bool,
}

impl TextLayoutIdentity {
    pub(in crate::text) fn new(
        text: String,
        spans: Vec<TextLayoutSpanCacheKey>,
        default_font: TextLayoutFontCacheKey,
        default_brush: [u8; 4],
        locale: String,
        alignment: TextLayoutAlignmentCacheKey,
    ) -> Self {
        let mut hasher = FxHasher::default();
        text.hash(&mut hasher);
        spans.hash(&mut hasher);
        default_font.hash(&mut hasher);
        default_brush.hash(&mut hasher);
        locale.hash(&mut hasher);
        alignment.hash(&mut hasher);
        Self {
            text,
            spans,
            default_font,
            default_brush,
            locale,
            alignment,
            hash: hasher.finish(),
        }
    }
}

impl PartialEq for TextLayoutIdentity {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.text == other.text
            && self.spans == other.spans
            && self.default_font == other.default_font
            && self.default_brush == other.default_brush
            && self.locale == other.locale
            && self.alignment == other.alignment
    }
}

impl Hash for TextLayoutIdentity {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash);
    }
}

/// Cache key for a shaped layout: what to shape, and how wide.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::text) struct TextLayoutCacheKey {
    pub(in crate::text) identity: Arc<TextLayoutIdentity>,
    pub(in crate::text) max_width: Option<u32>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::text) struct TextLayoutSpanCacheKey {
    pub(in crate::text) start: usize,
    pub(in crate::text) end: usize,
    pub(in crate::text) font: TextLayoutFontCacheKey,
    pub(in crate::text) foreground: Option<[u8; 4]>,
    pub(in crate::text) background: Option<[u8; 4]>,
    pub(in crate::text) italic: bool,
    pub(in crate::text) underline: bool,
    pub(in crate::text) strikethrough: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(in crate::text) struct TextLayoutFontCacheKey {
    pub(in crate::text) size: u32,
    pub(in crate::text) weight: u16,
    pub(in crate::text) line_height: Option<u32>,
    pub(in crate::text) letter_spacing: u32,
    pub(in crate::text) family: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::text_layout_locale;
    use waterui_core::Environment;
    use waterui_locale::locales;

    #[test]
    fn text_layout_locale_uses_environment_locale() {
        let mut env = Environment::new();
        env.insert(locales::ZH_TW);

        assert_eq!(text_layout_locale(&env), "zh-TW");
    }
}
