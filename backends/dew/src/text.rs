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
    /// Whether the collection holds any face at all. Shaping against an
    /// empty collection silently produces no glyphs, so it fails fast
    /// instead — see [`DewState::assert_has_fonts`].
    has_fonts: bool,
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
///
/// The cache deliberately holds two very different weights of state:
///
/// - a **size** per probed [`TextLayoutKey`] — a dozen bytes, kept for every
///   key, because container layouts probe several widths per pass and only
///   need the answer;
/// - the **glyph runs** for one key — the one the display list is actually
///   showing. A full `parley::Layout` per probed width (the previous design)
///   held clusters, runs, and style tables for layouts that would never be
///   painted; on a 320 KiB-SRAM target that was the difference between
///   fitting and not fitting, and the work simulation's peak-heap gate is
///   what caught it. The `Layout` itself is dropped as soon as runs are
///   derived from it.
#[derive(Debug, Default)]
pub(crate) struct TextLayoutCache {
    revision: u64,
    /// Laid-out size per proposal key, in probe order (a handful per node).
    sizes: Vec<(TextLayoutKey, (f32, f32))>,
    /// Full shaped state for the most recently emitted (or, before the first
    /// emit, most recently built) key.
    retained: Option<RetainedText>,
}

/// The heavyweight half of the cache: one key's shaped output.
#[derive(Debug)]
struct RetainedText {
    key: TextLayoutKey,
    /// The parley layout, kept only until [`RetainedText::runs`] is derived,
    /// then dropped — deriving runs is the sole remaining consumer.
    layout: Option<parley::Layout<[u8; 4]>>,
    /// Glyph runs in layout-local coordinates, replayed every frame.
    runs: Option<Vec<RetainedGlyphRun>>,
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
    pub(crate) const fn new(max_width: Option<f32>, brush: peniko::Color) -> Self {
        Self { max_width, brush }
    }
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
    /// Discards everything when the text's content revision moved on.
    fn sync_revision(&mut self, revision: u64) {
        if self.revision != revision {
            self.sizes.clear();
            self.retained = None;
            self.revision = revision;
        }
    }

    /// The laid-out size for `key`, shaping only on a cache miss.
    ///
    /// A missed measurement keeps its layout for the emit that usually
    /// follows at the same key — unless glyph runs for another key are
    /// already on screen, in which case the layout is measured and dropped
    /// rather than evicting live runs (layout passes probe widths that are
    /// never painted).
    pub(crate) fn measure(
        &mut self,
        revision: u64,
        key: TextLayoutKey,
        max_lines: Option<usize>,
        build: impl FnOnce() -> parley::Layout<[u8; 4]>,
    ) -> ((f32, f32), CacheOutcome) {
        self.sync_revision(revision);
        if let Some((_, size)) = self.sizes.iter().find(|(cached, _)| *cached == key) {
            return (*size, CacheOutcome::Reused);
        }
        let layout = build();
        let size = capped_layout_size(&layout, max_lines);
        self.sizes.push((key, size));
        let displayed_elsewhere = self
            .retained
            .as_ref()
            .is_some_and(|retained| retained.runs.is_some() && retained.key != key);
        if !displayed_elsewhere {
            self.retained = Some(RetainedText {
                key,
                layout: Some(layout),
                runs: None,
            });
        }
        (size, CacheOutcome::Built)
    }

    /// Appends this text's glyph runs to `list`, placed at `transform`.
    ///
    /// Derives the runs from the retained layout on first use — dropping the
    /// layout in the process — and replays them on every later frame, so
    /// unchanged text costs no shaping and no outline lookups.
    pub(crate) fn emit(
        &mut self,
        revision: u64,
        key: TextLayoutKey,
        max_lines: Option<usize>,
        transform: Affine,
        list: &mut DisplayList,
        build: impl FnOnce() -> parley::Layout<[u8; 4]>,
    ) -> CacheOutcome {
        self.sync_revision(revision);
        let mut outcome = CacheOutcome::Reused;
        if !self
            .retained
            .as_ref()
            .is_some_and(|retained| retained.key == key)
        {
            outcome = CacheOutcome::Built;
            let layout = build();
            if !self.sizes.iter().any(|(cached, _)| *cached == key) {
                self.sizes
                    .push((key, capped_layout_size(&layout, max_lines)));
            }
            self.retained = Some(RetainedText {
                key,
                layout: Some(layout),
                runs: None,
            });
        }
        let retained = self
            .retained
            .as_mut()
            .expect("retained text state exists after the key check");
        let runs = if let Some(runs) = &retained.runs {
            list.add_work(FrameWork {
                glyph_runs_reused: runs.len() as u64,
                ..FrameWork::ZERO
            });
            runs
        } else {
            let layout = retained
                .layout
                .take()
                .expect("the parley layout is present until runs are derived");
            // The layout's job ends here: runs replay from now on, and the
            // heavyweight parley structures go back to the heap.
            retained
                .runs
                .insert(retain_glyph_runs(&layout, list, max_lines))
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

/// Desktop-only default: the system font collection.
#[cfg(feature = "system-fonts")]
impl Default for DewState {
    fn default() -> Self {
        Self::new(crate::board::FontSources::System)
    }
}

impl DewState {
    /// Builds the shared shaping state for the given font sources.
    ///
    /// [`FontSources::Bundled`] registers each binary into a collection with
    /// system enumeration disabled — the firmware configuration, identical on
    /// host and target — and routes every CSS generic family to the
    /// registered faces so unstyled text resolves. Fallback coverage is the
    /// bundle's responsibility: a glyph the bundled fonts do not cover has
    /// nothing to fall back to, which is the documented asymmetry of an
    /// embedded target, not a defect.
    ///
    /// [`FontSources`]: crate::board::FontSources
    pub(crate) fn new(sources: crate::board::FontSources) -> Self {
        use parley::fontique;

        let (collection, has_fonts) = match sources {
            #[cfg(feature = "system-fonts")]
            crate::board::FontSources::System => (
                fontique::Collection::new(fontique::CollectionOptions {
                    shared: false,
                    system_fonts: true,
                }),
                true,
            ),
            crate::board::FontSources::Bundled(fonts) => {
                use fontique::GenericFamily;

                let mut collection = fontique::Collection::new(fontique::CollectionOptions {
                    shared: false,
                    system_fonts: false,
                });
                let mut families = Vec::new();
                for data in fonts {
                    families.extend(
                        collection
                            .register_fonts(data, None)
                            .into_iter()
                            .map(|(family, _)| family),
                    );
                }
                // Route every CSS generic to the bundled faces: with system
                // enumeration off there is nothing else a generic could mean.
                for generic in [
                    GenericFamily::Serif,
                    GenericFamily::SansSerif,
                    GenericFamily::Monospace,
                    GenericFamily::Cursive,
                    GenericFamily::Fantasy,
                    GenericFamily::SystemUi,
                    GenericFamily::UiSerif,
                    GenericFamily::UiSansSerif,
                    GenericFamily::UiMonospace,
                    GenericFamily::UiRounded,
                    GenericFamily::Emoji,
                    GenericFamily::Math,
                    GenericFamily::FangSong,
                ] {
                    collection.set_generic_families(generic, families.iter().copied());
                }
                let has_fonts = !families.is_empty();
                (collection, has_fonts)
            }
        };
        Self {
            font_cx: parley::FontContext {
                collection,
                source_cache: parley::fontique::SourceCache::default(),
            },
            layout_cx: parley::LayoutContext::new(),
            has_fonts,
            work: FrameWork::ZERO,
        }
    }

    /// Fails fast when text is shaped with no font registered.
    ///
    /// An empty collection would lay out zero glyph runs and the screen
    /// would simply show no text — the exact silent degradation dew's
    /// design forbids. Firmware boards must return their bundled fonts from
    /// `Board::fonts`.
    fn assert_has_fonts(&self) {
        assert!(
            self.has_fonts,
            "dew has no fonts to shape text with: this build has no system font \
             collection, and Board::fonts provided no bundled fonts. Return \
             FontSources::bundled(&[include_bytes!(\"YourFont.ttf\")]) from the \
             board (or register fonts on HostBoard::with_font in tests)."
        );
    }

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
        brush: peniko::Color,
    ) -> parley::Layout<[u8; 4]> {
        self.assert_has_fonts();
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::Brush(peniko_to_rgba8(brush)));
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
        self.assert_has_fonts();

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
        let layout = self.build_styled_layout(styled, env, max_width, theme::foreground(env));
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
    for run in retain_glyph_runs(layout, list, None) {
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
/// The laid-out size, counting at most `max_lines` lines.
///
/// Mirrors the hydrolysis renderer's capped measurement: a limited text
/// reserves height for its visible lines only, and Dew clips the remainder at
/// the line boundary — the frugal renderer draws no ellipsis.
fn capped_layout_size(layout: &parley::Layout<[u8; 4]>, max_lines: Option<usize>) -> (f32, f32) {
    let Some(limit) = max_lines else {
        return (layout.width(), layout.height());
    };
    let mut width = 0.0_f32;
    let mut height = 0.0_f32;
    for line in layout.lines().take(limit) {
        let metrics = line.metrics();
        width = width.max(metrics.advance);
        height += metrics.line_height;
    }
    (width, height)
}

fn retain_glyph_runs(
    layout: &parley::Layout<[u8; 4]>,
    list: &mut DisplayList,
    max_lines: Option<usize>,
) -> Vec<RetainedGlyphRun> {
    let mut retained = Vec::new();
    let transform = Affine::IDENTITY;
    let layout_bounds = Rect::new(
        0.0,
        0.0,
        f64::from(layout.width()),
        f64::from(layout.height()),
    );
    for line in layout.lines().take(max_lines.unwrap_or(usize::MAX)) {
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
                    glyphs: glyphs.into(),
                    glyph_bounds: glyph_bounds.into(),
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

        cache.measure(
            0,
            TextLayoutKey::new(Some(120.0), theme::FOREGROUND),
            None,
            build,
        );
        cache.measure(
            0,
            TextLayoutKey::new(Some(120.0), theme::FOREGROUND),
            None,
            build,
        );
        cache.measure(
            0,
            TextLayoutKey::new(Some(80.0), theme::FOREGROUND),
            None,
            build,
        );
        cache.measure(
            1,
            TextLayoutKey::new(Some(120.0), theme::FOREGROUND),
            None,
            build,
        );

        assert_eq!(builds.get(), 3);
    }
}
