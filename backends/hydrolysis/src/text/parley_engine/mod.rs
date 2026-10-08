//! The parley text engine: Hydrolysis's text stack built on `parley`,
//! `fontique` and `skrifa`.
//!
//! [`ParleyEngine`] owns the loaded fonts, the shaping scratch and every cache
//! keyed on engine types (the per-layout ink-extent and per-glyph ink-bounds
//! LRUs); [`ParleyLayout`] is the [`TextLayout`]: a ranged parley layout plus
//! the background spans of the input it was shaped from — a truncated layout
//! therefore carries its respelled spans, and no effective-input plumbing
//! survives.

#[cfg(target_os = "android")]
mod android_fonts;
pub mod fonts;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
mod web_fonts;

use core::num::NonZeroUsize;
use core::ops::Range;
use std::sync::{Arc, Mutex};

use icu_properties::props::{Emoji, EmojiPresentation};
use icu_properties::{CodePointSetData, CodePointSetDataBorrowed};
use lru::LruCache;
use skrifa::instance::{LocationRef, NormalizedCoord, Size};
use skrifa::metrics::GlyphMetrics;
use skrifa::{GlyphId, MetadataProvider};
use unicode_segmentation::UnicodeSegmentation;
use waterui_core::layout::HorizontalAlignment;
use waterui_text::FontCollection;
use waterui_text::font::FontWeight as TextFontWeight;

use crate::renderer::{FrameWorkCounters, Glyph, GlyphRun, Recording, rgba8_to_peniko};

use super::engine::{
    Affinity, CARET_WIDTH, FontFamilyResolution, LineMetrics, TextEngine, TextLayout, TextPosition,
    TextSelection,
};
use super::input::{ResolvedTextLayoutInput, ResolvedTextStyleSpec};

/// The [`TextEngine`] built on the parley stack.
///
/// `fonts` is the collection the engine was born with — the session's
/// installed fonts via [`Self::from_collection`], or the enumerated system
/// collection via [`Self::system`]. An engine's fonts never change after
/// construction, so no shape cache keyed on its inputs can go stale.
pub struct ParleyEngine {
    /// Fonts registered at construction; the clone source for shaping scratch.
    fonts: parley::FontContext,
    /// Whether a named family the collection cannot resolve fails the shape.
    /// See [`FontFamilyResolution`].
    family_resolution: FontFamilyResolution,
    /// Reusable `(FontContext, LayoutContext)` shaping scratch, checked out per
    /// shape call. Built once as a clone of `fonts` (carrying the registered
    /// resource fonts) plus a fresh layout context, then returned for the next
    /// call rather than rebuilt.
    scratch: Mutex<Option<TextShapingScratch>>,
    /// Per-layout horizontal ink extent behind the engine's `ink_extent`:
    /// measure and encode ask for the same layout's extent in one frame, and
    /// resize/re-layout replays measurements of layouts the shape cache still
    /// holds — the memo makes steady-state re-measure cost nothing. Entries pin
    /// their layout's `Arc` so the pointer key can never be recycled while the
    /// entry lives.
    ink_extents: Mutex<LruCache<LayoutInkExtentKey, LayoutInkExtentValue>>,
    /// Per-glyph horizontal ink bounds behind `ink_extent`.
    /// `GlyphMetrics::bounds` answers TrueType faces straight from the `glyf`
    /// header table and draws the outline only for faces that need it (gvar,
    /// CFF/CFF2); memoizing per (face, glyph, size, instance) keeps either
    /// path paid once per measure, not once per glyph.
    ink_bounds: Mutex<LruCache<GlyphInkBoundsKey, GlyphInkBoundsValue>>,
}

/// Capacity of the per-layout ink-extent LRU.
const LAYOUT_INK_EXTENT_CACHE_CAPACITY: usize = 1024;

/// Capacity of the per-glyph ink-bounds LRU: one entry per (face, glyph, size,
/// instance) — a few hundred entries cover a dense screen, the bound keeps
/// long-tail documents from growing it forever.
const GLYPH_INK_BOUNDS_CACHE_CAPACITY: usize = 16384;

/// Owned, mutable shaping scratch for one in-flight shape call.
struct TextShapingScratch {
    font_cx: parley::FontContext,
    layout_cx: parley::LayoutContext<[u8; 4]>,
}

/// The laid-out text this engine hands around: a shared parley layout plus the
/// `(byte range, colour)` pairs of the spans the input painted a background
/// on. The ranges already belong to the text the layout carries — a truncated
/// layout keeps the respelled ranges of the respelled input.
#[derive(Clone)]
pub struct ParleyLayout(Arc<ParleyLayoutData>);

struct ParleyLayoutData {
    layout: parley::Layout<[u8; 4]>,
    backgrounds: Box<[(Range<usize>, [u8; 4])]>,
}

impl ParleyEngine {
    /// An engine shaping against the session's installed font collection —
    /// the system's fonts plus the resource fonts the collection carries.
    pub(crate) fn from_collection(
        fonts: &FontCollection,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self::new(fonts.use_fonts(|fonts| fonts.clone()), family_resolution)
    }

    /// An engine shaping against the enumerated system collection — exactly
    /// the fonts the text service shaped with before the seam.
    pub(crate) fn system(family_resolution: FontFamilyResolution) -> Self {
        Self::new(parley::FontContext::new(), family_resolution)
    }

    fn new(fonts: parley::FontContext, family_resolution: FontFamilyResolution) -> Self {
        Self {
            fonts,
            family_resolution,
            scratch: Mutex::new(None),
            ink_extents: Mutex::new(LruCache::new(
                NonZeroUsize::new(LAYOUT_INK_EXTENT_CACHE_CAPACITY)
                    .expect("layout ink extent cache capacity must be non-zero"),
            )),
            ink_bounds: Mutex::new(LruCache::new(
                NonZeroUsize::new(GLYPH_INK_BOUNDS_CACHE_CAPACITY)
                    .expect("glyph ink bounds cache capacity must be non-zero"),
            )),
        }
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

impl TextEngine for ParleyEngine {
    type Layout = ParleyLayout;

    fn empty_layout(&self) -> Self::Layout {
        ParleyLayout(Arc::new(ParleyLayoutData {
            layout: parley::Layout::new(),
            backgrounds: Box::default(),
        }))
    }

    fn shape(&self, input: &ResolvedTextLayoutInput, max_width: Option<f32>) -> Self::Layout {
        let mut scratch = self.checkout_scratch();
        let layout = build_parley_layout(&mut scratch, input, max_width, self.family_resolution);
        self.return_scratch(scratch);
        ParleyLayout(Arc::new(ParleyLayoutData {
            layout,
            backgrounds: input.background_spans(),
        }))
    }

    fn truncate_tail(
        &self,
        layout: Self::Layout,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: NonZeroUsize,
        mut shape: impl FnMut(&ResolvedTextLayoutInput, Option<f32>) -> Self::Layout,
    ) -> Self::Layout {
        let limit = max_lines.get();
        let mut layout = layout;
        if !needs_tail_truncation(&layout.0.layout, limit) {
            return layout;
        }

        let ellipsis_advance = ellipsis_advance(input, &layout.0.layout, limit, &mut shape);
        let mut text = input.plain.clone();
        let mut spans = input.spans.clone();
        while let Some(cut) =
            truncate_layout_tail(&layout.0.layout, &text, &spans, limit, ellipsis_advance)
        {
            if cut.0 == text {
                break;
            }
            text = cut.0;
            spans = cut.1;
            let next = input.respell(text.clone(), spans.clone());
            layout = shape(&next, max_width);
            if !needs_tail_truncation(&layout.0.layout, limit) {
                break;
            }
        }
        layout
    }

    fn ink_extent(&self, layout: &Self::Layout, max_lines: Option<usize>) -> Option<(f32, f32)> {
        let key: LayoutInkExtentKey = (Arc::as_ptr(&layout.0) as usize, max_lines);
        let mut extents = self
            .ink_extents
            .lock()
            .expect("layout ink extent cache mutex must not be poisoned");
        if let Some((_, extent)) = extents.get(&key) {
            return *extent;
        }
        let extent = self.ink_extent_uncached(layout, max_lines);
        extents.put(key, (layout.clone(), extent));
        extent
    }

    fn record(
        &self,
        layout: &Self::Layout,
        max_lines: Option<usize>,
        x_shift: f32,
        scene: &mut Recording,
        counters: &mut FrameWorkCounters,
    ) {
        encode_text_layout(scene, counters, layout, max_lines, x_shift);
    }
}

impl ParleyEngine {
    /// The uncached walk behind `ink_extent`: the union of every drawn glyph's
    /// outline bounds translated to its pen position, in layout space.
    fn ink_extent_uncached(
        &self,
        layout: &ParleyLayout,
        max_lines: Option<usize>,
    ) -> Option<(f32, f32)> {
        let mut ink_min = f32::INFINITY;
        let mut ink_max = f32::NEG_INFINITY;
        for (index, line) in layout.0.layout.lines().enumerate() {
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
                // `GlyphMetrics::bounds` reads the `glyf` bbox without drawing
                // and draws through the outline only when the face needs it
                // (gvar, CFF/CFF2); built once per run so that choice — and the
                // scaled, instance-aware metrics it wraps — is shared by every
                // glyph on the run.
                let coords: Vec<NormalizedCoord> = run
                    .normalized_coords()
                    .iter()
                    .map(|coord| NormalizedCoord::from_bits(*coord))
                    .collect();
                let metrics =
                    font_ref.glyph_metrics(Size::new(run.font_size()), LocationRef::new(&coords));
                let mut ink_bounds = self
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
}

/// The marker a truncated line's tail is cut for.
const TAIL_ELLIPSIS: char = '\u{2026}';

/// The marker's advance in the style at the tail of the last allowed line, so
/// the truncation cut can reserve room for it.
fn ellipsis_advance(
    input: &ResolvedTextLayoutInput,
    layout: &parley::Layout<[u8; 4]>,
    limit: usize,
    shape: &mut impl FnMut(&ResolvedTextLayoutInput, Option<f32>) -> ParleyLayout,
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
    shape(&input.respell(String::from(TAIL_ELLIPSIS), spans), None)
        .0
        .layout
        .get(0)
        .map_or(0.0, |line| line.metrics().advance)
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

fn build_parley_layout(
    scratch: &mut TextShapingScratch,
    input: &ResolvedTextLayoutInput,
    max_width: Option<f32>,
    family_resolution: FontFamilyResolution,
) -> parley::Layout<[u8; 4]> {
    if family_resolution.is_strict() {
        let collection = &mut scratch.font_cx.collection;
        assert_family_list_installed(collection, input.default_font.family.as_deref());
        for (_, style) in &input.spans {
            assert_family_list_installed(collection, style.font.family.as_deref());
        }
    }
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

/// The [`FontFamilyResolution::Strict`] gate on one CSS `font-family` list:
/// every *named* entry must resolve in the collection — generic families
/// always resolve, so they are skipped, as is the generic fallback a missing
/// family list defaults to. A named family that is neither installed on the
/// host nor provided by a declared font file is a missing declaration;
/// panic naming it rather than measuring a substitute face.
fn assert_family_list_installed(
    collection: &mut parley::fontique::Collection,
    family: Option<&str>,
) {
    let Some(family) = family else {
        return;
    };
    for name in parley::FontFamilyName::parse_css_list(family) {
        let name = name.unwrap_or_else(|error| {
            panic!("font family {family:?} is not a CSS family list: {error:?}")
        });
        if let parley::FontFamilyName::Named(name) = name
            && collection.family_by_name(&name).is_none()
        {
            panic!(
                "font family `{name}` is neither installed on this host nor provided by a \
                 declared font file; declare it with `local_path` under \
                 `[[package.metadata.waterui.assets.font]]` in the crate that names it (fonts \
                 declared by registry name or `remote_path` are staged only by the `water` CLI)"
            );
        }
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

/// Encode `layout`'s glyph runs into `scene` at the local origin, shifted
/// right by `x_shift`. The caller positions the result by appending it under
/// a transform, which is what makes the encoded fragment reusable across
/// frames.
fn encode_text_layout(
    scene: &mut Recording,
    counters: &mut FrameWorkCounters,
    layout: &ParleyLayout,
    max_lines: Option<usize>,
    x_shift: f32,
) {
    let paint_backgrounds = !layout.0.backgrounds.is_empty();
    for (index, line) in layout.0.layout.lines().enumerate() {
        if max_lines.is_some_and(|limit| index >= limit) {
            break;
        }
        if paint_backgrounds {
            encode_line_backgrounds(scene, &line, layout, x_shift);
        }
        for item in line.items() {
            if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                let run = glyph_run.run();
                let style = glyph_run.style();
                let brush = rgba8_to_peniko(style.brush);
                let normalized_coords = run.normalized_coords();

                let mut run_x = glyph_run.offset() + x_shift;
                let run_y = glyph_run.baseline();
                let glyphs: Vec<crate::renderer::Glyph> = glyph_run
                    .glyphs()
                    .map(move |glyph| {
                        let x = run_x + glyph.x;
                        let y = run_y - glyph.y;
                        run_x += glyph.advance;
                        Glyph { id: glyph.id, x, y }
                    })
                    .collect();

                counters.font_registrations += 1;
                scene.glyphs(&GlyphRun {
                    font: run.font(),
                    font_size: run.font_size(),
                    normalized_coords,
                    transform: kurbo::Affine::IDENTITY,
                    brush: &peniko::Brush::Solid(brush),
                    brush_alpha: 1.0,
                    style: peniko::StyleRef::Fill(peniko::Fill::NonZero),
                    glyphs: &glyphs,
                });
            }
        }
    }
}

/// Fill each backgrounded span's glyph extent on `line` — the full line
/// box (`block_min_coord..block_max_coord`) tall — under the text.
///
/// The horizontal cursor accumulates cluster advances over `runs()` in
/// display order, the same sequence parley's own glyph-run iterator places
/// left-to-right (both are driven by `Run::visual_clusters`). These layouts
/// come from a ranged builder, which emits no inline boxes, so runs are
/// the whole item sequence.
fn encode_line_backgrounds(
    scene: &mut Recording,
    line: &parley::Line<'_, [u8; 4]>,
    layout: &ParleyLayout,
    x_shift: f32,
) {
    let metrics = line.metrics();
    let (top, bottom) = (
        f64::from(metrics.block_min_coord),
        f64::from(metrics.block_max_coord),
    );
    let mut cursor = metrics.inline_min_coord + metrics.offset + x_shift;
    // Adjacent clusters with the same background merge into one fill.
    let mut open: Option<(f32, [u8; 4])> = None;
    for run in line.runs() {
        for cluster in run.visual_clusters() {
            let end = cursor + cluster.advance();
            let background = layout
                .0
                .backgrounds
                .iter()
                .find(|(range, _)| range.contains(&cluster.text_range().start))
                .map(|(_, colour)| *colour);
            let extends = matches!(
                (open, background),
                (Some((_, open_colour)), Some(colour)) if open_colour == colour
            );
            if !extends {
                if let Some((start, colour)) = open.take() {
                    fill_span_background(scene, start, cursor, top, bottom, colour);
                }
                open = background.map(|colour| (cursor, colour));
            }
            cursor = end;
        }
    }
    if let Some((start, colour)) = open {
        fill_span_background(scene, start, cursor, top, bottom, colour);
    }
}

fn fill_span_background(
    scene: &mut Recording,
    start: f32,
    end: f32,
    top: f64,
    bottom: f64,
    colour: [u8; 4],
) {
    scene.fill(
        peniko::Fill::NonZero,
        kurbo::Affine::IDENTITY,
        &peniko::Brush::Solid(rgba8_to_peniko(colour)),
        None,
        &kurbo::Rect::new(f64::from(start), top, f64::from(end), bottom),
    );
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

/// Cache identity for one layout's ink extent: the layout's shared address —
/// the cached value pins the `Arc`, so the address is not recycled while the
/// entry lives — plus the `max_lines` truncation the extent was measured at.
type LayoutInkExtentKey = (usize, Option<usize>);

/// Value side of the layout ink-extent cache: the layout keeping the key's
/// address live, then the extent.
type LayoutInkExtentValue = (ParleyLayout, Option<(f32, f32)>);

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

/// Seam ⟷ parley mapping: affinities, positions, selections.
const fn to_parley(affinity: Affinity) -> parley::Affinity {
    match affinity {
        Affinity::Downstream => parley::Affinity::Downstream,
        Affinity::Upstream => parley::Affinity::Upstream,
    }
}

const fn from_parley(affinity: parley::Affinity) -> Affinity {
    match affinity {
        parley::Affinity::Downstream => Affinity::Downstream,
        parley::Affinity::Upstream => Affinity::Upstream,
    }
}

fn cursor(layout: &parley::Layout<[u8; 4]>, at: TextPosition) -> parley::Cursor {
    parley::Cursor::from_byte_index(layout, at.index, to_parley(at.affinity))
}

fn position(at: parley::Cursor) -> TextPosition {
    TextPosition {
        index: at.index(),
        affinity: from_parley(at.affinity()),
    }
}

fn to_parley_selection(
    layout: &parley::Layout<[u8; 4]>,
    selection: TextSelection,
) -> parley::Selection {
    parley::Selection::new(
        cursor(layout, selection.anchor),
        cursor(layout, selection.focus),
    )
}

fn from_parley_selection(selection: parley::Selection) -> TextSelection {
    TextSelection {
        anchor: position(selection.anchor()),
        focus: position(selection.focus()),
    }
}

impl TextLayout for ParleyLayout {
    fn line_count(&self) -> usize {
        self.0.layout.len()
    }

    fn line_metrics(&self, line: usize) -> LineMetrics {
        let line = self.0.layout.get(line).unwrap_or_else(|| {
            panic!(
                "line {line} is out of bounds for a layout with {} lines",
                self.0.layout.len()
            )
        });
        let metrics = line.metrics();
        LineMetrics {
            advance: metrics.advance,
            line_height: metrics.line_height,
            baseline: metrics.baseline,
        }
    }

    fn height(&self) -> f32 {
        self.0.layout.height()
    }

    fn selection(&self, anchor: TextPosition, focus: TextPosition) -> TextSelection {
        from_parley_selection(
            to_parley_selection(&self.0.layout, TextSelection { anchor, focus })
                .refresh(&self.0.layout),
        )
    }

    fn hit_test(&self, x: f32, y: f32) -> TextPosition {
        position(
            parley::Selection::from_point(&self.0.layout, x, y)
                .refresh(&self.0.layout)
                .focus(),
        )
    }

    fn word_at(&self, x: f32, y: f32) -> TextSelection {
        from_parley_selection(
            parley::Selection::word_from_point(&self.0.layout, x, y).refresh(&self.0.layout),
        )
    }

    fn line_at(&self, x: f32, y: f32) -> TextSelection {
        from_parley_selection(
            parley::Selection::line_from_point(&self.0.layout, x, y).refresh(&self.0.layout),
        )
    }

    fn caret_rect(&self, at: TextPosition) -> kurbo::Rect {
        let rect = cursor(&self.0.layout, at).geometry(&self.0.layout, CARET_WIDTH);
        kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1)
    }

    fn selection_rects(&self, selection: TextSelection, mut each: impl FnMut(kurbo::Rect)) {
        to_parley_selection(&self.0.layout, selection).geometry_with(&self.0.layout, |rect, _| {
            each(kurbo::Rect::new(rect.x0, rect.y0, rect.x1, rect.y1));
        });
    }

    fn previous_visual(&self, selection: TextSelection, extend: bool) -> TextSelection {
        from_parley_selection(
            to_parley_selection(&self.0.layout, selection).previous_visual(&self.0.layout, extend),
        )
    }

    fn next_visual(&self, selection: TextSelection, extend: bool) -> TextSelection {
        from_parley_selection(
            to_parley_selection(&self.0.layout, selection).next_visual(&self.0.layout, extend),
        )
    }
}

const fn parley_font_weight(weight: TextFontWeight) -> parley::FontWeight {
    let value = match weight {
        TextFontWeight::Thin => 100.0,
        TextFontWeight::UltraLight => 200.0,
        TextFontWeight::Light => 300.0,
        TextFontWeight::Normal => 400.0,
        TextFontWeight::Medium => 500.0,
        TextFontWeight::SemiBold => 600.0,
        TextFontWeight::Bold => 700.0,
        TextFontWeight::UltraBold => 800.0,
        TextFontWeight::Black => 900.0,
    };
    parley::FontWeight::new(value)
}

fn parley_alignment(alignment: HorizontalAlignment, right_to_left: bool) -> parley::Alignment {
    if alignment == HorizontalAlignment::Leading && right_to_left
        || alignment == HorizontalAlignment::Trailing && !right_to_left
    {
        parley::Alignment::Right
    } else if alignment == HorizontalAlignment::Leading
        || alignment == HorizontalAlignment::Trailing
    {
        parley::Alignment::Left
    } else {
        parley::Alignment::Center
    }
}

#[cfg(test)]
mod tests;
