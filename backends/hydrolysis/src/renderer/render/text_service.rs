//! Text shaping and measurement.
//!
//! Shaping (parley layout) is by far the heaviest part of measuring a text leaf —
//! tens of microseconds against the tens of nanoseconds every other leaf costs — so
//! it is the one measure that genuinely must be cached. This module splits the work
//! into two phases to make that cache correct:
//!
//! 1. **Resolve** ([`resolve_text_layout_input`]) reads the [`Environment`] and
//!    reactive signals to turn a [`StyledStr`] into a self-contained
//!    [`ResolvedTextLayoutInput`], capturing everything that can affect shaping.
//! 2. **Shape** ([`TextMeasureService::shape`]) turns that input into a
//!    [`parley::Layout`]. It reads no ambient state, so it is a pure function of
//!    the resolved input and can be memoized on that input's content identity.
//!
//! The service owns the loaded fonts plus that layout cache, so the render path and
//! measurement shape identical text through one cache.

use super::*;
#[cfg(test)]
use crate::renderer::{HydroState, HydrolysisRenderer};
use core::hash::{Hash, Hasher};
use core::num::NonZeroUsize;
use core::ops::Range;
use icu_properties::props::{Emoji, EmojiPresentation};
use icu_properties::{CodePointSetData, CodePointSetDataBorrowed};
use lru::LruCache;
use rustc_hash::FxHasher;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::{GlyphId, MetadataProvider};
use std::sync::{Arc, Mutex};
use unicode_segmentation::UnicodeSegmentation;

/// Upper bound on retained shaped layouts.
///
/// Shaping is keyed by content *and* fitted width, so one text leaf mints an
/// entry per distinct proposal a container probes it with, and reactive text — a
/// clock, a counter, a field's value — mints one per distinct string. Unbounded,
/// the cache grows for the life of the process. This holds several times a dense
/// screen's working set, so steady-state UI still never misses, while a long
/// session evicts what it has stopped drawing.
const TEXT_LAYOUT_CACHE_CAPACITY: usize = 4096;

/// Capacity of the per-layout ink-extent LRU behind [`layout_ink_extent`]:
/// measure and encode ask for the same layout's extent in one frame, and
/// resize/re-layout replays measurements of layouts the shape cache still
/// holds — the memo makes steady-state re-measure cost nothing. Entries pin
/// their layout's `Arc` so the pointer key can never be recycled while the
/// entry lives.
const LAYOUT_INK_EXTENT_CACHE_CAPACITY: usize = 1024;

/// Capacity of the per-glyph ink-bounds LRU behind [`layout_ink_extent`]: one
/// entry per (face, glyph, size, instance) — a few hundred entries cover a
/// dense screen, the bound keeps long-tail documents from growing it forever.
const GLYPH_INK_BOUNDS_CACHE_CAPACITY: usize = 16384;

/// Thread-safe text shaping service shared by the render path and layout
/// measurement. Cheaply cloneable shaping scratch is pooled so each worker
/// reuses a [`parley::FontContext`] carrying the registered resource fonts.
pub(crate) struct TextMeasureService {
    /// Fonts registered at startup; the clone source for shaping scratch.
    /// Mutated only during single-threaded font registration via
    /// [`Self::fonts_mut`], read-only afterward.
    fonts: parley::FontContext,
    /// Shared layout cache — one source of truth for the render path and
    /// measurement. Bounded at [`TEXT_LAYOUT_CACHE_CAPACITY`], evicting
    /// least-recently-shaped entries. Layouts are shared as [`Arc`] so a hit
    /// hands out a handle instead of copying the glyph runs.
    cache: Mutex<LruCache<TextLayoutCacheKey, Arc<parley::Layout<[u8; 4]>>>>,
    /// Per-layout horizontal ink extent behind [`layout_ink_extent`].
    ink_extents: Mutex<LruCache<LayoutInkExtentKey, LayoutInkExtentValue>>,
    /// Per-glyph horizontal ink bounds behind [`layout_ink_extent`].
    /// `GlyphMetrics::bounds` answers TrueType faces straight from the `glyf`
    /// header table and draws the outline only for faces that need it (gvar,
    /// CFF/CFF2); memoizing per (face, glyph, size, instance) keeps either
    /// path paid once per measure, not once per glyph.
    ink_bounds: Mutex<LruCache<GlyphInkBoundsKey, GlyphInkBoundsValue>>,
    /// Reusable `(FontContext, LayoutContext)` shaping scratch, checked out per
    /// shape call. Built once as a clone of [`Self::fonts`] (carrying the
    /// registered resource fonts) plus a fresh layout context, then returned for
    /// the next call rather than rebuilt.
    scratch: Mutex<Option<TextShapingScratch>>,
    /// Encoded glyph scenes, keyed by the shaped layout's identity plus the
    /// draw-time line limit. The render path redraws every text leaf on every
    /// frame (whole-scene re-encode), and re-emitting glyph runs — font
    /// resolution, per-glyph iteration, run encoding — dominates a text leaf's
    /// flush cost. A fragment is encoded once at the local origin and appended
    /// under the frame's transform, so a scrolled or animated frame pays one
    /// encoding copy per text instead of a full glyph-run walk.
    scene_cache: Mutex<LruCache<TextSceneCacheKey, Arc<Recording>>>,
}

/// Cache identity for an encoded glyph scene: the shaped layout it draws plus
/// the line limit and tail truncation applied while drawing.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextSceneCacheKey {
    layout: TextLayoutCacheKey,
    max_lines: Option<usize>,
    tail_ellipsis: bool,
}

/// Cache identity for one glyph's ink bounds: the font blob's own id plus the
/// face index inside it (TTCs share a blob), the glyph, the pixel size as raw
/// bits, and the instance's normalized-variation coordinates hashed — hashing
/// keeps every lookup alloc-free; a hit verifies against the coords the value
/// recorded before it is trusted.
#[derive(Clone, Eq, Hash, PartialEq)]
struct GlyphInkBoundsKey {
    font: u64,
    face: u32,
    glyph: u32,
    size: u32,
    coords: u64,
}

/// Value side of the glyph ink bounds cache: the instance's raw `F2Dot14`
/// coordinate bits the bounds were measured under, then the bounds.
type GlyphInkBoundsValue = (Vec<i16>, Option<(f32, f32)>);

/// Cache identity for one layout's ink extent: the `Arc<Layout>`'s address —
/// the cached value pins the `Arc`, so the address is not recycled while the
/// entry lives — plus the `max_lines` truncation the extent was measured at.
type LayoutInkExtentKey = (usize, Option<usize>);

/// Value side of the layout ink-extent cache: the `Arc` keeping the key's
/// address live, then the extent.
type LayoutInkExtentValue = (Arc<parley::Layout<[u8; 4]>>, Option<(f32, f32)>);

/// Hash of a normalized-coordinate instance for [`GlyphInkBoundsKey`]; the
/// no-variations case is every static face, so it never allocates or hashes.
fn normalized_coords_hash(coords: &[i16]) -> u64 {
    if coords.is_empty() {
        return 0;
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash_slice(coords, &mut hasher);
    std::hash::Hasher::finish(&hasher)
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
            cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(TEXT_LAYOUT_CACHE_CAPACITY)
                    .expect("text layout cache capacity must be non-zero"),
            )),
            ink_extents: Mutex::new(LruCache::new(
                NonZeroUsize::new(LAYOUT_INK_EXTENT_CACHE_CAPACITY)
                    .expect("layout ink extent cache capacity must be non-zero"),
            )),
            ink_bounds: Mutex::new(LruCache::new(
                NonZeroUsize::new(GLYPH_INK_BOUNDS_CACHE_CAPACITY)
                    .expect("glyph ink bounds cache capacity must be non-zero"),
            )),
            scratch: Mutex::new(None),
            scene_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(TEXT_LAYOUT_CACHE_CAPACITY)
                    .expect("text scene cache capacity must be non-zero"),
            )),
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
            .expect("text layout cache mutex must not be poisoned")
            .clear();
        self.scene_cache
            .get_mut()
            .expect("text scene cache mutex must not be poisoned")
            .clear();
        self.ink_extents
            .get_mut()
            .expect("layout ink extent cache mutex must not be poisoned")
            .clear();
        *self
            .scratch
            .get_mut()
            .expect("text shaping scratch mutex must not be poisoned") = None;
        &mut self.fonts
    }

    /// Shape `input` into a parley layout, reusing the shared cache.
    ///
    /// The layout is shared rather than copied: every consumer only reads it,
    /// and copying glyph runs on every probe is the bulk of a cache hit's cost.
    pub(crate) fn shape(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
    ) -> Arc<parley::Layout<[u8; 4]>> {
        if input.plain.is_empty() {
            return Arc::new(parley::Layout::new());
        }

        let cache_key = input.cache_key(max_width);
        if let Some(layout) = self
            .cache
            .lock()
            .expect("text layout cache mutex must not be poisoned")
            .get(&cache_key)
        {
            return Arc::clone(layout);
        }

        let mut scratch = self.checkout_scratch();
        let layout = Arc::new(build_parley_layout(&mut scratch, input, max_width));
        self.return_scratch(scratch);

        self.cache
            .lock()
            .expect("text layout cache mutex must not be poisoned")
            .put(cache_key, Arc::clone(&layout));
        layout
    }

    /// Shape `input` with at most `max_lines` laid-out lines, truncating the
    /// last allowed line to a trailing ellipsis when the text does not fit.
    /// The truncated line is respelled — its kept clusters plus the marker —
    /// and re-shaped, so the layout's widest line is the honest laid-out
    /// width the leaf reports, and the drawn line is cached and encoded
    /// through the same paths as any other.
    pub(crate) fn shape_limited(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> Arc<parley::Layout<[u8; 4]>> {
        self.shape_limited_effective(input, max_width, max_lines).0
    }

    /// [`Self::shape_limited`] plus the input the returned layout was actually
    /// shaped from: `Some` is the respelled truncation input — whose span
    /// ranges are shifted and clamped to the cut text — so an encoder reads
    /// per-span properties against the same text the layout carries; `None`
    /// means `input` itself produced the layout.
    fn shape_limited_effective(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: Option<usize>,
    ) -> (
        Arc<parley::Layout<[u8; 4]>>,
        Option<ResolvedTextLayoutInput>,
    ) {
        let mut layout = self.shape(input, max_width);
        let Some(limit) = max_lines.filter(|limit| *limit > 0) else {
            return (layout, None);
        };
        if !needs_tail_truncation(&layout, limit) {
            return (layout, None);
        }

        let ellipsis_advance = self.ellipsis_advance(input, &layout, limit);
        let mut text = input.plain.clone();
        let mut spans = input.spans.clone();
        let mut respelled = None;
        while let Some(cut) = truncate_layout_tail(&layout, &text, &spans, limit, ellipsis_advance)
        {
            if cut.0 == text {
                break;
            }
            text = cut.0;
            spans = cut.1;
            let next = input.respell(text.clone(), spans.clone());
            layout = self.shape(&next, max_width);
            respelled = Some(next);
            if !needs_tail_truncation(&layout, limit) {
                break;
            }
        }
        (layout, respelled)
    }

    /// The marker's advance in the style at the tail of the last allowed line,
    /// so the truncation cut can reserve room for it.
    fn ellipsis_advance(
        &self,
        input: &ResolvedTextLayoutInput,
        layout: &parley::Layout<[u8; 4]>,
        limit: usize,
    ) -> f32 {
        let Some(line) = layout.get(layout.len().min(limit) - 1) else {
            return 0.0;
        };
        let end = line.text_range().end;
        let spans = input
            .spans
            .iter()
            .rev()
            .find(|(range, _)| range.start < end)
            .map_or_else(Vec::new, |(_, style)| {
                vec![(0..TAIL_ELLIPSIS.len_utf8(), style.clone())]
            });
        self.shape(&input.respell(String::from(TAIL_ELLIPSIS), spans), None)
            .get(0)
            .map_or(0.0, |line| line.metrics().advance)
    }

    /// The encoded glyph scene for `input` at `max_width`, drawn with its
    /// `tail` treatment, at the local origin (identity transform). A miss
    /// shapes through [`Self::shape`] — or [`Self::shape_limited`] for
    /// [`TailMark::Ellipsis`] — and encodes once via `encode`; a hit returns
    /// the shared fragment so the caller only pays a transformed append into
    /// the frame's scene. `encode` receives the input the layout was shaped
    /// from — `input` itself, or its respelled truncation tail — because a
    /// truncated layout's text ranges no longer match `input`'s spans.
    pub(crate) fn glyph_scene_with(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        tail: TailMark,
        encode: impl FnOnce(&Arc<parley::Layout<[u8; 4]>>, &ResolvedTextLayoutInput, &mut Recording),
    ) -> Arc<Recording> {
        let (max_lines, tail_ellipsis) = tail.parts();
        let key = TextSceneCacheKey {
            layout: input.cache_key(max_width),
            max_lines,
            tail_ellipsis,
        };
        if let Some(scene) = self
            .scene_cache
            .lock()
            .expect("text scene cache mutex must not be poisoned")
            .get(&key)
        {
            return Arc::clone(scene);
        }
        let (layout, respelled) = if tail_ellipsis {
            self.shape_limited_effective(input, max_width, max_lines)
        } else {
            (self.shape(input, max_width), None)
        };
        let mut scene = Recording::new();
        encode(&layout, respelled.as_ref().unwrap_or(input), &mut scene);
        let scene = Arc::new(scene);
        self.scene_cache
            .lock()
            .expect("text scene cache mutex must not be poisoned")
            .put(key, Arc::clone(&scene));
        scene
    }

    fn checkout_scratch(&self) -> TextShapingScratch {
        self.scratch
            .lock()
            .expect("text shaping scratch mutex must not be poisoned")
            .take()
            .unwrap_or_else(|| TextShapingScratch {
                font_cx: self.fonts.clone(),
                layout_cx: parley::LayoutContext::new(),
            })
    }

    fn return_scratch(&self, scratch: TextShapingScratch) {
        *self
            .scratch
            .lock()
            .expect("text shaping scratch mutex must not be poisoned") = Some(scratch);
    }
}

impl core::fmt::Debug for TextMeasureService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TextMeasureService").finish_non_exhaustive()
    }
}

/// A fully-resolved, self-contained text layout description.
///
/// Every reactive input (signal values, [`Str`]-backed font families) has been
/// read out and projected into owned data, so shaping this input is a pure
/// function of it — which is what makes the content-keyed cache correct.
pub(crate) struct ResolvedTextLayoutInput {
    plain: String,
    spans: Vec<(Range<usize>, ResolvedTextStyleSpec)>,
    default_font: ResolvedFontSpec,
    default_brush: [u8; 4],
    locale: String,
    alignment: HorizontalAlignment,
    right_to_left: bool,
    /// This input's width-independent cache identity, built with the input.
    identity: Arc<TextLayoutIdentity>,
}

impl ResolvedTextLayoutInput {
    fn cache_key(&self, max_width: Option<f32>) -> TextLayoutCacheKey {
        TextLayoutCacheKey {
            identity: Arc::clone(&self.identity),
            max_width: max_width.map(f32::to_bits),
        }
    }

    /// The background colour of the span covering `byte_index`, if that span
    /// painted one. Spans partition `plain`, so every index belongs to exactly
    /// one span.
    pub(crate) fn span_background(&self, byte_index: usize) -> Option<[u8; 4]> {
        self.spans
            .iter()
            .find(|(range, _)| range.contains(&byte_index))
            .and_then(|(_, style)| style.background)
    }

    /// `true` when at least one resolved span paints a background.
    pub(crate) fn has_background(&self) -> bool {
        self.spans
            .iter()
            .any(|(_, style)| style.background.is_some())
    }

    /// The same shaping defaults respelled over different text — how the
    /// ellipsis probe and each truncated re-shape mint their inputs.
    fn respell(&self, plain: String, spans: Vec<(Range<usize>, ResolvedTextStyleSpec)>) -> Self {
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

/// `Send` projection of a resolved font (the `!Send` [`Str`] family is copied
/// into an owned [`String`]).
#[derive(Clone)]
struct ResolvedFontSpec {
    size: f32,
    weight: TextFontWeight,
    line_height: Option<f32>,
    letter_spacing: f32,
    family: Option<String>,
}

/// `Send` projection of a resolved text run style.
#[derive(Clone)]
struct ResolvedTextStyleSpec {
    font: ResolvedFontSpec,
    foreground: Option<[u8; 4]>,
    background: Option<[u8; 4]>,
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

    let default_font = font_spec(&waterui_text::font::Font::default().resolve(env).snapshot());
    let default_brush = default_text_brush(env);
    let locale = text_layout_locale(env);
    let right_to_left = waterui_core::layout::layout_direction(env)
        .snapshot()
        .is_right_to_left();
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

fn span_cache_key(range: &Range<usize>, style: &ResolvedTextStyleSpec) -> TextLayoutSpanCacheKey {
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

    if !input.plain.is_ascii() {
        // A cluster whose presentation is emoji consults the `emoji` generic
        // family ahead of the family its own style resolved: a text face can
        // carry a monochrome glyph for an emoji codepoint, and it must not
        // win over a colour emoji face. The cluster's own family stays in
        // the list, after `emoji`, to answer a codepoint no emoji face
        // carries.
        for range in emoji_cluster_ranges(&input.plain) {
            let family = input
                .spans
                .iter()
                .find(|(span, _)| span.contains(&range.start))
                .and_then(|(_, style)| style.font.family.as_deref())
                .or(input.default_font.family.as_deref());
            builder.push(
                parley::StyleProperty::FontFamily(emoji_font_family(family)),
                range,
            );
        }
    }

    let mut layout = builder.build(&input.plain);
    layout.break_all_lines(max_width);
    layout.align(
        parley_alignment(input.alignment, input.right_to_left),
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

/// The `font-family` value an emoji-presentation range shapes with: the
/// `emoji` generic family first, then the family list the range's own style
/// resolved — `sans-serif` when nothing more specific applied.
fn emoji_font_family(family: Option<&str>) -> parley::FontFamily<'static> {
    let own = family.map_or_else(
        || {
            vec![parley::FontFamilyName::Generic(
                parley::GenericFamily::SansSerif,
            )]
        },
        |family| {
            parley::FontFamilyName::parse_css_list(family)
                .map(|name| name.map(parley::FontFamilyName::into_owned))
                .collect::<Result<Vec<_>, _>>()
                .unwrap_or_else(|error| {
                    panic!("font family {family:?} is not a CSS family list: {error:?}")
                })
        },
    );
    let mut names = Vec::with_capacity(own.len() + 1);
    names.push(parley::FontFamilyName::Generic(
        parley::GenericFamily::Emoji,
    ));
    names.extend(own);
    parley::FontFamily::List(std::borrow::Cow::Owned(names))
}

/// The byte ranges of `text` that present as emoji, one per grapheme
/// cluster.
fn emoji_cluster_ranges(text: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let emoji = CodePointSetData::new::<Emoji>();
    let emoji_presentation = CodePointSetData::new::<EmojiPresentation>();
    text.grapheme_indices(true)
        .filter(move |(_, cluster)| is_emoji_cluster(cluster, emoji, emoji_presentation))
        .map(|(start, cluster)| start..start + cluster.len())
}

/// Whether a grapheme cluster presents as emoji: it carries an
/// `Emoji_Presentation=Yes` codepoint, or an `Emoji=Yes` base followed by
/// U+FE0F. U+FE0E following a base requests text presentation and keeps the
/// cluster off this path.
fn is_emoji_cluster(
    cluster: &str,
    emoji: CodePointSetDataBorrowed<'static>,
    emoji_presentation: CodePointSetDataBorrowed<'static>,
) -> bool {
    let mut chars = cluster.chars().peekable();
    while let Some(c) = chars.next() {
        if matches!(chars.peek(), Some('\u{FE0E}')) {
            continue;
        }
        if emoji_presentation.contains(c)
            || (matches!(chars.peek(), Some('\u{FE0F}')) && emoji.contains(c))
        {
            return true;
        }
    }
    false
}

fn text_layout_locale(env: &Environment) -> String {
    waterui_locale::locale_binding(env)
        .snapshot()
        .canonical_tag()
}

/// The marker a truncated line's tail is cut for.
const TAIL_ELLIPSIS: char = '\u{2026}';

/// How a drawn text treats the tail that a line limit cuts off.
#[derive(Clone, Copy)]
pub(crate) enum TailMark {
    /// No line limit — every laid-out line draws.
    None,
    /// At most the given lines draw; the rest are clipped.
    Clip(usize),
    /// At most the given lines draw and the last carries a trailing
    /// ellipsis — what a `Text` leaf's `line_limit` means.
    Ellipsis(usize),
}

impl TailMark {
    pub(crate) const fn parts(self) -> (Option<usize>, bool) {
        match self {
            Self::None => (None, false),
            Self::Clip(limit) => (Some(limit), false),
            Self::Ellipsis(limit) => (Some(limit), true),
        }
    }
}

/// Whether the laid-out text needs its tail truncated: more lines than the
/// limit allows, or a last visible line that overruns the bound — which is
/// how a single unbreakable cluster presents. Lines a plain wrap produced
/// stay inside the bound, so a small epsilon keeps benign rounding from
/// declaring a truncation.
fn needs_tail_truncation(layout: &parley::Layout<[u8; 4]>, limit: usize) -> bool {
    if layout.is_empty() {
        return false;
    }
    if layout.len() > limit {
        return true;
    }
    layout
        .get(layout.len() - 1)
        .is_some_and(|line| line.metrics().advance - layout.layout_max_advance() > 0.01)
}

/// The text and styled spans a tail cut leaves for the re-shape.
type RespelledTail = (String, Vec<(Range<usize>, ResolvedTextStyleSpec)>);

/// The `text`/`spans` pair with `text`'s last allowed line shortened to make
/// room for — and end in — the ellipsis marker. `None` when the text is
/// already exactly that, which is how the re-shape loop knows it cannot
/// shorten further.
fn truncate_layout_tail(
    layout: &parley::Layout<[u8; 4]>,
    text: &str,
    spans: &[(Range<usize>, ResolvedTextStyleSpec)],
    limit: usize,
    ellipsis_advance: f32,
) -> Option<RespelledTail> {
    let last_index = layout.len().min(limit) - 1;
    let line = layout.get(last_index)?;
    let bound = layout.layout_max_advance();
    let line_start = line.text_range().start;

    // The tail is every cluster from the last allowed line on: the truncated
    // line refills its bound from content a wrap placed on later lines. Byte
    // order cuts the *logical* tail, which is the edge the ellipsis belongs on
    // for either direction. A hard break ends the kept prefix — the marker
    // replaces the paragraph's tail, never the break itself — and a marker an
    // earlier pass appended is skipped so re-truncating only ever removes
    // real content.
    let marker_start = text.len().saturating_sub(TAIL_ELLIPSIS.len_utf8());
    let has_marker = text.ends_with(TAIL_ELLIPSIS);
    let mut clusters: Vec<(Range<usize>, f32)> = Vec::new();
    'lines: for line_index in last_index..layout.len() {
        let Some(tail_line) = layout.get(line_index) else {
            break;
        };
        for run in tail_line.runs() {
            for cluster in run.clusters() {
                if cluster.is_hard_line_break() {
                    break 'lines;
                }
                let range = cluster.text_range();
                if has_marker && range.end > marker_start {
                    continue;
                }
                clusters.push((range, cluster.advance()));
            }
        }
    }
    clusters.sort_by_key(|(range, _)| range.start);

    let mut used = 0.0_f32;
    let mut cut = line_start;
    for (range, advance) in clusters {
        if used + advance + ellipsis_advance > bound {
            break;
        }
        used += advance;
        cut = range.end;
    }

    let kept_end = line_start + text[line_start..cut].trim_end().len();
    let mut truncated = String::with_capacity(kept_end + TAIL_ELLIPSIS.len_utf8());
    truncated.push_str(&text[..line_start]);
    truncated.push_str(&text[line_start..kept_end]);
    truncated.push(TAIL_ELLIPSIS);
    if truncated == text {
        return None;
    }
    let truncated_len = truncated.len();
    Some((truncated, truncate_spans(spans, kept_end, truncated_len)))
}

/// Span ranges for the truncated text: clamped to what the cut kept, then the
/// span covering the cut extended to dress the marker too — the way a native
/// truncation takes the last visible run's style.
fn truncate_spans(
    spans: &[(Range<usize>, ResolvedTextStyleSpec)],
    kept_end: usize,
    truncated_len: usize,
) -> Vec<(Range<usize>, ResolvedTextStyleSpec)> {
    let mut truncated: Vec<_> = spans
        .iter()
        .filter_map(|(range, style)| {
            let start = range.start.min(kept_end);
            let end = range.end.min(kept_end);
            (start < end).then(|| (start..end, style.clone()))
        })
        .collect();
    if let Some((range, _)) = truncated
        .iter_mut()
        .rev()
        .find(|(range, _)| range.end == kept_end)
    {
        range.end = truncated_len;
    }
    truncated
}

/// Inline extent of a layout's painted glyph ink — the union of every drawn
/// glyph's outline bounds translated to its pen position — in the same
/// layout-space coordinates `encode_text_layout` draws into.
///
/// `LineMetrics::advance` and the `inline_*_coord` fields parley derives from
/// it track the pen advance only (see `layout/line_break.rs` where both are
/// set from `line_max_advance`). Glyph outlines are free to overhang the
/// advance: italic terminals, script faces, and the edge glyphs of the
/// Windows default face (Segoe UI) all paint past it on one or both sides, so
/// a frame measured to `advance` alone clips real ink (issue #237). The
/// measure path ([`text_dimensions_from_layout`]) widens the frame to cover
/// this extent and the encode path shifts the glyphs by the same correction,
/// keeping the two in lockstep.
///
/// Returns `None` when no drawn glyph carries a scalable outline (a run that
/// resolves to bitmap- or paint-only glyphs contributes nothing here).
pub(crate) fn layout_ink_extent(
    service: &TextMeasureService,
    layout: &Arc<parley::Layout<[u8; 4]>>,
    max_lines: Option<usize>,
) -> Option<(f32, f32)> {
    let key: LayoutInkExtentKey = (Arc::as_ptr(layout) as usize, max_lines);
    let mut extents = service
        .ink_extents
        .lock()
        .expect("layout ink extent cache mutex must not be poisoned");
    if let Some((_, extent)) = extents.get(&key) {
        return *extent;
    }
    let extent = layout_ink_extent_uncached(service, layout, max_lines);
    extents.put(key, (Arc::clone(layout), extent));
    extent
}

fn layout_ink_extent_uncached(
    service: &TextMeasureService,
    layout: &parley::Layout<[u8; 4]>,
    max_lines: Option<usize>,
) -> Option<(f32, f32)> {
    let mut ink_min = f32::INFINITY;
    let mut ink_max = f32::NEG_INFINITY;
    for (index, line) in layout.lines().enumerate() {
        if max_lines.is_some_and(|limit| index >= limit) {
            break;
        }
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let font = run.font();
            let Ok(font_ref) = skrifa::FontRef::from_index(font.data.data(), font.index) else {
                continue;
            };
            // `GlyphMetrics::bounds` reads the `glyf` bbox without drawing and
            // draws through the outline only when the face needs it (gvar,
            // CFF/CFF2); built once per run so that choice — and the scaled,
            // instance-aware metrics it wraps — is shared by every glyph on
            // the run.
            let coords: Vec<NormalizedCoord> = run
                .normalized_coords()
                .iter()
                .map(|coord| NormalizedCoord::from_bits(*coord))
                .collect();
            let metrics =
                font_ref.glyph_metrics(Size::new(run.font_size()), LocationRef::new(&coords));
            let mut ink_bounds = service
                .ink_bounds
                .lock()
                .expect("glyph ink bounds cache mutex must not be poisoned");
            // `glyph.x` accumulates onto `glyph_run.offset()` exactly as
            // `encode_text_layout` accumulates it, so these bounds land in
            // the same coordinates the painter uses.
            let mut run_x = glyph_run.offset();
            for glyph in glyph_run.glyphs() {
                let x = run_x + glyph.x;
                run_x += glyph.advance;
                if let Some((x_min, x_max)) = glyph_ink_bounds(
                    &mut ink_bounds,
                    &metrics,
                    font,
                    run.font_size(),
                    run.normalized_coords(),
                    glyph.id,
                ) {
                    ink_min = ink_min.min(x + x_min);
                    ink_max = ink_max.max(x + x_max);
                }
            }
        }
    }
    (ink_min <= ink_max).then_some((ink_min, ink_max))
}

/// Compute view dimensions (size plus first/last baselines) from a shaped
/// layout. Pure; safe to call on any thread.
pub(crate) fn text_dimensions_from_layout(
    service: &TextMeasureService,
    layout: &Arc<parley::Layout<[u8; 4]>>,
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

    // Advance is the pen distance; ink may extend past it on either edge.
    // The frame covers `[-ink_min, ink_max]` when ink overhangs so the encode
    // path's matching shift lands the painted extent inside the view rect.
    if let Some((ink_min, ink_max)) = layout_ink_extent(service, layout, max_lines) {
        width = width.max(ink_max) - ink_min.min(0.0);
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

/// Everything that identifies a shaped layout except the width it is fitted to.
///
/// Built once per [`ResolvedTextLayoutInput`] and shared by every width probe of
/// that text: a container measuring one leaf against several proposals then
/// allocates this once instead of once per probe. All fields are owned and
/// `Send`, so the cache can be shared across threads.
#[derive(Debug, Eq)]
struct TextLayoutIdentity {
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
struct TextLayoutAlignmentCacheKey {
    low: u64,
    high: u64,
    right_to_left: bool,
}

impl TextLayoutIdentity {
    fn new(
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

/// Cache key for a shaped [`parley::Layout`]: what to shape, and how wide.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextLayoutCacheKey {
    identity: Arc<TextLayoutIdentity>,
    max_width: Option<u32>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct TextLayoutSpanCacheKey {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) font: TextLayoutFontCacheKey,
    pub(crate) foreground: Option<[u8; 4]>,
    pub(crate) background: Option<[u8; 4]>,
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

#[cfg(test)]
mod truncation_tests {
    use super::*;
    use crate::renderer::tests::test_environment;

    fn test_input(env: &Environment, text: &'static str) -> ResolvedTextLayoutInput {
        resolve_text_layout_input(&StyledStr::plain(text), HorizontalAlignment::Leading, env)
    }

    /// The logically-last cluster's character on a line — where the marker
    /// sits after tail truncation.
    fn last_cluster_char(line: &parley::Line<'_, [u8; 4]>) -> Option<char> {
        let mut tail = None;
        for run in line.runs() {
            for cluster in run.clusters() {
                tail = Some(cluster.source_char());
            }
        }
        tail
    }

    #[test]
    fn a_truncated_line_ends_in_an_ellipsis_inside_its_bound() {
        let env = test_environment();
        let service = TextMeasureService::new();
        let input = test_input(
            &env,
            "a preview long enough that a single line cannot hold it",
        );

        let layout = service.shape_limited(&input, Some(60.0), Some(1));

        assert_eq!(layout.len(), 1, "a one-line limit lays out one line");
        let line = layout.get(0).expect("one line");
        assert!(
            line.metrics().advance <= 60.0,
            "the truncated line stays inside its bound"
        );
        assert!(
            line.metrics().advance > 45.0,
            "the cut fills the bound to glyph granularity"
        );
        assert_eq!(
            last_cluster_char(&line),
            Some(TAIL_ELLIPSIS),
            "the drawn line ends in an ellipsis"
        );
    }

    #[test]
    fn a_multiline_limit_carries_the_ellipsis_on_the_last_line() {
        let env = test_environment();
        let service = TextMeasureService::new();
        let input = test_input(
            &env,
            "a preview long enough that it wraps well past the two lines it may show",
        );

        let layout = service.shape_limited(&input, Some(80.0), Some(2));

        assert_eq!(
            layout.len(),
            2,
            "the truncated text keeps its allowed lines"
        );
        let last = layout.get(1).expect("last line");
        assert!(last.metrics().advance <= 80.0);
        assert_eq!(last_cluster_char(&last), Some(TAIL_ELLIPSIS));
        let first = layout.get(0).expect("first line");
        assert_ne!(
            last_cluster_char(&first),
            Some(TAIL_ELLIPSIS),
            "only the last line carries the marker"
        );
    }

    #[test]
    fn a_text_that_fits_its_limit_is_not_truncated() {
        let env = test_environment();
        let service = TextMeasureService::new();
        let input = test_input(&env, "short");

        let layout = service.shape_limited(&input, Some(200.0), Some(1));

        let line = layout.get(0).expect("one line");
        assert_eq!(last_cluster_char(&line), Some('t'));
    }

    /// The leaf contract (docs/layout-spec.md §6): the answer is the laid-out
    /// width of the truncated line — its kept clusters plus the ellipsis —
    /// never the proposal. The cut packs clusters to glyph granularity, so
    /// the laid-out line sits within one dropped cluster of the bound.
    #[test]
    fn a_truncated_leaf_reports_the_drawn_line() {
        let env = test_environment();
        let mut state = HydroState::default();
        let styled = StyledStr::plain("aaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

        let dimensions = HydrolysisRenderer::measure_text_dimensions(
            &mut state,
            styled,
            HorizontalAlignment::Leading,
            &env,
            Some(100.0),
            Some(1),
        );

        assert!(
            dimensions.size.width <= 100.0,
            "the laid-out line stays inside its bound"
        );
        assert!(
            dimensions.size.width > 85.0,
            "the laid-out line fills the bound to within a cluster"
        );
    }

    /// A tail the line count alone cuts — the kept line never reached the
    /// bound — reports its drawn extent, not the bound.
    #[test]
    fn a_line_count_truncation_reports_the_drawn_extent() {
        let env = test_environment();
        let mut state = HydroState::default();
        let styled = StyledStr::plain("a\nb\nc");

        let dimensions = HydrolysisRenderer::measure_text_dimensions(
            &mut state,
            styled,
            HorizontalAlignment::Leading,
            &env,
            Some(200.0),
            Some(1),
        );

        assert!(
            dimensions.size.width < 200.0,
            "a bound the text never filled is not reported"
        );
        assert!(dimensions.size.width > 0.0);
    }
}

/// Horizontal ink bounds of `glyph` under `metrics` — `(x_min, x_max)`
/// relative to the pen origin — memoized per face, glyph, size and
/// variation instance behind the caller's lock.
fn glyph_ink_bounds(
    cache: &mut LruCache<GlyphInkBoundsKey, GlyphInkBoundsValue>,
    metrics: &GlyphMetrics<'_>,
    font: &parley::FontData,
    size: f32,
    coords: &[i16],
    glyph: u32,
) -> Option<(f32, f32)> {
    let key = GlyphInkBoundsKey {
        font: font.data.id(),
        face: font.index,
        glyph,
        size: size.to_bits(),
        coords: normalized_coords_hash(coords),
    };
    // A hit is trusted only for the exact instance it recorded; on a
    // coordinate-hash collision the fresh bounds replace the entry.
    if let Some((stored_coords, bounds)) = cache.get(&key)
        && stored_coords.as_slice() == coords
    {
        return *bounds;
    }
    let bounds = metrics
        .bounds(GlyphId::new(glyph))
        .map(|bounds| (bounds.x_min, bounds.x_max));
    cache.put(key, (coords.to_vec(), bounds));
    bounds
}
