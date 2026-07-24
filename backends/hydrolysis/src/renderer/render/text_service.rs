//! Thread-safe text shaping and measurement.
//!
//! Text shaping (parley layout) is the heaviest part of measuring a text leaf,
//! and it is pure over fully-resolved, `Send` inputs. This module splits the
//! work into two phases so the expensive half can run off the main thread:
//!
//! 1. **Resolve** ([`resolve_text_layout_input`]) reads the [`Environment`] and
//!    reactive signals to turn a [`StyledStr`] into a fully-`Send`
//!    [`ResolvedTextLayoutInput`]. This must run on the main thread because it
//!    touches `!Send` reactive state ([`Str`]-backed fonts, signal `.get()`s).
//! 2. **Shape** ([`TextMeasureService::shape`]) takes that resolved input and
//!    produces a [`parley::Layout`]. It touches no `!Send` state and runs on any
//!    thread, which is what lets [`HydroSubview`](super::HydroSubview) report
//!    `require_main_thread() == false` for text leaves so the layout executor
//!    measures them across worker threads.
//!
//! The service owns the loaded fonts plus a shared layout cache, so the
//! main-thread render path and worker-thread measurement shape identical text
//! through one cache.

use super::*;
use core::ops::Range;
use rustc_hash::FxHashMap;
use std::sync::Mutex;

/// Thread-safe text shaping service shared by the render path and layout
/// measurement. Cheaply cloneable shaping scratch is pooled so each worker
/// reuses a [`parley::FontContext`] carrying the registered resource fonts.
pub(crate) struct TextMeasureService {
    /// Fonts registered at startup; the clone source for shaping scratch.
    /// Mutated only during single-threaded font registration via
    /// [`Self::fonts_mut`], read-only afterward.
    fonts: parley::FontContext,
    /// Shared layout cache — one source of truth for main-thread render and
    /// worker-thread measurement. Locked only to get/insert; shaping happens
    /// outside the lock.
    cache: Mutex<FxHashMap<TextLayoutCacheKey, parley::Layout<[u8; 4]>>>,
    /// Reusable `(FontContext, LayoutContext)` shaping scratch, checked out per
    /// shape call. Each entry is a clone of [`Self::fonts`] (carrying the
    /// registered resource fonts) plus a fresh layout context; the pool size
    /// settles at the number of concurrent shapers.
    scratch: Mutex<Vec<TextShapingScratch>>,
}

/// Owned, mutable shaping scratch for one in-flight shape call.
struct TextShapingScratch {
    font_cx: parley::FontContext,
    layout_cx: parley::LayoutContext<[u8; 4]>,
}

impl TextMeasureService {
    pub(crate) fn new() -> Self {
        Self {
            fonts: parley::FontContext::new(),
            cache: Mutex::new(FxHashMap::default()),
            scratch: Mutex::new(Vec::new()),
        }
    }

    /// Mutable access to the registered fonts, for startup font registration.
    ///
    /// Registering fonts invalidates any shaping scratch and cached layouts
    /// produced before the registration, so text shaped before its fonts were
    /// installed can never be reused. Requires unique ownership of the service,
    /// which holds during single-threaded setup before any subview clones it.
    pub(crate) fn fonts_mut(&mut self) -> &mut parley::FontContext {
        self.cache
            .get_mut()
            .expect("hydrolysis text layout cache mutex poisoned")
            .clear();
        self.scratch
            .get_mut()
            .expect("hydrolysis text shaping scratch mutex poisoned")
            .clear();
        &mut self.fonts
    }

    /// Shape `input` into a parley layout, reusing the shared cache. Safe to call
    /// from any thread.
    pub(crate) fn shape(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
    ) -> parley::Layout<[u8; 4]> {
        if input.plain.is_empty() {
            return parley::Layout::new();
        }

        let cache_key = input.cache_key(max_width);
        if let Some(layout) = self
            .cache
            .lock()
            .expect("hydrolysis text layout cache mutex poisoned")
            .get(&cache_key)
        {
            return layout.clone();
        }

        let mut scratch = self.checkout_scratch();
        let layout = build_parley_layout(&mut scratch, input, max_width);
        self.return_scratch(scratch);

        self.cache
            .lock()
            .expect("hydrolysis text layout cache mutex poisoned")
            .insert(cache_key, layout.clone());
        layout
    }

    fn checkout_scratch(&self) -> TextShapingScratch {
        self.scratch
            .lock()
            .expect("hydrolysis text shaping scratch mutex poisoned")
            .pop()
            .unwrap_or_else(|| TextShapingScratch {
                font_cx: self.fonts.clone(),
                layout_cx: parley::LayoutContext::new(),
            })
    }

    fn return_scratch(&self, scratch: TextShapingScratch) {
        self.scratch
            .lock()
            .expect("hydrolysis text shaping scratch mutex poisoned")
            .push(scratch);
    }
}

impl core::fmt::Debug for TextMeasureService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextMeasureService").finish_non_exhaustive()
    }
}

/// A fully-resolved, `Send` text layout description.
///
/// Every reactive/`!Send` input (signal values, [`Str`]-backed font families)
/// has been read on the main thread and projected into owned, thread-safe data,
/// so shaping this input is a pure function that runs on any thread.
pub(crate) struct ResolvedTextLayoutInput {
    plain: String,
    spans: Vec<(Range<usize>, ResolvedTextStyleSpec)>,
    default_font: ResolvedFontSpec,
    default_brush: [u8; 4],
    locale: String,
    alignment: HorizontalAlignment,
}

impl ResolvedTextLayoutInput {
    fn cache_key(&self, max_width: Option<f32>) -> TextLayoutCacheKey {
        let alignment_id = self.alignment.stable_id();
        TextLayoutCacheKey {
            text: self.plain.clone(),
            spans: self
                .spans
                .iter()
                .map(|(range, style)| TextLayoutSpanCacheKey {
                    start: range.start,
                    end: range.end,
                    font: text_layout_font_cache_key(&style.font),
                    foreground: style.foreground,
                    italic: style.italic,
                    underline: style.underline,
                    strikethrough: style.strikethrough,
                })
                .collect(),
            default_font: text_layout_font_cache_key(&self.default_font),
            default_brush: self.default_brush,
            locale: self.locale.clone(),
            alignment_low: alignment_id.low(),
            alignment_high: alignment_id.high(),
            max_width: max_width.map(f32::to_bits),
        }
    }
}

/// `Send` projection of a resolved font (the `!Send` [`Str`] family is copied
/// into an owned [`String`]).
struct ResolvedFontSpec {
    size: f32,
    weight: TextFontWeight,
    line_height: Option<f32>,
    letter_spacing: f32,
    family: Option<String>,
}

/// `Send` projection of a resolved text run style.
struct ResolvedTextStyleSpec {
    font: ResolvedFontSpec,
    foreground: Option<[u8; 4]>,
    italic: bool,
    underline: bool,
    strikethrough: bool,
}

/// Resolve a [`StyledStr`] into a `Send` [`ResolvedTextLayoutInput`].
///
/// Reads the environment and reactive signals, so it must run on the main
/// thread. The returned value can then be shaped on any thread.
pub(crate) fn resolve_text_layout_input(
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

    let default_font = font_spec(&waterui_text::font::Font::default().resolve(env).get());
    let default_brush = default_text_brush(env);
    let locale = text_layout_locale(env);
    ResolvedTextLayoutInput {
        plain,
        spans,
        default_font,
        default_brush,
        locale,
        alignment,
    }
}

fn font_spec(font: &waterui_text::font::ResolvedFont) -> ResolvedFontSpec {
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
        font: font_spec(&style.font.resolve(env).get()),
        foreground: style
            .foreground
            .clone()
            .map(|color| resolved_color_to_rgba8(color.resolve(env).get())),
        italic: style.italic,
        underline: style.underline,
        strikethrough: style.strikethrough,
    }
}

fn default_text_brush(env: &Environment) -> [u8; 4] {
    let color = theme::installed_color_signal::<theme::color::Foreground>(env).map_or_else(
        || Color::srgb(0, 0, 0).resolve(env).get(),
        |signal| signal.get(),
    );
    resolved_color_to_rgba8(color)
}

fn build_parley_layout(
    scratch: &mut TextShapingScratch,
    input: &ResolvedTextLayoutInput,
    max_width: Option<f32>,
) -> parley::Layout<[u8; 4]> {
    let mut builder =
        scratch
            .layout_cx
            .ranged_builder(&mut scratch.font_cx, &input.plain, 1.0, true);
    builder.push_default(parley::StyleProperty::Brush(input.default_brush));
    builder.push_default(parley::StyleProperty::FontSize(input.default_font.size));
    builder.push_default(parley::StyleProperty::FontWeight(parley_font_weight(
        input.default_font.weight,
    )));
    if let Some(line_height) = input.default_font.line_height {
        builder.push_default(parley::StyleProperty::LineHeight(
            parley::LineHeight::Absolute(line_height),
        ));
    }
    builder.push_default(parley::StyleProperty::LetterSpacing(
        input.default_font.letter_spacing,
    ));
    let locale = input
        .locale
        .parse::<parley::Language>()
        .unwrap_or_else(|error| {
            panic!(
                "WaterUI locale `{}` is not a valid BCP 47 language tag: {error}",
                input.locale
            )
        });
    builder.push_default(parley::StyleProperty::Locale(Some(locale)));
    builder.push_default(parley::StyleProperty::FontFamily(font_family(
        input.default_font.family.as_deref(),
    )));

    for (range, style) in &input.spans {
        push_text_style(&mut builder, style, range.clone());
    }

    let mut layout = builder.build(&input.plain);
    layout.break_all_lines(max_width);
    layout.align(
        parley_alignment(input.alignment),
        parley::AlignmentOptions::default(),
    );
    layout
}

fn push_text_style(
    builder: &mut parley::RangedBuilder<'_, [u8; 4]>,
    style: &ResolvedTextStyleSpec,
    range: Range<usize>,
) {
    builder.push(
        parley::StyleProperty::FontSize(style.font.size),
        range.clone(),
    );
    builder.push(
        parley::StyleProperty::FontWeight(parley_font_weight(style.font.weight)),
        range.clone(),
    );
    if let Some(line_height) = style.font.line_height {
        builder.push(
            parley::StyleProperty::LineHeight(parley::LineHeight::Absolute(line_height)),
            range.clone(),
        );
    }
    builder.push(
        parley::StyleProperty::LetterSpacing(style.font.letter_spacing),
        range.clone(),
    );
    if let Some(family) = &style.font.family {
        builder.push(
            parley::StyleProperty::FontFamily(font_family(Some(family.as_str()))),
            range.clone(),
        );
    }
    builder.push(
        parley::StyleProperty::FontStyle(if style.italic {
            parley::FontStyle::Italic
        } else {
            parley::FontStyle::Normal
        }),
        range.clone(),
    );
    builder.push(
        parley::StyleProperty::Underline(style.underline),
        range.clone(),
    );
    builder.push(
        parley::StyleProperty::Strikethrough(style.strikethrough),
        range.clone(),
    );
    if let Some(color) = style.foreground {
        builder.push(parley::StyleProperty::Brush(color), range);
    }
}

fn font_family(family: Option<&str>) -> parley::FontFamily<'static> {
    family.map_or_else(
        || parley::style::GenericFamily::SansSerif.into(),
        |family| parley::FontFamily::Source(std::borrow::Cow::Owned(family.to_string())),
    )
}

fn text_layout_locale(env: &Environment) -> String {
    waterui_locale::locale_binding(env).get().canonical_tag()
}

/// Compute view dimensions (size plus first/last baselines) from a shaped
/// layout. Pure; safe to call on any thread.
pub(crate) fn text_dimensions_from_layout(
    layout: &parley::Layout<[u8; 4]>,
    max_lines: Option<usize>,
) -> ViewDimensions {
    if layout.is_empty() {
        return ViewDimensions::new(LayoutSize::zero());
    }

    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    let mut first_baseline = None;
    let mut last_baseline = None;

    for (index, line) in layout.lines().enumerate() {
        if max_lines.is_some_and(|limit| index >= limit) {
            break;
        }
        let metrics = line.metrics();
        width = width.max(metrics.advance);
        height += metrics.line_height;
        if first_baseline.is_none() {
            first_baseline = Some(metrics.baseline);
        }
        last_baseline = Some(metrics.baseline);
    }

    let mut dimensions = ViewDimensions::new(LayoutSize::new(width, height));
    if let Some(first_baseline) = first_baseline {
        dimensions.set_vertical(VerticalAlignment::FirstBaseline, first_baseline);
    }
    if let Some(last_baseline) = last_baseline {
        dimensions.set_vertical(VerticalAlignment::LastBaseline, last_baseline);
    }
    dimensions
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

/// Cache key for a shaped [`parley::Layout`]. All fields are owned and `Send`,
/// so the cache can be shared across threads.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TextLayoutCacheKey {
    pub(crate) text: String,
    pub(crate) spans: Vec<TextLayoutSpanCacheKey>,
    pub(crate) default_font: TextLayoutFontCacheKey,
    pub(crate) default_brush: [u8; 4],
    pub(crate) locale: String,
    pub(crate) alignment_low: u64,
    pub(crate) alignment_high: u64,
    pub(crate) max_width: Option<u32>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TextLayoutSpanCacheKey {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) font: TextLayoutFontCacheKey,
    pub(crate) foreground: Option<[u8; 4]>,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    pub(crate) strikethrough: bool,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TextLayoutFontCacheKey {
    pub(crate) size: u32,
    pub(crate) weight: u16,
    pub(crate) line_height: Option<u32>,
    pub(crate) letter_spacing: u32,
    pub(crate) family: Option<String>,
}

#[cfg(test)]
mod font_family_tests {
    use super::{font_family, text_layout_locale};
    use parley::style::GenericFamily;
    use waterui_core::Environment;
    use waterui_locale::locales;

    #[test]
    fn explicit_family_list_is_preserved_for_parley_css_parsing() {
        let family = font_family(Some("Roboto, Noto Sans CJK SC, sans-serif"));

        assert_eq!(
            family,
            parley::FontFamily::Source("Roboto, Noto Sans CJK SC, sans-serif".into())
        );
    }

    #[test]
    fn missing_family_uses_sans_serif_generic() {
        assert_eq!(
            font_family(None),
            parley::FontFamily::from(GenericFamily::SansSerif)
        );
    }

    #[test]
    fn text_layout_locale_uses_environment_locale() {
        let mut env = Environment::new();
        env.insert(locales::ZH_TW);

        assert_eq!(text_layout_locale(&env), "zh-TW");
    }
}
