//! Text shaping and rasterization: parley layout → retained glyph runs.
//!
//! Dew shares the text stack with hydrolysis: [`parley`] shapes and lays out
//! text, and the resulting positioned glyph runs are retained as
//! [`DrawCommand::GlyphRun`]s that the painter rasterizes through
//! `vello_cpu`'s glyph pipeline. Fonts come from the system collection on
//! desktop; embedded targets register bundled fonts into the same
//! [`parley::FontContext`].

use kurbo::{Affine, Rect};
use waterui_text::styled::StyledStr;

use crate::display_list::{DisplayList, DrawCommand};

/// Shared text-shaping state: the font collection and parley's scratch
/// layout context.
///
/// One per renderer; rebuilding it is expensive (font enumeration).
pub struct DewState {
    font_cx: parley::FontContext,
    layout_cx: parley::LayoutContext<[u8; 4]>,
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
        }
    }
}

/// Default text color until themed text styles land (opaque black).
const DEFAULT_TEXT_RGBA: [u8; 4] = [0, 0, 0, 255];
/// Default font size in logical pixels until themed fonts land.
const DEFAULT_FONT_SIZE: f32 = 16.0;

impl DewState {
    /// Shapes `text` into a line-broken, aligned parley layout constrained
    /// to `max_width` logical pixels.
    pub(crate) fn build_text_layout(
        &mut self,
        text: &str,
        max_width: Option<f32>,
    ) -> parley::Layout<[u8; 4]> {
        let mut builder = self
            .layout_cx
            .ranged_builder(&mut self.font_cx, text, 1.0, true);
        builder.push_default(parley::StyleProperty::Brush(DEFAULT_TEXT_RGBA));
        builder.push_default(parley::StyleProperty::FontSize(DEFAULT_FONT_SIZE));
        let mut layout = builder.build(text);
        layout.break_all_lines(max_width);
        layout.align(
            max_width,
            parley::Alignment::Start,
            parley::AlignmentOptions::default(),
        );
        layout
    }

    /// Measures `text` at the given width constraint, returning the laid-out
    /// size in logical pixels.
    pub(crate) fn measure_text(&mut self, text: &str, max_width: Option<f32>) -> (f32, f32) {
        let layout = self.build_text_layout(text, max_width);
        (layout.width(), layout.height())
    }
}

/// Flattens a styled string to plain text.
///
/// Span styling (per-chunk fonts/colors) is not supported by dew yet; the
/// chunks render with the default style.
pub(crate) fn styled_to_plain(styled: &StyledStr) -> String {
    styled
        .chunks()
        .iter()
        .map(|(chunk, _)| chunk.as_str())
        .collect()
}

/// Appends one [`DrawCommand::GlyphRun`] per positioned glyph run in
/// `layout`, placed at `transform`.
pub(crate) fn emit_text_commands(
    list: &mut DisplayList,
    layout: &parley::Layout<[u8; 4]>,
    transform: Affine,
) {
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
            let glyphs: Vec<vello_cpu::Glyph> = glyph_run
                .glyphs()
                .map(|glyph| {
                    let x = run_x + glyph.x;
                    run_x += glyph.advance;
                    vello_cpu::Glyph {
                        id: glyph.id,
                        x,
                        y: run_y - glyph.y,
                    }
                })
                .collect();
            if glyphs.is_empty() {
                continue;
            }
            list.push(DrawCommand::GlyphRun {
                font: run.font().clone(),
                font_size: run.font_size(),
                glyphs,
                transform,
                brush: peniko::Color::from_rgba8(red, green, blue, alpha).into(),
                bounds: layout_bounds,
            });
        }
    }
}
