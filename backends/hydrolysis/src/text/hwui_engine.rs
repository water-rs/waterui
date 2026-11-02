//! The HWUI text engine: the renderer's shared [`TextEngine`] backed by the
//! platform text provider ([`PlatformText`]) — Android's `StaticLayout`
//! behind `HwuiTextProvider` on device, a test provider under `cfg(test)`.
//!
//! Where [`ParleyEngine`](crate::text::ParleyEngine) ellipsizes by respelling
//! a truncated tail and reshaping it, this engine hands the line limit to the
//! platform: one `shape` call carries `max_lines` + the ellipsis flag, and the
//! platform's own layout produces the drawn tail (water-rs/waterui#1811).

use std::num::NonZeroUsize;
use std::ops::Range;
use std::sync::Arc;

use waterui_core::layout::HorizontalAlignment;
use waterui_graphics::draw::kurbo::{Affine, Rect};

use crate::hwui::HwuiError;
use crate::hwui::text::{
    HwuiTextLayout, PlatformText, ShapeRequest, TextAlignment, TextLayoutIds, TextRun,
};
use crate::renderer::{FrameWorkCounters, Recording};

use super::engine::{FontFamilyResolution, TextEngine};
use super::input::{ResolvedFontSpec, ResolvedTextLayoutInput, font_weight};

/// The HWUI-backed text engine. Shaping runs on the platform behind
/// `provider`; layout ids come from `ids`, the table the encoder writes
/// releases into.
pub struct HwuiTextEngine<P: PlatformText> {
    provider: Arc<P>,
    ids: TextLayoutIds,
    family_resolution: FontFamilyResolution,
}

impl<P: PlatformText> core::fmt::Debug for HwuiTextEngine<P> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("HwuiTextEngine")
            .field("family_resolution", &self.family_resolution)
            .finish_non_exhaustive()
    }
}

impl<P: PlatformText> HwuiTextEngine<P> {
    /// The engine on platform text provider `provider`, layouts leased from
    /// `ids`, resolving named families `family_resolution`-strictly.
    pub const fn new(
        provider: Arc<P>,
        ids: TextLayoutIds,
        family_resolution: FontFamilyResolution,
    ) -> Self {
        Self {
            provider,
            ids,
            family_resolution,
        }
    }

    /// Shapes `input` through the platform — one `shape` call per miss, with
    /// the line limit and ellipsis owned by the platform layout.
    ///
    /// # Panics
    /// Panics when the platform refuses the shape or returns a malformed
    /// reply: a text the platform cannot lay out is a programmer error,
    /// never a blank fallback.
    fn shape_platform(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: Option<NonZeroUsize>,
    ) -> HwuiTextLayout<P> {
        let default_style = font_run(&input.default_font, 0..0, Some(input.default_brush));
        let runs: Vec<TextRun> = input
            .spans
            .iter()
            .filter(|(range, _)| !range.is_empty())
            .map(|(range, style)| {
                let mut run = font_run(
                    &style.font,
                    range.clone(),
                    Some(style.foreground.unwrap_or(input.default_brush)),
                );
                run.set_background(style.background.map(color_long));
                run.set_italic(style.italic);
                run.set_underline(style.underline);
                run.set_strikethrough(style.strikethrough);
                run
            })
            .collect();
        let request = {
            let mut literal = ShapeRequest::new(&input.plain, &runs, default_style);
            literal.set_locale(&input.locale);
            literal.set_max_width(max_width);
            literal.set_max_lines(max_lines.map(NonZeroUsize::get));
            literal.set_ellipsis(max_lines.is_some());
            literal.set_alignment(if input.alignment == HorizontalAlignment::Leading {
                TextAlignment::Start
            } else if input.alignment == HorizontalAlignment::Trailing {
                TextAlignment::End
            } else {
                TextAlignment::Center
            });
            literal.set_right_to_left(input.right_to_left);
            literal.set_strict_families(self.family_resolution.is_strict());
            literal
        };
        HwuiTextLayout::shape(&self.provider, &self.ids, &request)
            .unwrap_or_else(|error: HwuiError| panic!("HWUI text shaping failed: {error}"))
    }
}

/// Android's canonical sRGB `ColorLong`: the resolved input already carries
/// encoded RGBA8, so the word just packs it — no conversion.
fn color_long([r, g, b, a]: [u8; 4]) -> u64 {
    u64::from(u32::from_be_bytes([a, r, g, b])) << 32
}

/// A style run on the text wire from a resolved font.
fn font_run(font: &ResolvedFontSpec, range: Range<usize>, foreground: Option<[u8; 4]>) -> TextRun {
    {
        let mut literal = TextRun::new(range, font.size, font_weight(font.weight));
        literal.set_family(font.family.clone());
        literal.set_italic(false);
        literal.set_underline(false);
        literal.set_strikethrough(false);
        literal.set_foreground(foreground.map(color_long));
        literal.set_background(None);
        literal.set_line_height(font.line_height);
        literal.set_letter_spacing(font.letter_spacing);
        literal
    }
}

impl<P: PlatformText> TextEngine for HwuiTextEngine<P> {
    type Layout = HwuiTextLayout<P>;

    fn empty_layout(&self) -> Self::Layout {
        HwuiTextLayout::empty()
    }

    fn shape(&self, input: &ResolvedTextLayoutInput, max_width: Option<f32>) -> Self::Layout {
        self.shape_platform(input, max_width, None)
    }

    /// The platform owns the line limit and the ellipsis: `shape_limited`'s
    /// cache callback is the respell path Parley needs, not this engine.
    fn shape_limited(
        &self,
        input: &ResolvedTextLayoutInput,
        max_width: Option<f32>,
        max_lines: NonZeroUsize,
        _shape: impl FnMut(&ResolvedTextLayoutInput, Option<f32>) -> Self::Layout,
    ) -> Self::Layout {
        self.shape_platform(input, max_width, Some(max_lines))
    }

    fn ink_extent(&self, layout: &Self::Layout, max_lines: Option<usize>) -> Option<(f32, f32)> {
        layout.ink_extent(max_lines)
    }

    fn record(
        &self,
        layout: &Self::Layout,
        max_lines: Option<usize>,
        x_shift: f32,
        scene: &mut Recording,
        _counters: &mut FrameWorkCounters,
    ) {
        // A platform layout records no glyph font: the font counters count
        // registrations on the engine's own tables, not the provider's.
        if max_lines == Some(0) {
            return;
        }
        let Some(resource) = layout.resource() else {
            // The zero-line layout draws nothing.
            return;
        };
        let clip = max_lines
            .filter(|limit| *limit < layout.line_count())
            .map(|limit| {
                let bounds = layout.bounds();
                let bottom = (0..limit)
                    .map(|line| f64::from(layout.line_metrics(line).line_height))
                    .sum::<f64>();
                Rect::new(bounds.x0, bounds.y0, bounds.x1, bottom)
            });
        scene.draw_platform_text(resource, Affine::translate((f64::from(x_shift), 0.0)), clip);
    }

    /// The engine holds no shape or ink caches of its own — [`TextService`]
    /// invalidates the layouts above it, and the provider's family-list
    /// cache drops its typefaces on registration. There is no fallback.
    #[cfg(all(target_arch = "wasm32", feature = "web"))]
    fn fonts_changed(&self) {}
}
