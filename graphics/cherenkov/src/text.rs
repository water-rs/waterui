//! The parley adapter: a shaped [`parley::Layout`] whose brush is the
//! engine's [`Paint`], lowered to the engine's glyph runs and rectangle
//! fills.
//!
//! Shaping stays outside the engine. [`TextLayout::new`] resolves every
//! font the layout shaped with to an engine [`Font`] and lowers the layout
//! once; [`draw_text`] then records those primitives at an origin through
//! any [`Draw`], so text reaches both backends through the same glyph-run
//! path as [`Draw::glyphs`](crate::Draw::glyphs).

use std::sync::Arc;

use kurbo::{Affine, Cap, Join, Point, Rect, Stroke};
use parley::{Decoration, GlyphRun as ParleyRun, Layout, PositionedLayoutItem};

use crate::error::ResourceError;
use crate::glyph::{FontId, Glyph, GlyphRun, GlyphStyle};
use crate::paint::Paint;
use crate::resource::{Font, FontSource};
use crate::{DisplayList, Draw, Fixed, Group, Picture, ResourceId, TextLayoutId};
use rustc_hash::FxHashMap;

/// A shaped parley layout, ready to record with [`draw_text`].
///
/// Every style's brush is a [`Paint`]. The layout lowers to:
/// - one [`GlyphRun`] per parley glyph run, filled with the style's brush;
///   a synthetic oblique (fontique's `skew`) becomes each glyph's
///   transform;
/// - for a run fontique asks to embolden (a heavier weight than any face
///   or axis of the font offers), the same run filled and then stroked
///   with a mitred outline 1/24 of the em wide at 9 px and below, 1/32
///   at 36 px and above and linear in between, so each outline grows by
///   half that width on every side. Unless the brush is an opaque
///   colour, the pair draws in an isolated group, so a translucent or
///   patterned brush covers the overlap once;
/// - one rectangle fill per underline and strikethrough, at the
///   decoration's offset and thickness (the run's font metrics when the
///   style leaves them unset), filled with the decoration's brush.
///   Decorations that continue across adjacent runs of a line with the
///   same brush and geometry are one rectangle.
///
/// Each line draws its underlines, then its glyphs, then its
/// strikethroughs. Inline boxes draw nothing: their content belongs to
/// the host.
#[derive(Debug)]
pub struct TextLayout {
    commands: Vec<TextCommand>,
    /// The colour of the paragraph's unstyled runs: what a
    /// [`Command::Text`](crate::Command::Text) paint replaces.
    default: Option<Paint>,
    /// Keeps every font the commands name registered.
    fonts: Vec<Font>,
}

/// One lowered primitive, in layout coordinates.
#[derive(Clone, Debug)]
pub enum TextCommand {
    /// A glyph run and its paint.
    Glyphs(GlyphRun, Paint),
    /// A synthetic bold: the filled run, the stroke that widens it, and
    /// their paint.
    Bold(GlyphRun, Stroke, Paint),
    /// A decoration rectangle and its paint.
    Fill(Rect, Paint),
}

/// A decoration rectangle still open for extension along its line, in
/// the layout's f32 coordinates so contiguity and geometry compare
/// exactly.
struct Stripe {
    start: f32,
    end: f32,
    top: f32,
    size: f32,
    paint: Paint,
}

impl Stripe {
    fn into_command(self) -> TextCommand {
        TextCommand::Fill(
            Rect::new(
                f64::from(self.start),
                f64::from(self.top),
                f64::from(self.end),
                f64::from(self.top) + f64::from(self.size),
            ),
            self.paint,
        )
    }
}

impl TextLayout {
    /// Lowers `layout`, whose styles' brushes `brush` turns into engine
    /// paints, resolving each font it shaped with through `font`.
    ///
    /// `font` is called once per distinct font data (blob and collection
    /// index) in the layout. It should hand out the same engine [`Font`]
    /// for the same data across layouts, typically from a cache over
    /// [`Engine::font`](crate::Engine::font) and [`FontSource::from`]: the
    /// engine keys its glyph caches on the font id.
    ///
    /// # Errors
    /// Any error `font` returns.
    pub fn new<B: parley::Brush>(
        layout: &Layout<B>,
        brush: impl Fn(&B) -> Paint,
        mut font: impl FnMut(&parley::FontData) -> Result<Font, ResourceError>,
    ) -> Result<Self, ResourceError> {
        let mut fonts: Vec<(u64, u32, Font)> = Vec::new();
        let mut commands = Vec::new();
        for line in layout.lines() {
            let mut underlines: Vec<Stripe> = Vec::new();
            let mut strikethroughs: Vec<Stripe> = Vec::new();
            let mut glyphs = Vec::new();
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(run) = item else {
                    continue;
                };
                let data = run.run().font();
                let key = (data.data.id(), data.index);
                let id = if let Some((_, _, registered)) =
                    fonts.iter().find(|(blob, index, _)| (*blob, *index) == key)
                {
                    registered.id()
                } else {
                    let registered = font(data)?;
                    let id = registered.id();
                    fonts.push((key.0, key.1, registered));
                    id
                };
                glyphs.push(lower_run(&run, id, brush(&run.style().brush)));
                let metrics = run.run().font_metrics();
                if let Some(underline) = &run.style().underline {
                    stripe(
                        &mut underlines,
                        &run,
                        brush(&underline.brush),
                        underline,
                        metrics.underline_offset,
                        metrics.underline_size,
                    );
                }
                if let Some(strikethrough) = &run.style().strikethrough {
                    stripe(
                        &mut strikethroughs,
                        &run,
                        brush(&strikethrough.brush),
                        strikethrough,
                        metrics.strikethrough_offset,
                        metrics.strikethrough_size,
                    );
                }
            }
            commands.extend(underlines.into_iter().map(Stripe::into_command));
            commands.append(&mut glyphs);
            commands.extend(strikethroughs.into_iter().map(Stripe::into_command));
        }
        Ok(Self {
            commands,
            default: None,
            fonts: fonts.into_iter().map(|(_, _, font)| font).collect(),
        })
    }

    /// Names the colour of the paragraph's unstyled runs: the colour a
    /// [`Command::Text`](crate::Command::Text) paint replaces. Runs the
    /// text styled with another colour keep theirs.
    #[must_use]
    pub fn with_default_paint(mut self, paint: Paint) -> Self {
        self.default = Some(paint);
        self
    }

    /// Fills `rects` beneath the text, each in its paint: the backgrounds
    /// of styled spans, which a paragraph's runs do not carry.
    #[must_use]
    pub fn with_underlay(mut self, rects: impl IntoIterator<Item = (Rect, Paint)>) -> Self {
        let under = rects
            .into_iter()
            .map(|(rect, paint)| TextCommand::Fill(rect, paint));
        self.commands.splice(0..0, under);
        self
    }

    /// A layout of `commands` drawing with `fonts`, for engine tests that
    /// have no shaped paragraph.
    #[cfg(test)]
    pub(crate) const fn from_commands(commands: Vec<TextCommand>, fonts: Vec<Font>) -> Self {
        Self {
            commands,
            default: None,
            fonts,
        }
    }

    /// The registered fonts the commands draw with, held for as long as
    /// the layout lives.
    pub(crate) fn into_parts(self) -> (Vec<TextCommand>, Option<Paint>, Vec<Font>) {
        (self.commands, self.default, self.fonts)
    }

    /// The lowered primitives in layout coordinates, in draw order.
    pub(crate) fn commands(&self) -> &[TextCommand] {
        &self.commands
    }
}

/// The stroke width that draws a synthetic bold at `size` units per em:
/// 1/24 of the em at 9 and below, 1/32 at 36 and above, and linear in
/// between, so small text thickens relatively more.
fn embolden_width(size: f32) -> f64 {
    let size = f64::from(size);
    let t = ((size - 9.0) / 27.0).clamp(0.0, 1.0);
    size * t.mul_add(1.0 / 32.0 - 1.0 / 24.0, 1.0 / 24.0)
}

/// `run`'s glyphs in layout coordinates, drawn with font `font` and the
/// run's brush.
fn lower_run<B: parley::Brush>(run: &ParleyRun<'_, B>, font: FontId, paint: Paint) -> TextCommand {
    let shaped = run.run();
    let synthesis = shaped.synthesis();
    // A synthetic oblique leans the glyph's top forward: `x' = x − y·tan θ`
    // in the glyph's y-down space, about its origin.
    let transform = synthesis
        .skew()
        .map(|degrees| Affine::skew(-libm::tan(f64::from(degrees).to_radians()), 0.0));
    let glyphs = GlyphRun {
        font,
        size: shaped.font_size(),
        coords: shaped
            .normalized_coords()
            .iter()
            .map(|coord| coord.to_bits())
            .collect(),
        glyphs: run
            .positioned_glyphs()
            .map(|glyph| Glyph {
                id: glyph.id,
                x: glyph.x,
                y: glyph.y,
                transform,
            })
            .collect(),
        style: GlyphStyle::Fill,
    };
    if synthesis.embolden() {
        let stroke = Stroke::new(embolden_width(shaped.font_size()))
            .with_join(Join::Miter)
            .with_caps(Cap::Butt);
        TextCommand::Bold(glyphs, stroke, paint)
    } else {
        TextCommand::Glyphs(glyphs, paint)
    }
}

/// Adds `run`'s span of `decoration` to `stripes`, extending the last
/// stripe when it ends where the run starts with the same brush and
/// geometry. `offset` and `size` are the run's metrics, used where the
/// decoration leaves them unset; `offset` is the top edge's distance above
/// the baseline.
fn stripe<B: parley::Brush>(
    stripes: &mut Vec<Stripe>,
    run: &ParleyRun<'_, B>,
    paint: Paint,
    decoration: &Decoration<B>,
    offset: f32,
    size: f32,
) {
    let top = run.baseline() - decoration.offset.unwrap_or(offset);
    let size = decoration.size.unwrap_or(size);
    let start = run.offset();
    let end = start + run.advance();
    if let Some(last) = stripes.last_mut()
        && contiguous(last, start, top, size)
        && last.paint == paint
    {
        last.end = end;
        return;
    }
    stripes.push(Stripe {
        start,
        end,
        top,
        size,
        paint,
    });
}

/// Whether a stripe starting at `start` with `top` and `size` continues
/// `last`: parley accumulates run offsets in f32, so a run that follows
/// another starts exactly where it ends.
#[expect(
    clippy::float_cmp,
    reason = "contiguity is exact: parley adds each run's advance to the previous offset"
)]
fn contiguous(last: &Stripe, start: f32, top: f32, size: f32) -> bool {
    last.end == start && last.top == top && last.size == size
}

/// Records `layout`'s lowered primitives into `draw` with the layout's
/// top-left at `origin`.
///
/// The primitives are the [`Draw::glyphs`], [`Draw::fill`] and
/// [`Draw::group`] commands [`TextLayout`] lowers to, recorded as
/// constants — a layout is not a signal, so nothing subscribes, and a new
/// layout is a new recording.
pub fn draw_text<D>(draw: &mut D, layout: &TextLayout, origin: Point)
where
    D: Draw,
    D::Value<GlyphRun>: From<Fixed<GlyphRun>>,
    D::Value<Paint>: From<Fixed<Paint>>,
    D::Value<Rect>: From<Fixed<Rect>>,
    D::Value<Group>: From<Fixed<Group>>,
{
    draw_commands(draw, layout.commands(), origin, None);
}

/// Records lowered text `commands` at `origin`. With `replace`, a command
/// painted with the first paint is painted with the second instead.
fn draw_commands<D>(
    draw: &mut D,
    commands: &[TextCommand],
    origin: Point,
    replace: Option<(&Paint, &Paint)>,
) where
    D: Draw,
    // The lowered primitives are constants: `Fixed` moves them into the
    // commands without a signal snapshot.
    D::Value<GlyphRun>: From<Fixed<GlyphRun>>,
    D::Value<Paint>: From<Fixed<Paint>>,
    D::Value<Rect>: From<Fixed<Rect>>,
    D::Value<Group>: From<Fixed<Group>>,
{
    let painted = |paint: &Paint| match replace {
        Some((default, with)) if paint == default => with.clone(),
        _ => paint.clone(),
    };
    for command in commands {
        match command {
            TextCommand::Glyphs(run, paint) => {
                let paint = &painted(paint);
                draw.glyphs(Fixed(place_run(run, origin)), Fixed(paint.clone()));
            }
            TextCommand::Bold(run, stroke, paint) => {
                let paint = &painted(paint);
                let fill = place_run(run, origin);
                let outline = GlyphRun {
                    style: GlyphStyle::Stroke(stroke.clone()),
                    ..fill.clone()
                };
                // With one opaque colour, the fill and then the stroke
                // composite to exactly what the isolated pair does; any
                // other paint would blend twice where the two overlap.
                if matches!(paint, Paint::Solid(color) if color.components[3] >= 1.0) {
                    draw.glyphs(Fixed(fill), Fixed(paint.clone()));
                    draw.glyphs(Fixed(outline), Fixed(paint.clone()));
                } else {
                    draw.group(Fixed(Group::new()), |draw| {
                        draw.glyphs(Fixed(fill), Fixed(paint.clone()));
                        draw.glyphs(Fixed(outline), Fixed(paint.clone()));
                    });
                }
            }
            TextCommand::Fill(rect, paint) => {
                draw.fill(Fixed(*rect + origin.to_vec2()), Fixed(painted(paint)));
            }
        }
    }
}

/// `run` moved by `origin`: glyph positions add in f64 and round once to
/// the run's f32.
#[expect(
    clippy::cast_possible_truncation,
    reason = "glyph positions are f32 by contract; the sum rounds once"
)]
pub fn place_run(run: &GlyphRun, origin: Point) -> GlyphRun {
    GlyphRun {
        font: run.font,
        size: run.size,
        coords: Arc::clone(&run.coords),
        glyphs: run
            .glyphs
            .iter()
            .map(|glyph| Glyph {
                id: glyph.id,
                x: (f64::from(glyph.x) + origin.x) as f32,
                y: (f64::from(glyph.y) + origin.y) as f32,
                transform: glyph.transform,
            })
            .collect(),
        style: run.style.clone(),
    }
}

/// One registered text layout on the render side.
#[derive(Debug)]
struct TextEntry {
    commands: Arc<[TextCommand]>,
    default: Option<Paint>,
    fonts: Box<[FontId]>,
    /// The layout recorded as drawn, for a `Command::Text` that keeps its
    /// colours.
    plain: Picture,
}

/// The text layouts a renderer holds, by the id their registration was
/// given. Lowering expands a [`Command::Text`](crate::Command::Text) into
/// the layout's glyph-run commands the way it expands a
/// [`Command::Picture`](crate::Command::Picture).
#[derive(Debug, Default)]
pub struct TextLayouts {
    entries: FxHashMap<TextLayoutId, TextEntry>,
}

/// What registering a text layout sends the render thread.
#[derive(Debug)]
pub struct TextLayoutData {
    commands: Arc<[TextCommand]>,
    default: Option<Paint>,
    fonts: Box<[FontId]>,
}

impl TextLayoutData {
    pub(crate) fn new(commands: Vec<TextCommand>, default: Option<Paint>, fonts: &[Font]) -> Self {
        Self {
            commands: commands.into(),
            default,
            fonts: fonts.iter().map(Font::id).collect(),
        }
    }
}

impl TextLayouts {
    /// Registers `data` as layout `id`.
    pub fn add(&mut self, id: TextLayoutId, data: TextLayoutData) {
        let TextLayoutData {
            commands,
            default,
            fonts,
        } = data;
        let plain = Picture::record(|c| draw_commands(c, &commands, Point::ZERO, None));
        self.entries.insert(
            id,
            TextEntry {
                commands,
                default,
                fonts,
                plain,
            },
        );
    }

    /// Unregisters layout `id`.
    ///
    /// # Panics
    /// When `id` is not registered.
    pub fn remove(&mut self, id: TextLayoutId) {
        assert!(
            self.entries.remove(&id).is_some(),
            "removal of unregistered text layout {}",
            id.raw()
        );
    }

    /// The glyph-run expansion of layout `id`, in layout coordinates. With
    /// `paint`, the runs drawn in the layout's default colour draw with it.
    ///
    /// # Panics
    /// When `id` is not registered: content names only layouts it holds.
    #[must_use]
    pub fn expand(&self, id: TextLayoutId, paint: Option<&Paint>) -> Picture {
        let entry = self.entries.get(&id).unwrap_or_else(|| {
            panic!(
                "text layout {} is drawn but not registered: a recording names only what it holds",
                id.raw()
            )
        });
        match (paint, &entry.default) {
            (Some(with), Some(default)) => Picture::record(|c| {
                draw_commands(c, &entry.commands, Point::ZERO, Some((default, with)));
            }),
            _ => entry.plain.clone(),
        }
    }

    /// Whether `list` samples `resource`, looking through its text commands
    /// to the fonts their layouts draw with.
    #[must_use]
    pub fn samples(&self, list: &DisplayList, resource: ResourceId) -> bool {
        list.references(resource)
            || match resource {
                ResourceId::Font(font) => self.entries.iter().any(|(id, entry)| {
                    entry.fonts.contains(&font) && list.references(ResourceId::TextLayout(*id))
                }),
                _ => false,
            }
    }
}

impl From<&parley::FontData> for FontSource {
    /// The font a parley run shaped with. The bytes are copied once into
    /// the source, as [`FontSource::mapped`] reads them.
    fn from(font: &parley::FontData) -> Self {
        Self::bytes(font.data.data()).with_index(font.index)
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::borrow::Cow;
    use std::cell::Cell;
    use std::ops::Range;
    use std::sync::Arc;

    use kurbo::{Affine, Point, Rect};
    use parley::fontique::{Blob, Collection, CollectionOptions, SourceCache};
    use parley::{
        FontContext, FontFamily, FontFamilyName, FontStyle, FontWeight, Layout, LayoutContext,
        PositionedLayoutItem, StyleProperty,
    };

    use super::{TextCommand, TextLayout, TextLayouts, draw_text};
    use crate::display_list::Command;
    use crate::error::ResourceError;
    use crate::glyph::{FontId, GlyphStyle};
    use crate::paint::Paint;
    use crate::resource::Font;
    use crate::shape::ShapeData;
    use crate::{Draw, Fixed, ResourceId, TextLayoutId};
    use crate::{Group, Picture, WorkingColor};

    const INK: Paint = Paint::Solid(WorkingColor::new([0.1, 0.2, 0.3, 1.0]));
    const RED: Paint = Paint::Solid(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
    const BLUE: Paint = Paint::Solid(WorkingColor::new([0.0, 0.0, 1.0, 1.0]));

    /// Glyph ids and positions.
    type Placed = Vec<(u32, f32, f32)>;

    /// A style a test applies to a byte range.
    enum Style {
        Brush(Paint),
        Underline(Option<Paint>),
        Strikethrough(Option<Paint>),
        Italic,
        Bold,
    }

    /// Lays `text` out in the generated font `file` at 20 px, with no
    /// system fonts.
    fn layout(file: &str, text: &str, styles: &[(Range<usize>, Style)]) -> Layout<Paint> {
        let data = std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("scenes/fonts")
                .join(file),
        )
        .expect("the scene fonts are generated; run `scenes/tools/generate.py` first");
        let mut fcx = FontContext {
            collection: Collection::new(CollectionOptions {
                shared: false,
                system_fonts: false,
            }),
            source_cache: SourceCache::default(),
        };
        let registered = fcx
            .collection
            .register_fonts(Blob::new(Arc::new(data)), None);
        let family = fcx
            .collection
            .family_name(registered[0].0)
            .expect("the font names its family")
            .to_owned();
        let mut lcx = LayoutContext::new();
        let mut builder = lcx.ranged_builder(&mut fcx, text, 1.0, false);
        builder.push_default(StyleProperty::FontFamily(FontFamily::Single(
            FontFamilyName::Named(Cow::Owned(family)),
        )));
        builder.push_default(StyleProperty::FontSize(20.0));
        builder.push_default(StyleProperty::Brush(INK));
        for (range, style) in styles {
            let range = range.clone();
            match style {
                Style::Brush(paint) => builder.push(StyleProperty::Brush(paint.clone()), range),
                Style::Underline(paint) => {
                    builder.push(StyleProperty::Underline(true), range.clone());
                    builder.push(StyleProperty::UnderlineBrush(paint.clone()), range);
                }
                Style::Strikethrough(paint) => {
                    builder.push(StyleProperty::Strikethrough(true), range.clone());
                    builder.push(StyleProperty::StrikethroughBrush(paint.clone()), range);
                }
                Style::Italic => builder.push(StyleProperty::FontStyle(FontStyle::Italic), range),
                Style::Bold => builder.push(StyleProperty::FontWeight(FontWeight::BOLD), range),
            }
        }
        let mut layout = builder.build(text);
        layout.break_all_lines(None);
        layout
    }

    /// Lowers `layout`, counting font resolutions.
    fn lower(layout: &Layout<Paint>, calls: &Cell<usize>) -> Result<TextLayout, ResourceError> {
        TextLayout::new(layout, Paint::clone, |_| {
            calls.set(calls.get() + 1);
            Ok(Font::new(FontId::new(7), || {}))
        })
    }

    fn fills(text: &TextLayout) -> Vec<(Rect, Paint)> {
        text.commands()
            .iter()
            .filter_map(|command| match command {
                TextCommand::Fill(rect, paint) => Some((*rect, paint.clone())),
                TextCommand::Glyphs(..) | TextCommand::Bold(..) => None,
            })
            .collect()
    }

    #[test]
    fn a_layout_lowers_to_its_runs_and_decorations_in_line_order() {
        let text = "Hello world again";
        let parley = layout(
            "NotoSans.ttf",
            text,
            &[
                (6..11, Style::Brush(RED)),
                (6..11, Style::Underline(None)),
                (12..17, Style::Strikethrough(Some(BLUE))),
            ],
        );
        let line = parley.lines().next().expect("one line");
        let baseline = f64::from(line.metrics().baseline);
        let mut expected = Vec::new();
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(run) = item else {
                continue;
            };
            let glyphs: Placed = run.positioned_glyphs().map(|g| (g.id, g.x, g.y)).collect();
            expected.push((glyphs, run.style().brush.clone()));
        }
        let calls = Cell::new(0);
        let lowered = lower(&parley, &calls).expect("lowers");
        assert_eq!(calls.get(), 1, "one font, resolved once");

        let commands = lowered.commands();
        assert!(
            matches!(commands.first(), Some(TextCommand::Fill(..))),
            "the underline draws first"
        );
        assert!(
            matches!(commands.last(), Some(TextCommand::Fill(..))),
            "the strikethrough draws last"
        );
        let runs: Vec<(Placed, Paint)> = commands
            .iter()
            .filter_map(|command| match command {
                TextCommand::Glyphs(run, paint) => {
                    assert_eq!(run.font, FontId::new(7));
                    assert!(run.glyphs.iter().all(|g| g.transform.is_none()));
                    Some((
                        run.glyphs.iter().map(|g| (g.id, g.x, g.y)).collect(),
                        paint.clone(),
                    ))
                }
                TextCommand::Fill(..) => None,
                TextCommand::Bold(..) => panic!("no run is emboldened"),
            })
            .collect();
        assert_eq!(
            runs, expected,
            "each parley run is one glyph run with its brush"
        );

        let fills = fills(&lowered);
        assert_eq!(fills.len(), 2);
        let (underline, paint) = &fills[0];
        assert_eq!(*paint, RED, "an underline without a brush takes the text's");
        assert!(
            underline.y0 > baseline,
            "the underline sits below the baseline"
        );
        let (strikethrough, paint) = &fills[1];
        assert_eq!(*paint, BLUE);
        assert!(
            strikethrough.y1 < baseline && strikethrough.y0 > baseline - 20.0,
            "the strikethrough crosses the lowercase letters: {strikethrough:?}"
        );
        assert!(
            underline.x1 <= strikethrough.x0,
            "each decoration spans its own range"
        );
    }

    #[test]
    fn a_decoration_continuing_across_runs_is_one_rectangle() {
        let text = "abcdef";
        let merged = lower(
            &layout(
                "NotoSans.ttf",
                text,
                &[
                    (0..3, Style::Brush(RED)),
                    (3..6, Style::Brush(BLUE)),
                    (0..6, Style::Underline(Some(INK))),
                ],
            ),
            &Cell::new(0),
        )
        .expect("lowers");
        let runs = merged
            .commands()
            .iter()
            .filter(|command| matches!(command, TextCommand::Glyphs(..)))
            .count();
        assert_eq!(runs, 2, "the brush change splits the glyphs");
        assert_eq!(fills(&merged).len(), 1, "the shared underline does not");

        let split = lower(
            &layout(
                "NotoSans.ttf",
                text,
                &[
                    (0..3, Style::Underline(Some(RED))),
                    (3..6, Style::Underline(Some(BLUE))),
                ],
            ),
            &Cell::new(0),
        )
        .expect("lowers");
        let fills = fills(&split);
        assert_eq!(fills.len(), 2, "a brush change splits the underline");
        // parley starts each run at the f32 sum of the previous run's
        // offset and advance, the same sum that ends the first stripe, and
        // both widen to f64 exactly.
        assert_eq!(
            fills[0].0.x1.to_bits(),
            fills[1].0.x0.to_bits(),
            "the halves abut"
        );
    }

    #[test]
    fn a_synthetic_oblique_leans_each_glyph_about_its_origin() {
        let lowered = lower(
            &layout("NotoSans.ttf", "lean", &[(0..4, Style::Italic)]),
            &Cell::new(0),
        )
        .expect("lowers");
        let Some(TextCommand::Glyphs(run, _)) = lowered.commands().first() else {
            panic!("one glyph run");
        };
        let lean = Affine::skew(-libm::tan(14_f64.to_radians()), 0.0);
        assert!(run.glyphs.iter().all(|g| g.transform == Some(lean)));
        // The top of an em leans forward, right of the glyph origin.
        assert!((lean * Point::new(0.0, -10.0)).x > 0.0);
    }

    #[test]
    fn a_synthetic_bold_fills_then_strokes_and_isolates_a_translucent_brush() {
        const GLASS: Paint = Paint::Solid(WorkingColor::new([0.0, 0.0, 1.0, 0.5]));
        let styles = [(0..10, Style::Bold), (5..10, Style::Brush(GLASS))];

        // A weight axis carries the bold itself.
        let axis = lower(
            &layout("NotoSans.ttf", "bold glass", &styles),
            &Cell::new(0),
        )
        .expect("lowers");
        assert!(
            axis.commands()
                .iter()
                .all(|command| matches!(command, TextCommand::Glyphs(..))),
            "a variable font is not emboldened"
        );

        // A single regular face has nothing heavier, so fontique emboldens.
        let lowered = lower(
            &layout("CherenkovStaticSans.ttf", "bold glass", &styles),
            &Cell::new(0),
        )
        .expect("lowers");
        let [
            TextCommand::Bold(ink, stroke, ink_paint),
            TextCommand::Bold(glass, _, glass_paint),
        ] = lowered.commands()
        else {
            panic!("two emboldened runs: {:?}", lowered.commands());
        };
        assert_eq!((ink_paint, glass_paint), (&INK, &GLASS));
        let t = 11.0 / 27.0;
        let width = 20.0 * f64::mul_add(t, 1.0 / 32.0 - 1.0 / 24.0, 1.0 / 24.0);
        assert!((stroke.width - width).abs() < 1e-12, "{}", stroke.width);
        assert_eq!(stroke.join, kurbo::Join::Miter);
        assert_eq!(ink.style, GlyphStyle::Fill);

        let picture = Picture::record(|c| draw_text(c, &lowered, Point::ZERO));
        let recorded = picture.display_list().commands();
        let run = |command: &Command| match command {
            Command::Glyphs { run, paint } => {
                (run.glyphs.clone(), run.style.clone(), paint.clone())
            }
            other => panic!("expected glyphs, got {other:?}"),
        };
        assert_eq!(recorded.len(), 6, "{recorded:?}");
        let filled = GlyphStyle::Fill;
        let stroked = GlyphStyle::Stroke(stroke.clone());
        assert_eq!(run(&recorded[0]), (ink.glyphs.clone(), filled.clone(), INK));
        assert_eq!(
            run(&recorded[1]),
            (ink.glyphs.clone(), stroked.clone(), INK)
        );
        assert!(
            matches!(&recorded[2], Command::BeginGroup { group, end: 5 } if *group == Group::new()),
            "the translucent pair is isolated: {:?}",
            recorded[2]
        );
        assert_eq!(run(&recorded[3]), (glass.glyphs.clone(), filled, GLASS));
        assert_eq!(run(&recorded[4]), (glass.glyphs.clone(), stroked, GLASS));
        assert_eq!(recorded[5], Command::End);
    }

    #[test]
    fn text_records_its_primitives_at_the_origin() {
        let lowered = lower(
            &layout(
                "NotoSans.ttf",
                "Shift me",
                &[(0..5, Style::Underline(None))],
            ),
            &Cell::new(0),
        )
        .expect("lowers");
        let origin = Point::new(12.5, 40.25);
        let picture = Picture::record(|c| draw_text(c, &lowered, origin));
        let recorded = picture.display_list().commands();
        assert_eq!(recorded.len(), lowered.commands().len());
        for (command, lowered) in recorded.iter().zip(lowered.commands()) {
            match (command, lowered) {
                (Command::Glyphs { run, paint }, TextCommand::Glyphs(source, expected)) => {
                    assert_eq!(paint, expected);
                    for (glyph, source) in run.glyphs.iter().zip(source.glyphs.iter()) {
                        assert_eq!(glyph.id, source.id);
                        // `place_run` adds the origin in f64 and rounds once to
                        // f32. A layout offset of a few hundred pixels plus 12.5
                        // or 40.25 is exact in f64, so that one rounding is the
                        // correctly rounded f32 sum computed here.
                        assert_eq!(glyph.x.to_bits(), (source.x + 12.5).to_bits());
                        assert_eq!(glyph.y.to_bits(), (source.y + 40.25).to_bits());
                    }
                }
                (
                    Command::Fill {
                        shape: ShapeData::Rect(rect),
                        paint,
                    },
                    TextCommand::Fill(source, expected),
                ) => {
                    assert_eq!(paint, expected);
                    assert_eq!(*rect, *source + origin.to_vec2());
                }
                other => panic!("mismatched command {other:?}"),
            }
        }
    }

    /// Registers a layout of `text` whose unstyled runs draw in `INK`,
    /// returning its data.
    fn registered(text: &str, styles: &[(Range<usize>, Style)]) -> super::TextLayoutData {
        let (commands, default, fonts) =
            lower(&layout("NotoSans.ttf", text, styles), &Cell::new(0))
                .expect("lowers")
                .with_default_paint(INK)
                .into_parts();
        super::TextLayoutData::new(commands, default, &fonts)
    }

    fn run_paints(picture: &Picture) -> Vec<Paint> {
        picture
            .display_list()
            .commands()
            .iter()
            .filter_map(|command| match command {
                Command::Glyphs { paint, .. } => Some(paint.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_registered_layout_expands_to_its_runs_and_a_paint_replaces_the_default() {
        let id = TextLayoutId::new(3);
        let mut layouts = TextLayouts::default();
        layouts.add(id, registered("Red Blue", &[(4..8, Style::Brush(BLUE))]));

        let plain = layouts.expand(id, None);
        assert_eq!(run_paints(&plain), [INK, BLUE]);

        // The unstyled run takes the override; the run styled with its own
        // colour keeps it.
        let replaced = layouts.expand(id, Some(&RED));
        assert_eq!(run_paints(&replaced), [RED, BLUE]);

        layouts.remove(id);
    }

    #[test]
    fn a_layout_keeps_its_fonts_sampled_while_content_names_it() {
        let id = TextLayoutId::new(1);
        let mut layouts = TextLayouts::default();
        layouts.add(id, registered("Font", &[]));
        let font = ResourceId::Font(FontId::new(7));

        let naming = Picture::record(|c| c.text(id, Fixed(Affine::IDENTITY)));
        let unrelated = Picture::record(|c| c.text(TextLayoutId::new(2), Fixed(Affine::IDENTITY)));
        assert!(layouts.samples(naming.display_list(), font));
        assert!(layouts.samples(naming.display_list(), ResourceId::TextLayout(id)));
        assert!(!layouts.samples(unrelated.display_list(), font));
        assert!(!layouts.samples(Picture::record(|_| {}).display_list(), font));
    }

    #[test]
    #[should_panic(expected = "not registered")]
    fn expanding_an_unregistered_layout_panics() {
        let _ = TextLayouts::default().expand(TextLayoutId::new(9), None);
    }

    #[test]
    #[should_panic(expected = "unregistered text layout")]
    fn removing_an_unregistered_layout_panics() {
        TextLayouts::default().remove(TextLayoutId::new(9));
    }
}
