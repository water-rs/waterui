use std::{mem, str::FromStr};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag};
use waterui_core::{AnyView, Environment, View};
use waterui_graphics::color::Blue;
use waterui_layout::spacer::spacer;
use waterui_layout::stack::{HStack, HorizontalAlignment, VStack, hstack};
#[cfg(feature = "media")]
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
        /// Per-column alignment metadata parsed from Markdown.
        alignments: Vec<MarkdownTableAlignment>,
    },
    /// A list of items.
    List {
        /// List items.
        items: Vec<Self>,
        /// Whether the list is ordered.
        ordered: bool,
        /// Start index for ordered lists.
        start: usize,
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

/// Column alignment metadata for Markdown tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkdownTableAlignment {
    /// No explicit alignment.
    None,
    /// Left aligned.
    Left,
    /// Center aligned.
    Center,
    /// Right aligned.
    Right,
}

impl View for RichTextElement {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Self::Text(s) => text(s).anyview(),
            Self::Link { label, url } => crate::component::link::link(text(label), url).anyview(),
            Self::Image { src, alt: _ } => render_image(src),
            Self::Table {
                headers,
                rows,
                alignments,
            } => render_table(headers, rows, alignments).anyview(),
            Self::List {
                items,
                ordered,
                start,
            } => render_list(items.as_slice(), ordered, start).anyview(),
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

fn render_image(src: Str) -> AnyView {
    #[cfg(feature = "media")]
    {
        let url = Url::parse(&*src)
            .unwrap_or_else(|| panic!("RichText image source is not a valid URL: {src}"));
        return media_photo(url).anyview();
    }

    #[cfg(not(feature = "media"))]
    {
        panic!("RichText image rendering requires the `media` feature: {src}");
    }
}

impl View for RichText {
    fn body(self, _env: &Environment) -> impl View {
        // Use Leading alignment so code blocks and other elements align properly
        VStack::from_iter(self.elements).alignment(HorizontalAlignment::Leading)
    }
}

fn render_list(items: &[RichTextElement], ordered: bool, start: usize) -> impl View {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let marker = if ordered {
                format!("{}. ", start + i)
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

fn render_table(
    headers: Vec<RichTextElement>,
    rows: Vec<Vec<RichTextElement>>,
    alignments: Vec<MarkdownTableAlignment>,
) -> AnyView {
    let col_count = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or_default());
    if col_count == 0 {
        return AnyView::new(());
    }

    let header_row: Vec<AnyView> = (0..col_count)
        .map(|col_idx| {
            let header = headers
                .get(col_idx)
                .map_or_else(|| Text::from(""), element_to_text)
                .bold();
            table_cell(
                header,
                alignments
                    .get(col_idx)
                    .copied()
                    .unwrap_or(MarkdownTableAlignment::None),
            )
        })
        .collect();

    let mut row_views = Vec::with_capacity(rows.len() + 2);
    row_views.push(AnyView::new(
        HStack::from_iter(header_row)
            .spacing(12.0)
            .alignment(waterui_layout::stack::VerticalAlignment::Top),
    ));
    row_views.push(AnyView::new(Divider));

    for row in rows {
        let cells: Vec<AnyView> = (0..col_count)
            .map(|col_idx| {
                let cell = row
                    .get(col_idx)
                    .map_or_else(|| Text::from(""), element_to_text);
                table_cell(
                    cell,
                    alignments
                        .get(col_idx)
                        .copied()
                        .unwrap_or(MarkdownTableAlignment::None),
                )
            })
            .collect();
        row_views.push(AnyView::new(
            HStack::from_iter(cells)
                .spacing(12.0)
                .alignment(waterui_layout::stack::VerticalAlignment::Top),
        ));
    }

    AnyView::new(
        VStack::from_iter(row_views)
            .spacing(6.0)
            .alignment(HorizontalAlignment::Leading),
    )
}

fn table_cell(content: Text, alignment: MarkdownTableAlignment) -> AnyView {
    match alignment {
        MarkdownTableAlignment::None | MarkdownTableAlignment::Left => {
            AnyView::new(content.max_width(f32::MAX))
        }
        MarkdownTableAlignment::Center => {
            AnyView::new(hstack((spacer(), content, spacer())).max_width(f32::MAX))
        }
        MarkdownTableAlignment::Right => {
            AnyView::new(hstack((spacer(), content)).max_width(f32::MAX))
        }
    }
}

/// Converts a `RichTextElement` to plain `Text` for table cells.
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
                        start: ordered_list_start(start),
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
                Tag::Table(alignments) => {
                    flush_list_item_inline(&mut stack);
                    stack.push(Container::Table {
                        headers: Vec::new(),
                        rows: Vec::new(),
                        alignments: alignments.into_iter().map(map_table_alignment).collect(),
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
                Tag::Strikethrough => {
                    if let Some(mut sink) = current_inline_sink(&mut stack) {
                        sink.enter_strikethrough();
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
                    if let Some(Container::List {
                        ordered,
                        start,
                        items,
                    }) = stack.pop()
                    {
                        push_to_parent(
                            &mut stack,
                            RichTextElement::List {
                                items,
                                ordered,
                                start,
                            },
                        );
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
                    if let Some(Container::Table {
                        headers,
                        rows,
                        alignments,
                        ..
                    }) = stack.pop()
                    {
                        push_to_parent(
                            &mut stack,
                            RichTextElement::Table {
                                headers,
                                rows,
                                alignments,
                            },
                        );
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
                            ..
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
                pulldown_cmark::TagEnd::Emphasis
                | pulldown_cmark::TagEnd::Strong
                | pulldown_cmark::TagEnd::Strikethrough => {
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
            Event::Code(text) => {
                if let Some(mut sink) = current_inline_sink(&mut stack) {
                    sink.push_inline_code(text.as_ref());
                } else {
                    let mut styled = StyledStr::empty();
                    styled.push(text.as_ref().to_string(), inline_code_style());
                    push_to_parent(&mut stack, RichTextElement::Text(styled));
                }
            }
            Event::Html(text)
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

fn inline_code_style() -> Style {
    Style::default()
        .font(waterui_text::font::Font::from(waterui_text::font::Body).family("monospace"))
        .background(waterui_graphics::color::Srgb::new_u8(236, 239, 241))
}

fn ordered_list_start(start: Option<u64>) -> usize {
    start
        .and_then(|v| usize::try_from(v).ok())
        .filter(|v| *v >= 1)
        .unwrap_or(1)
}

const fn map_table_alignment(alignment: Alignment) -> MarkdownTableAlignment {
    match alignment {
        Alignment::None => MarkdownTableAlignment::None,
        Alignment::Left => MarkdownTableAlignment::Left,
        Alignment::Center => MarkdownTableAlignment::Center,
        Alignment::Right => MarkdownTableAlignment::Right,
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

    fn enter_strikethrough(&mut self) {
        match self {
            Self::Group(group) => group.enter_strikethrough(),
            Self::Builder(builder) => builder.enter_strikethrough(),
        }
    }

    fn push_inline_code(&mut self, text: &str) {
        match self {
            Self::Group(group) => group.push_inline_code(text),
            Self::Builder(builder) => builder.push_inline_code(text),
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
        start: usize,
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
        alignments: Vec<MarkdownTableAlignment>,
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

    fn enter_strikethrough(&mut self) {
        self.builder.enter_strikethrough();
    }

    fn push_inline_code(&mut self, text: &str) {
        self.builder.push_inline_code(text);
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
            if let RichTextElement::Table {
                headers,
                rows,
                alignments,
            } = el
            {
                assert_eq!(headers.len(), 3, "Expected 3 headers");
                assert_eq!(rows.len(), 2, "Expected 2 rows");
                assert_eq!(alignments.len(), 3, "Expected 3 alignment slots");
            }
        }
    }

    #[test]
    fn ordered_list_respects_start_index() {
        let markdown = "5. five\n6. six";
        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();
        let list = elements
            .iter()
            .find_map(|el| match el {
                RichTextElement::List {
                    ordered,
                    start,
                    items,
                } => Some((*ordered, *start, items.len())),
                _ => None,
            })
            .expect("Expected parsed list");
        assert!(list.0, "List should be ordered");
        assert_eq!(list.1, 5, "Ordered list should keep explicit start index");
        assert_eq!(list.2, 2, "Ordered list should include two items");
    }

    #[test]
    fn inline_code_has_code_style() {
        let markdown = "Use `cargo run`";
        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();

        fn find_code_chunk(elements: &[RichTextElement]) -> bool {
            elements.iter().any(|el| match el {
                RichTextElement::Text(styled) => styled
                    .clone()
                    .into_chunks()
                    .into_iter()
                    .any(|(txt, style)| txt.as_str() == "cargo run" && style.background.is_some()),
                RichTextElement::Group { elements, .. } => find_code_chunk(elements),
                _ => false,
            })
        }

        let code_styled = find_code_chunk(elements);
        assert!(
            code_styled,
            "Expected inline code chunk with background style"
        );
    }

    #[test]
    fn parses_link_text_correctly() {
        let markdown =
            "Visit [WaterUI on GitHub](https://github.com/water-rs/waterui) for more information.";
        let rich = RichText::from_markdown(markdown);
        let elements = rich.elements();

        // Should have one Group element (inline paragraph)
        assert_eq!(elements.len(), 1);

        if let RichTextElement::Group {
            elements: inner,
            inline,
        } = &elements[0]
        {
            assert!(inline, "Should be inline group");

            // Find the Link element
            let link = inner
                .iter()
                .find(|el| matches!(el, RichTextElement::Link { .. }));
            assert!(link.is_some(), "Should have a Link element");

            if let Some(RichTextElement::Link { label, url: _ }) = link {
                let label_text = label.to_plain();
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
