//! Canvas text: the styled text a canvas names, laid out by the session's
//! text engine, the one every `Text` leaf goes through.

use waterui_core::layout::Size;
use waterui_graphics::TextRequest;
use waterui_text::styled::StyledStr;

use super::engine::{TextEngine, TextLayout};
use super::input::resolve_text_layout_input;
use super::service::TextService;

/// A canvas text laid out, and the box it filled.
pub struct CanvasLayout<L> {
    pub(crate) layout: L,
    pub(crate) size: Size,
}

/// Lays out `request`'s text at its width and alignment, in its
/// environment; `None` for a text with no lines, which draws nothing.
///
/// The box is as wide as the requested width, or as the longest line when
/// there is none.
///
/// # Panics
///
/// When the canvas text was not made from `waterui-text`'s styled string:
/// that crate is the only one that builds it.
pub fn shape<E: TextEngine>(
    service: &TextService<E>,
    request: &TextRequest,
) -> Option<CanvasLayout<E::Layout>> {
    let styled = request
        .text()
        .get::<StyledStr>()
        .expect("canvas text is the styled text `waterui-text` converts into");
    let placement = request.placement();
    let input = resolve_text_layout_input(styled, placement.alignment(), request.environment());
    let layout = service.shape(&input, placement.get_width());
    let lines = layout.line_count();
    if lines == 0 {
        return None;
    }
    let width = placement.get_width().unwrap_or_else(|| {
        (0..lines)
            .map(|line| layout.line_metrics(line).advance)
            .fold(0.0, f32::max)
    });
    let size = Size::new(width, layout.height());
    Some(CanvasLayout { layout, size })
}
