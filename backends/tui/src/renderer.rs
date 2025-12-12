use crossterm::style::{Attribute, Attributes, Color as TermColor, ContentStyle, StyledContent};
use waterui::{
    Color as UiColor,
    background::{Background, ForegroundColor},
    component::focu::Focused,
    gesture::GestureObserver,
    style::Shadow,
    view::ViewExt,
};
use waterui_core::{
    AnyView, Environment, IgnorableMetadata, Metadata, Native, Signal, Str, View, views::Views,
};
use waterui_layout::{
    container::{LazyContainer, FixedContainer},
    scroll::ScrollView,
    spacer::Spacer,
};
use waterui_navigation::NavigationView;
use waterui_text::{TextConfig, link::LinkConfig, styled::Style as TextStyle};

use crate::error::TuiError;

/// Represents a fully resolved frame ready to be drawn to the terminal.
#[derive(Debug, Default, Clone)]
pub struct RenderFrame {
    lines: Vec<RenderLine>,
}

impl RenderFrame {
    /// Pushes a new line with the provided indentation level.
    ///
    /// # Panics
    ///
    /// Panics if a line cannot be retrieved after insertion, which should be
    /// impossible unless the internal storage becomes inconsistent.
    pub fn push_line(&mut self, indent: usize) -> &mut RenderLine {
        self.lines.push(RenderLine::new(indent));
        self.lines
            .last_mut()
            .expect("line should be available after insertion")
    }

    /// Returns the set of lines recorded in this frame.
    #[must_use]
    pub fn lines(&self) -> &[RenderLine] {
        &self.lines
    }

    /// Appends a blank line.
    pub fn push_blank(&mut self) {
        self.lines.push(RenderLine::default());
    }
}

/// Representation of a single line in the terminal output.
#[derive(Debug, Default, Clone)]
pub struct RenderLine {
    segments: Vec<RenderSegment>,
}

impl RenderLine {
    fn new(indent: usize) -> Self {
        let mut segments = Vec::new();
        if indent > 0 {
            segments.push(RenderSegment::plain(" ".repeat(indent * 2)));
        }
        Self { segments }
    }

    /// Pushes a new segment onto the line.
    pub fn push(&mut self, segment: RenderSegment) {
        self.segments.push(segment);
    }

    /// Returns the list of segments contained in this line.
    pub fn segments(&self) -> &[RenderSegment] {
        &self.segments
    }
}

/// Atomic piece of content rendered on a line.
#[derive(Debug, Clone)]
pub struct RenderSegment {
    content: String,
    style: ContentStyle,
}

impl RenderSegment {
    /// Creates a plain (unstyled) segment.
    pub fn plain(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            style: ContentStyle::new(),
        }
    }

    /// Creates a segment with custom styling.
    pub fn styled(content: impl Into<String>, style: ContentStyle) -> Self {
        Self {
            content: content.into(),
            style,
        }
    }

    /// Borrows the raw text stored in this segment.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Converts this segment into a [`StyledContent`] for printing through crossterm.
    pub fn as_styled_content(&self) -> StyledContent<String> {
        self.style.apply(self.content.clone())
    }
}

/// Walks view trees and produces terminal friendly frames.
#[derive(Debug, Default)]
pub struct Renderer;

impl Renderer {
    /// Creates a new renderer instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Renders a view into a [`RenderFrame`].
    ///
    /// # Errors
    ///
    /// Returns an error if rendering the view tree fails for any reason.
    pub fn render<V: View>(&mut self, env: &Environment, view: V) -> Result<RenderFrame, TuiError> {
        let mut frame = RenderFrame::default();
        self.render_any(env, &mut frame, 0, AnyView::new(view))?;
        Ok(frame)
    }

    #[allow(clippy::manual_let_else, clippy::too_many_lines)]
    fn render_any(
        &mut self,
        env: &Environment,
        frame: &mut RenderFrame,
        indent: usize,
        view: AnyView,
    ) -> Result<(), TuiError> {
        let view = match view.downcast::<Str>() {
            Ok(text) => {
                Self::render_str(frame, indent, &text);
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<()>() {
            Ok(_) => return Ok(()),
            Err(view) => view,
        };

        let view = match view.downcast::<Native<TextConfig>>() {
            Ok(native) => {
                Self::render_text(env, frame, indent, &native.0);
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Native<LinkConfig>>() {
            Ok(native) => {
                self.render_link(env, frame, indent, native.0)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Metadata<Background>>() {
            Ok(metadata) => {
                self.render_any(env, frame, indent, metadata.content)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Metadata<ForegroundColor>>() {
            Ok(metadata) => {
                self.render_any(env, frame, indent, metadata.content)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Metadata<GestureObserver>>() {
            Ok(metadata) => {
                self.render_any(env, frame, indent, metadata.content)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Metadata<Shadow>>() {
            Ok(metadata) => {
                self.render_any(env, frame, indent, metadata.content)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Metadata<Focused>>() {
            Ok(metadata) => {
                self.render_any(env, frame, indent, metadata.content)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<IgnorableMetadata<Background>>() {
            Ok(metadata) => {
                self.render_any(env, frame, indent, metadata.content)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<FixedContainer>() {
            Ok(container) => {
                self.render_fixed_container(env, frame, indent, *container)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<LazyContainer>() {
            Ok(container) => {
                self.render_container(env, frame, indent, *container)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<ScrollView>() {
            Ok(scroll) => {
                self.render_scroll(env, frame, indent, *scroll)?;
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<Spacer>() {
            Ok(_) => {
                frame.push_blank();
                return Ok(());
            }
            Err(view) => view,
        };

        let view = match view.downcast::<NavigationView>() {
            Ok(nav) => {
                self.render_navigation(env, frame, indent, *nav)?;
                return Ok(());
            }
            Err(view) => view,
        };

        // Fallback: evaluate the view body and continue walking.
        let next = view.body(env);
        self.render_any(env, frame, indent, AnyView::new(next))
    }

    fn render_str(frame: &mut RenderFrame, indent: usize, text: &Str) {
        let line = frame.push_line(indent);
        line.push(RenderSegment::plain(text.to_string()));
    }

    fn render_text(env: &Environment, frame: &mut RenderFrame, indent: usize, config: &TextConfig) {
        let content = config.content.get();
        let line = frame.push_line(indent);
        let mut chunks = content.into_chunks();
        if chunks.is_empty() {
            chunks.push((Str::from(""), TextStyle::default()));
        }
        for (chunk, style) in chunks {
            let style = Self::content_style_from_text_style(env, &style);
            line.push(RenderSegment::styled(chunk.to_string(), style));
        }
    }

    fn render_link(
        &mut self,
        env: &Environment,
        frame: &mut RenderFrame,
        indent: usize,
        config: LinkConfig,
    ) -> Result<(), TuiError> {
        let line = frame.push_line(indent);
        line.push(RenderSegment::plain("[link] "));
        let url = config.url.get();
        line.push(RenderSegment::plain(url.to_string()));
        self.render_any(env, frame, indent + 1, config.label)?;
        Ok(())
    }

    fn render_fixed_container(
        &mut self,
        env: &Environment,
        frame: &mut RenderFrame,
        indent: usize,
        container: FixedContainer,
    ) -> Result<(), TuiError> {
        let (_layout, children) = container.into_inner();
        for child in children {
            self.render_any(env, frame, indent + 1, child)?;
        }
        Ok(())
    }

    fn render_container(
        &mut self,
        env: &Environment,
        frame: &mut RenderFrame,
        indent: usize,
        container: LazyContainer,
    ) -> Result<(), TuiError> {
        let (_layout, children) = container.into_inner();
        let len = children.len();
        for index in 0..len {
            if let Some(child) = children.get_view(index) {
                self.render_any(env, frame, indent + 1, child)?;
            }
        }
        Ok(())
    }

    fn render_scroll(
        &mut self,
        env: &Environment,
        frame: &mut RenderFrame,
        indent: usize,
        scroll: ScrollView,
    ) -> Result<(), TuiError> {
        let (axis, content) = scroll.into_inner();
        let line = frame.push_line(indent);
        line.push(RenderSegment::plain(format!("[scroll {axis:?}]")));
        self.render_any(env, frame, indent + 1, content)?;
        Ok(())
    }

    fn render_navigation(
        &mut self,
        env: &Environment,
        frame: &mut RenderFrame,
        indent: usize,
        navigation: NavigationView,
    ) -> Result<(), TuiError> {
        let line = frame.push_line(indent);
        line.push(RenderSegment::plain("[navigation]"));
        self.render_any(env, frame, indent + 1, navigation.bar.title.anyview())?;
        self.render_any(env, frame, indent + 1, navigation.content)?;
        Ok(())
    }

    fn content_style_from_text_style(env: &Environment, style: &TextStyle) -> ContentStyle {
        let mut content_style = ContentStyle::new();

        if let Some(color) = style.foreground.clone() {
            content_style.foreground_color = Some(color_to_terminal(env, &color));
        }

        if let Some(color) = style.background.clone() {
            content_style.background_color = Some(color_to_terminal(env, &color));
        }

        let mut attributes = Attributes::default();
        if let Some(attribute) = style.font.resolve(env).get().weight.into_bold_attribute() {
            attributes = attributes | Attributes::from(attribute);
        }
        if style.italic {
            attributes = attributes | Attributes::from(Attribute::Italic);
        }
        if style.underline {
            attributes = attributes | Attributes::from(Attribute::Underlined);
        }
        if style.strikethrough {
            attributes = attributes | Attributes::from(Attribute::CrossedOut);
        }
        content_style.attributes = attributes;
        content_style
    }
}

trait FontWeightExt {
    fn into_bold_attribute(self) -> Option<Attribute>;
}

impl FontWeightExt for waterui_text::font::FontWeight {
    fn into_bold_attribute(self) -> Option<Attribute> {
        match self {
            Self::Bold | Self::SemiBold | Self::UltraBold | Self::Black => Some(Attribute::Bold),
            _ => None,
        }
    }
}

fn color_to_terminal(env: &Environment, color: &UiColor) -> TermColor {
    let resolved = color.resolve(env).get();
    TermColor::Rgb {
        r: clamp_color_component(resolved.red),
        g: clamp_color_component(resolved.green),
        b: clamp_color_component(resolved.blue),
    }
}

fn clamp_color_component(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    {
        (value * 255.0).round() as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_core::Environment;
    use waterui_text::text;

    #[test]
    fn render_plain_text() {
        let mut renderer = Renderer::new();
        let env = Environment::new();
        let frame = renderer
            .render(&env, text("hello"))
            .expect("render should succeed");
        assert_eq!(frame.lines().len(), 1);
        assert_eq!(frame.lines()[0].segments()[0].content(), "hello");
    }
}
