use std::{mem, str::FromStr};

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag};
use waterui_color::Blue;
use waterui_core::{Environment, View};
use waterui_layout::stack::{HStack, HorizontalAlignment, VStack, hstack};
use waterui_media::{Url, photo::photo as media_photo};
use waterui_str::Str;
use waterui_text::{
    Text,
    highlight::Language,
    styled::{MarkdownInlineBuilder, Style, StyledStr, heading_style},
    text,
};

use crate::{
    ViewExt,
    component::table::{col, table},
    widget::{self, Divider},
};

/// Rich text widget for displaying formatted content.
#[derive(Debug, Default, Clone)]
pub struct RichText {
    elements: Vec<RichTextElement>,
}

/// Includes a Markdown file as a [`RichText`] widget at compile time.
#[macro_export]
macro_rules! include_markdown {
    ($path:expr) => {
        $crate::widget::rich_text::RichText::from_markdown(::core::include_str!($path))
    };
}

impl RichText {
    /// Creates a new [`RichText`] widget from the provided elements.
    #[must_use]
    pub fn new(elements: impl Into<Vec<RichTextElement>>) -> Self {
        Self {
            elements: elements.into(),
        }
    }

    /// Parses a Markdown document into a [`RichText`] tree.
    #[must_use]
    pub fn from_markdown(markdown: &str) -> Self {
        Self {
            elements: parse_markdown(markdown),
        }
    }

    /// Returns the rich text elements for inspection or testing.
    #[must_use]
    pub fn elements(&self) -> &[RichTextElement] {
        &self.elements
    }
}

impl FromIterator<RichTextElement> for RichText {
    fn from_iter<T: IntoIterator<Item = RichTextElement>>(iter: T) -> Self {
        Self {
            elements: iter.into_iter().collect(),
        }
    }
}

/// Convenience constructor for creating a [`RichText`] view inline.
#[must_use]
pub fn rich_text(elements: impl Into<Vec<RichTextElement>>) -> RichText {
    RichText::new(elements)
}

/// Represents different types of rich text elements.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RichTextElement {
    /// Plain text with styling.
    Text(StyledStr),
    /// A horizontal divider.
    Divider,
    /// A hyperlink.
    Link {
        /// The link label.
        label: StyledStr,
        /// The link URL.
        url: Str,
    },
    /// An image.
    Image {
        /// Image source URL.
        src: Str,
        /// Alternative text.
        alt: Str,
    },
    /// A table with headers and rows.
    Table {
        /// Table headers.
        headers: Vec<Self>,
        /// Table rows.
        rows: Vec<Vec<Self>>,
    },
    /// A list of items.
    List {
        /// List items.
        items: Vec<Self>,
        /// Whether the list is ordered.
        ordered: bool,
    },
    /// A code block.
    Code {
        /// The code content.
        code: Str,
        /// Optional language specification.
        language: Language,
    },
    /// A quotation block.
    Quote {
        /// The quoted content.
        content: Vec<Self>,
    },
    /// A group of elements arranged either inline (horizontally) or stacked
    /// vertically.
    Group {
        /// Child elements.
        elements: Vec<Self>,
        /// When `true`, children are rendered in a horizontal stack.
        inline: bool,
    },
}

impl View for RichTextElement {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Self::Text(s) => text(s).anyview(),
            Self::Link { label, url } => crate::component::link::link(text(label), url).anyview(),
            Self::Image { src, alt: _ } => {
                Url::parse(&*src).map_or_else(|| ().anyview(), |url| media_photo(url).anyview())
            }
            Self::Table { headers, rows } => {
                // Convert row-based data to column-based for native Table
                let num_cols = headers.len();
                let columns = (0..num_cols)
                    .map(|col_idx| {
                        let header = element_to_text(&headers[col_idx]);
                        let row_texts: Vec<Text> = rows
                            .iter()
                            .filter_map(|row| row.get(col_idx).map(element_to_text))
                            .collect();
                        col(header, row_texts)
                    })
                    .collect::<Vec<_>>();
                table(columns).anyview()
            }
            Self::List { items, ordered } => render_list(items.as_slice(), ordered).anyview(),
            Self::Code { code, language } => widget::code(language, code).anyview(),
            Self::Quote { content } => quote(content).anyview(),
            Self::Group { elements, inline } => {
                if inline {
                    // Inline content already contains explicit whitespace in the source text.
                    // Use zero stack spacing to avoid double-spacing between adjacent spans.
                    elements
                        .into_iter()
                        .collect::<HStack<_>>()
                        .spacing(0.0)
                        .anyview()
                } else {
                    VStack::from_iter(elements).anyview()
                }
            }
            Self::Divider => Divider.anyview(),
        }
    }
}

impl View for RichText {
    fn body(self, _env: &Environment) -> impl View {
        // Use Leading alignment so code blocks and other elements align properly
        VStack::from_iter(self.elements).alignment(HorizontalAlignment::Leading)
    }
}

fn render_list(items: &[RichTextElement], ordered: bool) -> impl View {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = if ordered {
                format!("{}. ", i + 1)
            } else {
                "• ".to_string()
            };
            hstack((text(marker), item.clone()))
        })
        .collect::<VStack<_>>()
        .alignment(HorizontalAlignment::Leading)
}

fn quote(content: Vec<RichTextElement>) -> impl View {
    // Quote marker: fixed width, stretch to fill height (use max_height to trigger stretch)
    let quote_marker = Blue.width(4.0).max_height(f32::MAX);
    hstack((
        quote_marker,
        VStack::from_iter(content).alignment(HorizontalAlignment::Leading),
    ))
}

/// Converts a `RichTextElement` to plain `Text` for use in native Table columns.
fn element_to_text(element: &RichTextElement) -> Text {
    match element {
        RichTextElement::Text(styled) => Text::from(styled.clone()),
        RichTextElement::Link { label, .. } => Text::from(label.clone()),
        RichTextElement::Group { elements, .. } => {
            // Concatenate all text from group elements
            let combined: String = elements.iter().map(element_to_plain_text).collect();
            Text::from(combined)
        }
        _ => Text::from(""),
    }
}

/// Extracts plain text string from a `RichTextElement`.
fn element_to_plain_text(element: &RichTextElement) -> String {
    match element {
        RichTextElement::Text(styled) => styled.to_plain().to_string(),
        RichTextElement::Link { label, .. } => label.to_plain().to_string(),
        RichTextElement::Group { elements, .. } => {
            elements.iter().map(element_to_plain_text).collect()
        }
        _ => String::new(),
    }
}

#[allow(clippy::too_many_lines)]
fn parse_markdown(markdown: &str) -> Vec<RichTextElement> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(markdown, options);

    let mut stack = vec![Container::Root(Vec::new())];

    for event in parser {
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => {
                    flush_list_item_inline(&mut stack);
                    stack.push(Container::Paragraph(InlineGroup::default()));
                }
                Tag::Heading { level, .. } => {
                    flush_list_item_inline(&mut stack);
                    stack.push(Container::Heading(InlineGroup::with_style(heading_style(
                        level,
                    ))));
                }
                Tag::BlockQuote(_) => {
                    flush_list_item_inline(&mut stack);
                    stack.push(Container::BlockQuote(Vec::new()));
                }
                Tag::List(start) => {
                    flush_list_item_inline(&mut stack);
                    stack.push(Container::List {
                        ordered: start.is_some(),
                        items: Vec::new(),
                    });
                }
                Tag::Item => stack.push(Container::ListItem {
                    blocks: Vec::new(),
                    inline: InlineGroup::default(),
                }),
                Tag::CodeBlock(kind) => {
                    flush_list_item_inline(&mut stack);
                    let language = language_from_kind(&kind);
                    stack.push(Container::CodeBlock {
                        language,
                        code: String::new(),
                    });
                }
                Tag::Table(_) => {
                    flush_list_item_inline(&mut stack);
                    stack.push(Container::Table {
                        headers: Vec::new(),
                        rows: Vec::new(),
                        in_head: false,
                    });
                }
                Tag::TableHead => {
                    // TableHead contains cells directly (no TableRow wrapper)
                    // Push a TableRow container to collect header cells
                    if let Some(idx) = current_table_index(&stack)
                        && let Container::Table { in_head, .. } = &mut stack[idx]
                    {
                        *in_head = true;
                    }
                    stack.push(Container::TableRow { cells: Vec::new() });
                }
                Tag::TableRow => stack.push(Container::TableRow { cells: Vec::new() }),
                Tag::TableCell => {
                    let header_cell = current_table_index(&stack)
                        .and_then(|idx| match &stack[idx] {
                            Container::Table { in_head, .. } => Some(*in_head),
                            _ => None,
                        })
                        .unwrap_or(false);

                    let style = if header_cell {
                        Style::default().bold()
                    } else {
                        Style::default()
                    };

                    stack.push(Container::TableCell(InlineGroup::with_style(style)));
                }
                Tag::Emphasis => {
                    if let Some(mut sink) = current_inline_sink(&mut stack) {
                        sink.enter_emphasis();
                    }
                }
                Tag::Strong => {
                    if let Some(mut sink) = current_inline_sink(&mut stack) {
                        sink.enter_strong();
                    }
                }
                Tag::Link { dest_url, .. } => {
                    stack.push(Container::InlineLink {
                        url: Str::from(dest_url.into_string()),
                        label: MarkdownInlineBuilder::new(),
                    });
                }
                Tag::Image { dest_url, .. } => {
                    stack.push(Container::InlineImage {
                        url: Str::from(dest_url.into_string()),
                        alt: MarkdownInlineBuilder::new(),
                    });
                }

                _ => {}
            },
            Event::End(tag) => match tag {
                pulldown_cmark::TagEnd::Paragraph => {
                    if let Some(Container::Paragraph(group)) = stack.pop() {
                        let element = collapse_inline(group.finish());
                        push_to_parent(&mut stack, element);
                    }
                }
                pulldown_cmark::TagEnd::Heading(_) => {
                    if let Some(Container::Heading(group)) = stack.pop() {
                        let element = collapse_inline(group.finish());
                        push_to_parent(&mut stack, element);
                    }
                }
                pulldown_cmark::TagEnd::BlockQuote(_) => {
                    if let Some(Container::BlockQuote(content)) = stack.pop() {
                        push_to_parent(&mut stack, RichTextElement::Quote { content });
                    }
                }
                pulldown_cmark::TagEnd::List(_) => {
                    if let Some(Container::List { ordered, items }) = stack.pop() {
                        push_to_parent(&mut stack, RichTextElement::List { items, ordered });
                    }
                }
                pulldown_cmark::TagEnd::Item => {
                    if let Some(Container::ListItem {
                        mut blocks,
                        mut inline,
                    }) = stack.pop()
                    {
                        if let Some(segments) = inline.take() {
                            blocks.push(collapse_inline(segments));
                        }

                        let element = collapse_block(blocks);
                        if let Some(Container::List { items, .. }) = stack.last_mut() {
                            items.push(element);
                        }
                    }
                }
                pulldown_cmark::TagEnd::CodeBlock => {
                    if let Some(Container::CodeBlock { language, code }) = stack.pop() {
                        push_to_parent(
                            &mut stack,
                            RichTextElement::Code {
                                language,
                                code: code.into(),
                            },
                        );
                    }
                }
                pulldown_cmark::TagEnd::Table => {
                    if let Some(Container::Table { headers, rows, .. }) = stack.pop() {
                        push_to_parent(&mut stack, RichTextElement::Table { headers, rows });
                    }
                }
                pulldown_cmark::TagEnd::TableHead => {
                    // Pop the header row we pushed in Tag::TableHead
                    if let Some(Container::TableRow { cells }) = stack.pop()
                        && let Some(idx) = current_table_index(&stack)
                        && let Container::Table {
                            headers, in_head, ..
                        } = &mut stack[idx]
                    {
                        *headers = cells;
                        *in_head = false;
                    }
                }
                pulldown_cmark::TagEnd::TableRow => {
                    if let Some(Container::TableRow { cells }) = stack.pop()
                        && let Some(idx) = current_table_index(&stack)
                        && let Container::Table {
                            headers,
                            rows,
                            in_head,
                        } = &mut stack[idx]
                    {
                        if *in_head && headers.is_empty() {
                            *headers = cells;
                        } else {
                            rows.push(cells);
                        }
                    }
                }
                pulldown_cmark::TagEnd::TableCell => {
                    if let Some(Container::TableCell(group)) = stack.pop() {
                        let cell = collapse_inline(group.finish());
                        if let Some(Container::TableRow { cells }) = stack.last_mut() {
                            cells.push(cell);
                        }
                    }
                }
                pulldown_cmark::TagEnd::Link => {
                    if let Some(Container::InlineLink { url, label }) = stack.pop() {
                        let element = RichTextElement::Link {
                            label: label.finish(),
                            url,
                        };
                        push_inline_element(&mut stack, element);
                    }
                }
                pulldown_cmark::TagEnd::Image => {
                    if let Some(Container::InlineImage { url, alt }) = stack.pop() {
                        let alt_text = alt.finish().to_plain();
                        let element = RichTextElement::Image {
                            src: url,
                            alt: alt_text,
                        };
                        push_inline_element(&mut stack, element);
                    }
                }
                pulldown_cmark::TagEnd::Emphasis | pulldown_cmark::TagEnd::Strong => {
                    if let Some(mut sink) = current_inline_sink(&mut stack) {
                        sink.exit();
                    }
                }
                _ => {}
            },

            Event::Text(text) => match stack.last_mut() {
                Some(Container::CodeBlock { code, .. }) => code.push_str(text.as_ref()),
                _ => {
                    if let Some(mut sink) = current_inline_sink(&mut stack) {
                        sink.push_text(text.as_ref());
                    } else {
                        push_to_parent(
                            &mut stack,
                            RichTextElement::Text(StyledStr::plain(text.as_ref().to_string())),
                        );
                    }
                }
            },
            Event::Code(text)
            | Event::Html(text)
            | Event::FootnoteReference(text)
            | Event::InlineMath(text)
            | Event::DisplayMath(text)
            | Event::InlineHtml(text) => {
                if let Some(mut sink) = current_inline_sink(&mut stack) {
                    sink.push_text(text.as_ref());
                } else {
                    push_to_parent(
                        &mut stack,
                        RichTextElement::Text(StyledStr::plain(text.as_ref().to_string())),
                    );
                }
            }
            Event::SoftBreak => {
                if let Some(Container::CodeBlock { code, .. }) = stack.last_mut() {
                    code.push('\n');
                } else if let Some(mut sink) = current_inline_sink(&mut stack) {
                    sink.soft_break();
                }
            }
            Event::HardBreak => {
                if let Some(Container::CodeBlock { code, .. }) = stack.last_mut() {
                    code.push('\n');
                } else if let Some(mut sink) = current_inline_sink(&mut stack) {
                    sink.hard_break();
                }
            }

            Event::TaskListMarker(checked) => {
                if let Some(mut sink) = current_inline_sink(&mut stack) {
                    sink.push_text(if checked { "[x] " } else { "[ ] " });
                }
            }
            Event::Rule => {
                push_to_parent(&mut stack, RichTextElement::Divider);
            }
        }
    }

    match stack.pop() {
        Some(Container::Root(elements)) => elements,
        _ => Vec::new(),
    }
}

fn language_from_kind(kind: &CodeBlockKind) -> Language {
    match kind {
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .and_then(|token| Language::from_str(token).ok())
            .unwrap_or(Language::Plaintext),
        CodeBlockKind::Indented => Language::Plaintext,
    }
}

fn collapse_inline(mut elements: Vec<RichTextElement>) -> RichTextElement {
    match elements.len() {
        0 => RichTextElement::Text(StyledStr::empty()),
        1 => elements.pop().expect("elements should have one item"),
        _ => RichTextElement::Group {
            elements,
            inline: true,
        },
    }
}

fn collapse_block(mut elements: Vec<RichTextElement>) -> RichTextElement {
    match elements.len() {
        0 => RichTextElement::Text(StyledStr::empty()),
        1 => elements.pop().expect("elements should have one item"),
        _ => RichTextElement::Group {
            elements,
            inline: false,
        },
    }
}

fn current_table_index(stack: &[Container]) -> Option<usize> {
    stack
        .iter()
        .rposition(|container| matches!(container, Container::Table { .. }))
}

fn push_to_parent(stack: &mut [Container], element: RichTextElement) {
    if let Some(parent) = stack.last_mut() {
        match parent {
            Container::Root(elements) | Container::BlockQuote(elements) => {
                elements.push(element);
            }
            Container::List { items, .. } => items.push(element),
            Container::ListItem { blocks, .. } => blocks.push(element),
            Container::TableRow { cells } => cells.push(element),
            _ => {}
        }
    }
}

fn push_inline_element(stack: &mut [Container], element: RichTextElement) {
    for container in stack.iter_mut().rev() {
        match container {
            Container::Paragraph(group)
            | Container::Heading(group)
            | Container::TableCell(group)
            | Container::ListItem { inline: group, .. } => {
                group.push_element(element);
                return;
            }
            _ => {}
        }
    }

    push_to_parent(stack, element);
}

fn flush_list_item_inline(stack: &mut [Container]) {
    if let Some(Container::ListItem { inline, blocks }) = stack.last_mut()
        && let Some(segments) = inline.take()
    {
        blocks.push(collapse_inline(segments));
    }
}

enum InlineSinkMut<'a> {
    Group(&'a mut InlineGroup),
    Builder(&'a mut MarkdownInlineBuilder),
}

impl InlineSinkMut<'_> {
    fn push_text(&mut self, text: &str) {
        match self {
            Self::Group(group) => group.push_text(text),
            Self::Builder(builder) => builder.push_text(text),
        }
    }

    fn soft_break(&mut self) {
        match self {
            Self::Group(group) => group.soft_break(),
            Self::Builder(builder) => builder.push_soft_break(),
        }
    }

    fn hard_break(&mut self) {
        match self {
            Self::Group(group) => group.hard_break(),
            Self::Builder(builder) => builder.push_hard_break(),
        }
    }

    fn enter_emphasis(&mut self) {
        match self {
            Self::Group(group) => group.enter_emphasis(),
            Self::Builder(builder) => builder.enter_emphasis(),
        }
    }

    fn enter_strong(&mut self) {
        match self {
            Self::Group(group) => group.enter_strong(),
            Self::Builder(builder) => builder.enter_strong(),
        }
    }

    fn exit(&mut self) {
        match self {
            Self::Group(group) => group.exit_style(),
            Self::Builder(builder) => builder.exit(),
        }
    }
}

fn current_inline_sink(stack: &mut [Container]) -> Option<InlineSinkMut<'_>> {
    for container in stack.iter_mut().rev() {
        match container {
            Container::InlineLink { label, .. } => return Some(InlineSinkMut::Builder(label)),
            Container::InlineImage { alt, .. } => return Some(InlineSinkMut::Builder(alt)),
            Container::Paragraph(group)
            | Container::Heading(group)
            | Container::TableCell(group)
            | Container::ListItem { inline: group, .. } => {
                return Some(InlineSinkMut::Group(group));
            }
            _ => {}
        }
    }

    None
}

#[derive(Debug)]
enum Container {
    Root(Vec<RichTextElement>),
    Paragraph(InlineGroup),
    Heading(InlineGroup),
    BlockQuote(Vec<RichTextElement>),
    List {
        ordered: bool,
        items: Vec<RichTextElement>,
    },
    ListItem {
        blocks: Vec<RichTextElement>,
        inline: InlineGroup,
    },
    InlineLink {
        url: Str,
        label: MarkdownInlineBuilder,
    },
    InlineImage {
        url: Str,
        alt: MarkdownInlineBuilder,
    },
    CodeBlock {
        language: Language,
        code: String,
    },
    Table {
        headers: Vec<RichTextElement>,
        rows: Vec<Vec<RichTextElement>>,
        in_head: bool,
    },
    TableRow {
        cells: Vec<RichTextElement>,
    },
    TableCell(InlineGroup),
}

#[derive(Debug)]
struct InlineGroup {
    builder: MarkdownInlineBuilder,
    segments: Vec<RichTextElement>,
}

impl InlineGroup {
    fn with_style(style: Style) -> Self {
        Self {
            builder: MarkdownInlineBuilder::with_base_style(style),
            segments: Vec::new(),
        }
    }

    fn push_text(&mut self, text: &str) {
        self.builder.push_text(text);
    }

    fn soft_break(&mut self) {
        self.builder.push_soft_break();
    }

    fn hard_break(&mut self) {
        self.builder.push_hard_break();
    }

    fn enter_emphasis(&mut self) {
        self.builder.enter_emphasis();
    }

    fn enter_strong(&mut self) {
        self.builder.enter_strong();
    }

    fn exit_style(&mut self) {
        self.builder.exit();
    }

    fn push_element(&mut self, element: RichTextElement) {
        if let Some(text) = self.builder.take() {
            self.segments.push(RichTextElement::Text(text));
        }
        self.segments.push(element);
    }

    fn take(&mut self) -> Option<Vec<RichTextElement>> {
        if let Some(text) = self.builder.take() {
            self.segments.push(RichTextElement::Text(text));
        }

        if self.segments.is_empty() {
            return None;
        }

        let mut segments = Vec::new();
        mem::swap(&mut segments, &mut self.segments);
        self.builder = MarkdownInlineBuilder::with_base_style(self.builder.base_style());
        Some(segments)
    }

    fn finish(mut self) -> Vec<RichTextElement> {
        if let Some(text) = self.builder.take() {
            self.segments.push(RichTextElement::Text(text));
        }
        self.segments
    }
}

impl Default for InlineGroup {
    fn default() -> Self {
        Self {
            builder: MarkdownInlineBuilder::new(),
            segments: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_markdown_into_rich_text() {
        let markdown = "# Heading\n\nA paragraph with **bold** and [link](https://example.com).\n\n- Item 1\n- Item 2\n\n| Col A | Col B |\n| ----- | ----- |\n| 1 | 2 |\n";

        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();
        assert!(!elements.is_empty());

        assert!(matches!(elements[0], RichTextElement::Text(_)));
        assert!(matches!(elements[1], RichTextElement::Group { .. }));
        assert!(matches!(elements[2], RichTextElement::List { .. }));
        assert!(matches!(elements[3], RichTextElement::Table { .. }));
    }

    #[test]
    fn parses_code_block() {
        let markdown = r#"# WaterUI Markdown Example
This is an example of using **WaterUI** to render Markdown content in a cross-platform application.

Supports **bold**, *italic*, and `code` text styles. blocks

```rust

fn main() {
    println!("Hello, Markdown!");
}
```

"#;

        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();

        println!("Total elements: {}", elements.len());
        for (i, el) in elements.iter().enumerate() {
            println!("Element {}: {:?}", i, std::mem::discriminant(el));
            match el {
                RichTextElement::Code { language, code } => {
                    println!("  Code: lang={language:?}, code={code:?}");
                }
                RichTextElement::Text(s) => {
                    println!("  Text: {:?}", s.to_plain());
                }
                _ => {}
            }
        }

        // Should have a Code element
        let has_code = elements
            .iter()
            .any(|el| matches!(el, RichTextElement::Code { .. }));
        assert!(has_code, "Expected a Code element in the parsed markdown");
    }

    #[test]
    fn parses_table() {
        let markdown = r"
| Platform | Backend | Status |
| -------- | ------- | ------ |
| iOS | SwiftUI | Ready |
| macOS | AppKit | Ready |
";

        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();

        let has_table = elements
            .iter()
            .any(|el| matches!(el, RichTextElement::Table { .. }));
        assert!(has_table, "Expected a Table element in the parsed markdown");

        // Verify table structure
        for el in elements {
            if let RichTextElement::Table { headers, rows } = el {
                assert_eq!(headers.len(), 3, "Expected 3 headers");
                assert_eq!(rows.len(), 2, "Expected 2 rows");
            }
        }
    }

    #[test]
    fn parses_link_text_correctly() {
        let markdown =
            "Visit [WaterUI on GitHub](https://github.com/water-rs/waterui) for more information.";
        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();

        println!("Elements: {elements:#?}");

        // Should have one Group element (inline paragraph)
        assert_eq!(elements.len(), 1);

        if let RichTextElement::Group {
            elements: inner,
            inline,
        } = &elements[0]
        {
            assert!(inline, "Should be inline group");
            println!("Inner elements: {inner:#?}");

            // Find the Link element
            let link = inner
                .iter()
                .find(|el| matches!(el, RichTextElement::Link { .. }));
            assert!(link.is_some(), "Should have a Link element");

            if let Some(RichTextElement::Link { label, url }) = link {
                let label_text = label.to_plain();
                println!("Link label: '{label_text}'");
                println!("Link URL: '{url}'");
                assert_eq!(
                    &*label_text, "WaterUI on GitHub",
                    "Link text should be complete"
                );
            }
        } else {
            panic!("Expected Group element, got {:?}", elements[0]);
        }
    }
}
