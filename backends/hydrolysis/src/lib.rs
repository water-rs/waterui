use waterui_core::Str;
use waterui_text::font::ResolvedFont;

pub trait Node {
    fn render(&self) -> impl Iterator<Item = RenderCommand>;
}

pub enum RenderCommand {
    Text { font: ResolvedFont, content: Str },
}
