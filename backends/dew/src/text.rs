//! Text shaping and rasterization: parley layout → retained glyph runs.
//!
//! Dew shares the text stack with hydrolysis: [`parley`] shapes and lays out
//! text, and the resulting positioned glyph runs are retained as
//! [`DrawCommand::GlyphRun`]s that the painter rasterizes through
//! `vello_cpu`'s glyph pipeline. Fonts come from the system collection on
//! desktop; embedded targets register bundled fonts into the same
//! [`parley::FontContext`].
//!
//! Styled text is shaped per span: every [`StyledStr`] chunk pushes its
//! resolved font size, weight, family, slant, decorations, and foreground
//! color as parley range styles, so `.title()`, `.bold()`, or a per-span
//! color produce visibly distinct glyph runs. Measurement reuses the same
//! styled layout, keeping layout and rasterization byte-identical.

use kurbo::{Affine, Rect};
use nami::Signal;
use skrifa::prelude::{FontRef, GlyphId, LocationRef, MetadataProvider, Size};
use waterui_core::Environment;
use waterui_graphics::color::ResolvedColor;
use waterui_text::font::{Font, FontWeight, ResolvedFont};
use waterui_text::styled::{Style, StyledStr};

use crate::display_list::{DisplayList, DrawCommand};
use crate::stats::FrameWork;
use crate::theme;

/// Shared text-shaping state: the font collection and parley's scratch
/// layout context.
///
/// One per renderer; rebuilding it is expensive (font enumeration).
pub struct DewState {
    font_cx: parley::FontContext,
    layout_cx: parley::LayoutContext<[u8; 4]>,
    /// Measure-side work performed this frame.
    ///
    /// Measurement runs behind `&self` — a `SubView` cannot mutate the tree —
    /// so the counters live in the one piece of shared state every measure
    /// call already reaches mutably. [`crate::dispatch::DewRenderer`] drains
    /// them into the frame's totals.
    pub(crate) work: FrameWork,
}

/// Whether a cache lookup was answered from the cache or had to build.
///
/// Returned rather than counted internally because the caller holds the
/// `DewState` the counters live in, and the build closure borrows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheOutcome {
    /// The entry already existed.
    Reused,
    /// The entry was built by this call.
    Built,
}

/// Per-text-node layout cache keyed by signal revision and width proposal.
///
/// Dew confines layout to the main render thread, so the cache stays with the
/// retained node instead of adding synchronization or global state.
#[derive(Debug, Default)]
pub(crate) struct TextLayoutCache {
    revision: u64,
    entries: Vec<CachedTextLayout>,
}

/// What a cached layout depends on.
///
/// The width proposal changes the line breaking; the default brush is baked
/// into the shaped runs, and controls vary it (a disabled button's label is
/// painted in a muted colour), so a layout cached for one brush cannot be
/// replayed for another.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextLayoutKey {
    /// Width proposal the text was laid out against.
    pub(crate) max_width: Option<f32>,
    /// Default brush spans without an explicit colour were shaped with.
    pub(crate) brush: peniko::Color,
}

impl TextLayoutKey {
    /// A key for text painted in the theme foreground.
    pub(crate) const fn foreground(max_width: Option<f32>) -> Self {
        Self {
            max_width,
            brush: theme::FOREGROUND,
        }
    }
}

#[derive(Debug)]
struct CachedTextLayout {
    key: TextLayoutKey,
    layout: parley::Layout<[u8; 4]>,
    /// Glyph runs derived from `layout`, in layout-local coordinates.
    ///
    /// Built on first emit and reused afterwards. Deriving them costs one
    /// `skrifa` outline-bounds lookup per glyph, which parses `glyf`/`loca`
    /// from font data that lives in flash on an embedded target — by far the
    /// largest per-frame cost Dew had, and pure waste for text that has not
    /// changed. Placement is *not* baked in: only the transform differs
    /// between frames, and substituting it is free.
    runs: Option<Vec<RetainedGlyphRun>>,
}

/// One shaped glyph run held in layout-local coordinates, ready to be placed.
#[derive(Debug, Clone)]
struct RetainedGlyphRun {
    /// The run with an identity transform; emitting substitutes the real one.
    command: DrawCommand,
    /// Ink bounds of the run before placement.
    local_bounds: Rect,
}

impl TextLayoutCache {
    /// The index of the entry for `max_width`, building it when absent.
    fn entry(
        &mut self,
        revision: u64,
        key: TextLayoutKey,
        build: impl FnOnce() -> parley::Layout<[u8; 4]>,
    ) -> (usize, CacheOutcome) {
        if self.revision != revision {
            self.entries.clear();
            self.revision = revision;
        }
        let mut outcome = CacheOutcome::Reused;
        let index = self
            .entries
            .iter()
            .position(|entry| entry.key == key)
            .unwrap_or_else(|| {
                outcome = CacheOutcome::Built;
                self.entries.push(CachedTextLayout {
                    key,
                    layout: build(),
                    runs: None,
                });
                self.entries.len() - 1
            });
        (index, outcome)
    }

    pub(crate) fn get_or_build(
        &mut self,
        revision: u64,
        key: TextLayoutKey,
        build: impl FnOnce() -> parley::Layout<[u8; 4]>,
    ) -> (&parley::Layout<[u8; 4]>, CacheOutcome) {
        let (index, outcome) = self.entry(revision, key, build);
        (&self.entries[index].layout, outcome)
    }

    /// Appends this text's glyph runs to `list`, placed at `transform`.
    ///
    /// Derives the runs from the cached layout on first use and replays them
    /// on every later frame, so unchanged text costs no outline lookups.
    pub(crate) fn emit(
        &mut self,
        revision: u64,
        key: TextLayoutKey,
        transform: Affine,
        list: &mut DisplayList,
        build: impl FnOnce() -> parley::Layout<[u8; 4]>,
    ) -> CacheOutcome {
        let (index, outcome) = self.entry(revision, key, build);
        let entry = &mut self.entries[index];
        let runs = match &entry.runs {
            Some(runs) => {
                list.add_work(FrameWork {
                    glyph_runs_reused: runs.len() as u64,
                    ..FrameWork::ZERO
                });
                runs
            }
            None => entry.runs.insert(retain_glyph_runs(&entry.layout, list)),
        };
        for run in runs {
            let mut command = run.command.clone();
            command.set_transform(transform);
            list.push_placed(command, transform.transform_rect_bbox(run.local_bounds));
        }
        outcome
    }
}

impl core::fmt::Debug for DewState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DewState").finish_non_exhaustive()
    }
}

impl Default for DewState {
    fn default() -> Self {
        Self {
            font_cx: parley::FontContext::new(),
            layout_cx: parley::LayoutContext::new(),
            work: FrameWork::ZERO,
        }
    }
}

impl DewState {
    /// Takes the work accumulated since the previous call.
    pub(crate) const fn take_work(&mut self) -> FrameWork {
        core::mem::replace(&mut self.work, FrameWork::ZERO)
    }

    /// Records that a text layout was shaped or served from cache.
    pub(crate) const fn record_layout(&mut self, outcome: CacheOutcome) {
        match outcome {
            CacheOutcome::Built => self.work.text_layouts_shaped += 1,
            CacheOutcome::Reused => self.work.text_layouts_reused += 1,
        }
    }
}

/// Font size for plain [`waterui_core::Str`] leaves in logical pixels,
/// matching the [`waterui_text::font::Body`] preset default.
const PLAIN_FONT_SIZE: f32 = 16.0;

impl DewState {
    /// Shapes a plain string with the default body style — the fast path
    /// for bare [`waterui_core::Str`] leaves that carry no span styling.
    pub(crate) fn build_plain_layout(
        &mut self,
        text: &str,
        max_width: Option<f32>,
    ) -> parley::Layout<[u8; 4]> {
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::Brush(peniko_to_rgba8(
            theme::FOREGROUND,
        )));
        builder.push_default(parley::StyleProperty::FontSize(PLAIN_FONT_SIZE));
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout.align(
            parley::Alignment::Start,
            parley::AlignmentOptions::default(),
        );
        layout
    }

    /// Shapes styled text, pushing one parley range style per chunk.
    ///
    /// `default_brush` paints spans without an explicit foreground (theme
    /// foreground for content text, muted for placeholders); span fonts and
    /// colors resolve through `env` so installed theme fonts apply.
    pub(crate) fn build_styled_layout(
        &mut self,
        styled: &StyledStr,
        env: &Environment,
        max_width: Option<f32>,
        default_brush: peniko::Color,
    ) -> parley::Layout<[u8; 4]> {
        let mut plain = String::new();
        let mut spans = Vec::with_capacity(styled.chunks().len());
        for (chunk, style) in styled.chunks() {
            let start = plain.len();
            plain.push_str(chunk.as_str());
            spans.push((start..plain.len(), style));
        }
        if plain.is_empty() {
            return parley::Layout::new();
        }

        let default_font = Font::default().resolve(env).get();
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, &plain, 1.0, true);
        builder.push_default(parley::StyleProperty::Brush(peniko_to_rgba8(default_brush)));
        builder.push_default(parley::StyleProperty::FontSize(default_font.size));
        builder.push_default(parley::StyleProperty::FontWeight(parley_font_weight(
            default_font.weight,
        )));
        builder.push_default(parley::StyleProperty::FontFamily(font_family(
            default_font.family.as_deref(),
        )));

        for (range, style) in spans {
            push_span_style(&mut builder, style, env, range);
        }

        let mut layout = builder.build(&plain);
        layout.break_all_lines(max_width);
        layout.align(
            parley::Alignment::Start,
            parley::AlignmentOptions::default(),
        );
        layout
    }

    /// Measures styled text through the same span-styled layout used for
    /// rendering, returning the laid-out size in logical pixels.
    pub(crate) fn measure_styled(
        &mut self,
        styled: &StyledStr,
        env: &Environment,
        max_width: Option<f32>,
    ) -> (f32, f32) {
        let layout = self.build_styled_layout(styled, env, max_width, theme::FOREGROUND);
        (layout.width(), layout.height())
    }
}

/// Pushes one [`StyledStr`] chunk's resolved style as parley range styles.
fn push_span_style(
    builder: &mut parley::RangedBuilder<'_, [u8; 4]>,
    style: &Style,
    env: &Environment,
    range: core::ops::Range<usize>,
) {
    let font: ResolvedFont = style.font.resolve(env).get();
    builder.push(parley::StyleProperty::FontSize(font.size), range.clone());
    builder.push(
        parley::StyleProperty::FontWeight(parley_font_weight(font.weight)),
        range.clone(),
    );
    if let Some(family) = &font.family {
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
    if let Some(color) = &style.foreground {
        let resolved: ResolvedColor = color.resolve(env).get();
        builder.push(
            parley::StyleProperty::Brush(resolved_color_to_rgba8(&resolved)),
            range,
        );
    }
}

const fn parley_font_weight(weight: FontWeight) -> parley::FontWeight {
    parley::FontWeight::new(match weight {
        FontWeight::Thin => 100.0,
        FontWeight::UltraLight => 200.0,
        FontWeight::Light => 300.0,
        FontWeight::Normal => 400.0,
        FontWeight::Medium => 500.0,
        FontWeight::SemiBold => 600.0,
        FontWeight::Bold => 700.0,
        FontWeight::UltraBold => 800.0,
        FontWeight::Black => 900.0,
    })
}

fn font_family(family: Option<&str>) -> parley::FontFamily<'static> {
    family.map_or_else(
        || parley::style::GenericFamily::SansSerif.into(),
        |family| parley::FontFamily::Source(std::borrow::Cow::Owned(family.to_string())),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "color channels are clamped to [0, 1] before scaling to u8"
)]
fn resolved_color_to_rgba8(color: &ResolvedColor) -> [u8; 4] {
    let srgb = color.to_srgb_with_headroom();
    [
        (srgb.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (srgb.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

/// Converts a theme constant into parley's brush representation.
pub(crate) fn peniko_to_rgba8(color: peniko::Color) -> [u8; 4] {
    let rgba = color.to_rgba8();
    [rgba.r, rgba.g, rgba.b, rgba.a]
}

/// Emits `layout`'s glyph runs at `transform` without retaining them.
///
/// The uncached path, for callers that do not own a layout cache. Every call
/// re-reads one outline bound per glyph, so a caller on a per-frame path
/// should hold a [`TextLayoutCache`] and use [`TextLayoutCache::emit`]
/// instead.
pub(crate) fn emit_text_commands(
    list: &mut DisplayList,
    layout: &parley::Layout<[u8; 4]>,
    transform: Affine,
) {
    for run in retain_glyph_runs(layout, list) {
        let mut command = run.command;
        command.set_transform(transform);
        list.push_placed(command, transform.transform_rect_bbox(run.local_bounds));
    }
}

/// Derives one retained glyph run per positioned run in `layout`, in
/// layout-local coordinates.
///
/// This is where the per-glyph `skrifa` outline-bounds lookups happen, so it
/// runs once per (text, width) rather than once per frame; `list` receives the
/// work accounting.
fn retain_glyph_runs(
    layout: &parley::Layout<[u8; 4]>,
    list: &mut DisplayList,
) -> Vec<RetainedGlyphRun> {
    let mut retained = Vec::new();
    let transform = Affine::IDENTITY;
    let layout_bounds = Rect::new(
        0.0,
        0.0,
        f64::from(layout.width()),
        f64::from(layout.height()),
    );
    for line in layout.lines() {
        for item in line.items() {
            let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                continue;
            };
            let run = glyph_run.run();
            let [red, green, blue, alpha] = glyph_run.style().brush;
            let mut run_x = glyph_run.offset();
            let run_y = glyph_run.baseline();
            let font = run.font().clone();
            let font_size = run.font_size();
            let font_ref = FontRef::from_index(font.data.as_ref(), font.index)
                .expect("Dew text layout produced invalid font data");
            let metrics = font_ref.glyph_metrics(Size::new(font_size), LocationRef::default());
            let (glyphs, glyph_bounds): (Vec<_>, Vec<_>) = glyph_run
                .glyphs()
                .map(|glyph| {
                    let x = run_x + glyph.x;
                    run_x += glyph.advance;
                    let positioned = vello_cpu::Glyph {
                        id: glyph.id,
                        x,
                        y: run_y - glyph.y,
                    };
                    let bounds =
                        metrics
                            .bounds(GlyphId::new(glyph.id))
                            .map_or(layout_bounds, |bounds| {
                                Rect::new(
                                    f64::from(positioned.x + bounds.x_min),
                                    f64::from(positioned.y - bounds.y_max),
                                    f64::from(positioned.x + bounds.x_max),
                                    f64::from(positioned.y - bounds.y_min),
                                )
                                .inflate(1.0, 1.0)
                            });
                    (positioned, bounds)
                })
                .unzip();
            if glyphs.is_empty() {
                continue;
            }
            // Each glyph above cost one `skrifa` outline-bounds lookup, which
            // parses `glyf`/`loca` from font data that lives in flash on an
            // embedded target. Counting them is how the simulation sees a cost
            // the host barely pays.
            list.add_work(FrameWork {
                glyph_bounds_measured: glyphs.len() as u64,
                ..FrameWork::ZERO
            });
            let local_bounds = glyph_bounds
                .iter()
                .copied()
                .reduce(|current, glyph| current.union(glyph))
                .unwrap_or(layout_bounds);
            retained.push(RetainedGlyphRun {
                command: DrawCommand::GlyphRun {
                    font,
                    font_size,
                    glyphs,
                    glyph_bounds,
                    transform,
                    brush: peniko::Color::from_rgba8(red, green, blue, alpha).into(),
                    bounds: layout_bounds,
                    clip: None,
                },
                local_bounds,
            });
        }
    }
    retained
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui::Plugin as _;
    use waterui::theme::{FontSettings, Theme};
    use waterui_text::font::{FontWeight, ResolvedFont, Subheadline, Title};

    fn test_environment() -> Environment {
        let mut env = Environment::new();
        Theme::new()
            .fonts(
                FontSettings::new()
                    .body(ResolvedFont::new(16.0, FontWeight::Normal))
                    .title(ResolvedFont::new(24.0, FontWeight::Normal))
                    .headline(ResolvedFont::new(22.0, FontWeight::Normal))
                    .subheadline(ResolvedFont::new(20.0, FontWeight::Normal))
                    .caption(ResolvedFont::new(12.0, FontWeight::Normal))
                    .footnote(ResolvedFont::new(11.0, FontWeight::Normal)),
            )
            .install(&mut env);
        env
    }

    fn run_font_sizes(layout: &parley::Layout<[u8; 4]>) -> Vec<f32> {
        let mut sizes = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    sizes.push(glyph_run.run().font_size());
                }
            }
        }
        sizes
    }

    /// `.title()` / `.sub_headline()` spans must shape at their preset font
    /// sizes, visibly distinct from body text.
    #[test]
    fn styled_spans_produce_distinct_font_sizes() {
        let env = test_environment();
        let mut state = DewState::default();
        let mut styled = StyledStr::empty();
        styled.push("Heading", Style::new().font(Title));
        styled.push(" subhead", Style::new().font(Subheadline));
        styled.push(" body", Style::new());

        let layout = state.build_styled_layout(&styled, &env, None, theme::FOREGROUND);
        let sizes = run_font_sizes(&layout);
        assert!(
            sizes.contains(&24.0) && sizes.contains(&20.0) && sizes.contains(&16.0),
            "expected title (24), subheadline (20), and body (16) runs, got {sizes:?}"
        );

        let (_, title_height) = state.measure_styled(&StyledStr::plain("Heading"), &env, None);
        let mut titled = StyledStr::empty();
        titled.push("Heading", Style::new().font(Title));
        let (_, styled_height) = state.measure_styled(&titled, &env, None);
        assert!(
            styled_height > title_height,
            "title-styled text must measure taller than body text \
             ({styled_height} vs {title_height})"
        );
    }

    /// A bold span must shape with a heavier synthesized or real weight than
    /// the surrounding body text, producing a separate glyph run.
    #[test]
    fn bold_span_splits_into_its_own_run() {
        let env = test_environment();
        let mut state = DewState::default();
        let mut styled = StyledStr::empty();
        styled.push("normal ", Style::new());
        styled.push("bold", Style::new().bold());

        let layout = state.build_styled_layout(&styled, &env, None, theme::FOREGROUND);
        let mut runs = 0;
        for line in layout.lines() {
            for item in line.items() {
                if matches!(item, parley::PositionedLayoutItem::GlyphRun(_)) {
                    runs += 1;
                }
            }
        }
        assert!(
            runs >= 2,
            "bold span must not collapse into the normal-weight run"
        );
    }

    /// A per-span foreground color must reach the glyph-run brush.
    #[test]
    fn span_color_reaches_the_brush() {
        use waterui_graphics::color::Color;

        let env = test_environment();
        let mut state = DewState::default();
        let mut styled = StyledStr::empty();
        styled.push("red", Style::new().foreground(Color::srgb(255, 0, 0)));

        let layout = state.build_styled_layout(&styled, &env, None, theme::FOREGROUND);
        let mut brushes = Vec::new();
        for line in layout.lines() {
            for item in line.items() {
                if let parley::PositionedLayoutItem::GlyphRun(glyph_run) = item {
                    brushes.push(glyph_run.style().brush);
                }
            }
        }
        assert_eq!(brushes, vec![[255, 0, 0, 255]]);
    }

    #[test]
    fn text_layout_cache_reuses_width_and_invalidates_revision() {
        use core::cell::Cell;

        let builds = Cell::new(0);
        let mut cache = TextLayoutCache::default();
        let build = || {
            builds.set(builds.get() + 1);
            parley::Layout::new()
        };

        cache.get_or_build(0, TextLayoutKey::foreground(Some(120.0)), build);
        cache.get_or_build(0, TextLayoutKey::foreground(Some(120.0)), build);
        cache.get_or_build(0, TextLayoutKey::foreground(Some(80.0)), build);
        cache.get_or_build(1, TextLayoutKey::foreground(Some(120.0)), build);

        assert_eq!(builds.get(), 3);
    }
}
