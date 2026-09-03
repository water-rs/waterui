use std::{mem, num::NonZeroUsize, str::FromStr};

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag};
use waterui_core::{AnyView, Environment, View};
#[cfg(feature = "snackbar")]
use waterui_core::{State, extract::Extractor as _};
use waterui_graphics::color::Blue;
use waterui_layout::{
    Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions,
    container::FixedContainer,
    stack::{HStack, HorizontalAlignment, VStack, hstack},
};
#[cfg(feature = "media")]
use waterui_media::{Url, photo::photo as media_photo};
use waterui_str::Str;
use waterui_text::{
    Text,
    highlight::Language,
    styled::{MarkdownInlineBuilder, Style, StyledStr, heading_style},
    text,
};

#[cfg(feature = "snackbar")]
use crate::snackbar::{Snackbar, SnackbarManager};
use crate::{ViewExt, widget::Divider};

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
    /// A mathematical formula, as LaTeX source.
    ///
    /// Only produced when the `markdown-math` feature is on, because that is
    /// what enables the parser extension that recognises `$…$`.
    #[cfg(feature = "markdown-math")]
    Math {
        /// The LaTeX between the delimiters.
        source: Str,
        /// `$$…$$` is set on its own line in display style; `$…$` is inline.
        block: bool,
    },
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
        /// The fence's info token as written, before it was resolved to a
        /// [`Language`]; what a realization dispatches on.
        ///
        /// `None` for an indented block and for a fence with an empty info
        /// string. A token no [`Language`] recognises — `mermaid`, say —
        /// survives here even though `language` resolved to
        /// [`Language::Plaintext`].
        info: Option<Str>,
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
            Self::Text(s) => AnyView::new(text(s)),
            Self::Link { label, url } => {
                AnyView::new(crate::component::link::link(text(label), url))
            }
            Self::Image { src, alt: _ } => AnyView::new(render_image(&src)),
            #[cfg(feature = "markdown-math")]
            Self::Math { source, block } => AnyView::new(render_math(source, block)),
            Self::Table {
                headers,
                rows,
                alignments,
            } => render_table(&headers, &rows, &alignments),
            Self::List {
                items,
                ordered,
                start,
            } => AnyView::new(render_list(items.as_slice(), ordered, start)),
            Self::Code {
                code,
                info,
                language,
            } => {
                let view = crate::widget::code(language, code);
                let view = match info {
                    Some(info) => view.info(info),
                    None => view,
                };
                // Copy feedback goes through the window's `SnackbarManager`, a
                // semantic object the runtime owns. `Code` cannot name it — it
                // lives in `waterui-text` — and this is the one place a fence
                // is rendered from Markdown, so the snackbar coupling is here.
                #[cfg(feature = "snackbar")]
                let view = view.on_copied(|env| {
                    let State(snackbar) = State::<SnackbarManager>::extract(env)
                        .expect("the window's environment carries its SnackbarManager");
                    snackbar.show(Snackbar::new("Copied to clipboard"));
                });
                AnyView::new(view)
            }
            Self::Quote { content } => AnyView::new(quote(content)),
            Self::Group { elements, inline } => {
                if inline {
                    // Inline content already contains explicit whitespace in the source text.
                    // Use zero stack spacing to avoid double-spacing between adjacent spans.
                    AnyView::new(elements.into_iter().collect::<HStack<_>>().spacing(0.0))
                } else {
                    AnyView::new(VStack::from_iter(elements))
                }
            }
            Self::Divider => AnyView::new(Divider),
        }
    }
}

/// Recognising `$…$` is only correct when something can typeset the result.
///
/// Left on in a build with no math renderer, every dollar sign in prose would
/// start a formula that nothing could draw — so the extension is enabled with
/// the renderer and not otherwise.
#[cfg(feature = "markdown-math")]
const MATH_OPTIONS: Options = Options::ENABLE_MATH;

/// The Markdown parser recognises no math without a renderer for it.
#[cfg(not(feature = "markdown-math"))]
const MATH_OPTIONS: Options = Options::empty();

/// Typesets a formula parsed out of Markdown.
///
/// `$$…$$` is set in display style, which is what gives a summation its
/// full-height limits and a fraction its wider spacing; `$…$` is set inline so
/// it sits at the size of the surrounding prose.
#[cfg(feature = "markdown-math")]
fn render_math(source: Str, block: bool) -> AnyView {
    let formula = waterui_math::view::Math::new(source);
    AnyView::new(if block {
        formula.display()
    } else {
        formula.inline()
    })
}

fn render_image(src: &Str) -> AnyView {
    #[cfg(feature = "media")]
    {
        let url = Url::parse(src)
            .unwrap_or_else(|| panic!("RichText image source is not a valid URL: {src}"));
        AnyView::new(media_photo(url))
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

fn render_list(items: &[RichTextElement], ordered: bool, start: usize) -> impl View + use<> {
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
    headers: &[RichTextElement],
    rows: &[Vec<RichTextElement>],
    alignments: &[MarkdownTableAlignment],
) -> AnyView {
    let col_count = headers
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or_default());
    if col_count == 0 {
        return AnyView::new(());
    }

    let mut cells = Vec::with_capacity(col_count * (rows.len() + 1) + 1);
    cells.extend((0..col_count).map(|col_idx| {
        AnyView::new(
            headers
                .get(col_idx)
                .map_or_else(|| Text::from(""), element_to_text)
                .bold(),
        )
    }));
    cells.push(AnyView::new(Divider));

    for row in rows {
        cells.extend((0..col_count).map(|col_idx| {
            AnyView::new(
                row.get(col_idx)
                    .map_or_else(|| Text::from(""), element_to_text),
            )
        }));
    }

    let resolved_alignments = (0..col_count)
        .map(|col_idx| {
            alignments
                .get(col_idx)
                .copied()
                .unwrap_or(MarkdownTableAlignment::None)
        })
        .collect();
    let layout = MarkdownTableLayout::new(
        NonZeroUsize::new(col_count).expect("Markdown table must have at least one column"),
        resolved_alignments,
    );
    AnyView::new(FixedContainer::new(layout, cells))
}

const MARKDOWN_TABLE_COLUMN_SPACING: f32 = 12.0;
const MARKDOWN_TABLE_ROW_SPACING: f32 = 6.0;

#[derive(Debug)]
struct MarkdownTableLayout {
    columns: NonZeroUsize,
    alignments: Vec<MarkdownTableAlignment>,
}

struct MarkdownTableMeasurement {
    column_widths: Vec<f32>,
    cell_dimensions: Vec<ViewDimensions>,
    row_heights: Vec<f32>,
    separator_height: f32,
    size: Size,
}

impl MarkdownTableLayout {
    fn new(columns: NonZeroUsize, alignments: Vec<MarkdownTableAlignment>) -> Self {
        assert_eq!(
            alignments.len(),
            columns.get(),
            "Markdown table must provide one alignment for every column"
        );
        Self {
            columns,
            alignments,
        }
    }

    const fn separator_index(&self) -> usize {
        self.columns.get()
    }

    fn validate_children(&self, children: &[&dyn SubView]) {
        let columns = self.columns.get();
        assert!(
            children.len() > columns,
            "Markdown table layout requires a header row and separator"
        );
        assert_eq!(
            (children.len() - 1) % columns,
            0,
            "Markdown table layout requires complete rows"
        );
    }

    const fn child_index(&self, cell_index: usize) -> usize {
        if cell_index < self.separator_index() {
            cell_index
        } else {
            cell_index + 1
        }
    }

    fn measure(
        &self,
        proposed_width: Option<f32>,
        children: &[&dyn SubView],
    ) -> MarkdownTableMeasurement {
        self.validate_children(children);

        let columns = self.columns.get();
        let cell_count = children.len() - 1;
        let row_count = cell_count / columns;
        let mut column_widths = vec![0.0_f32; columns];

        for cell_index in 0..cell_count {
            let dimensions =
                children[self.child_index(cell_index)].measure(ProposalSize::UNSPECIFIED);
            let width = dimensions.size.width;
            assert!(
                width.is_finite(),
                "Markdown table cells must have finite intrinsic widths"
            );
            let column = cell_index % columns;
            column_widths[column] = column_widths[column].max(width.max(0.0));
        }

        let column_spacing = repeated_spacing(MARKDOWN_TABLE_COLUMN_SPACING, columns - 1);
        let intrinsic_content_width = column_widths.iter().sum::<f32>();
        let intrinsic_width = intrinsic_content_width + column_spacing;
        let width = proposed_width
            .filter(|width| width.is_finite())
            .map_or(intrinsic_width, |width| width.max(0.0));
        let available_content_width = (width - column_spacing).max(0.0);

        if intrinsic_content_width > available_content_width && intrinsic_content_width > 0.0 {
            let scale = available_content_width / intrinsic_content_width;
            for column_width in &mut column_widths {
                *column_width *= scale;
            }
        }

        let mut cell_dimensions = Vec::with_capacity(cell_count);
        let mut row_heights = vec![0.0_f32; row_count];
        for cell_index in 0..cell_count {
            let column = cell_index % columns;
            let dimensions = children[self.child_index(cell_index)]
                .measure(ProposalSize::new(Some(column_widths[column]), None));
            assert!(
                dimensions.size.width.is_finite() && dimensions.size.height.is_finite(),
                "Markdown table cells must have finite measured sizes"
            );
            let row = cell_index / columns;
            row_heights[row] = row_heights[row].max(dimensions.size.height.max(0.0));
            cell_dimensions.push(dimensions);
        }

        let separator_dimensions =
            children[self.separator_index()].measure(ProposalSize::new(Some(width), None));
        assert!(
            separator_dimensions.size.height.is_finite(),
            "Markdown table separator must have a finite height"
        );
        let separator_height = separator_dimensions.size.height.max(0.0);
        let height = row_heights.iter().sum::<f32>()
            + separator_height
            + repeated_spacing(MARKDOWN_TABLE_ROW_SPACING, row_count);

        MarkdownTableMeasurement {
            column_widths,
            cell_dimensions,
            row_heights,
            separator_height,
            size: Size::new(width, height),
        }
    }
}

impl Layout for MarkdownTableLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        self.measure(proposal.width, children).size
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        let measurement = self.measure(Some(bounds.width()), children);
        let columns = self.columns.get();
        let row_count = measurement.row_heights.len();
        let mut placements = vec![Rect::from_size(Size::zero()); children.len()];
        let mut row_y = bounds.y();

        for row in 0..row_count {
            let mut column_x = bounds.x();
            for column in 0..columns {
                let cell_index = row * columns + column;
                let dimensions = &measurement.cell_dimensions[cell_index];
                let column_width = measurement.column_widths[column];
                let child_width = dimensions.size.width.clamp(0.0, column_width);
                let alignment = self.alignments[column];
                let alignment_offset = match alignment {
                    MarkdownTableAlignment::None | MarkdownTableAlignment::Left => 0.0,
                    MarkdownTableAlignment::Center => (column_width - child_width) * 0.5,
                    MarkdownTableAlignment::Right => column_width - child_width,
                };
                placements[self.child_index(cell_index)] = Rect::new(
                    Point::new(column_x + alignment_offset, row_y),
                    Size::new(child_width, dimensions.size.height.max(0.0)),
                );
                column_x += column_width + MARKDOWN_TABLE_COLUMN_SPACING;
            }

            row_y += measurement.row_heights[row];
            if row == 0 {
                row_y += MARKDOWN_TABLE_ROW_SPACING;
                placements[self.separator_index()] = Rect::new(
                    Point::new(bounds.x(), row_y),
                    Size::new(bounds.width(), measurement.separator_height),
                );
                row_y += measurement.separator_height;
            }
            if row + 1 < row_count {
                row_y += MARKDOWN_TABLE_ROW_SPACING;
            }
        }

        placements
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

fn repeated_spacing(spacing: f32, count: usize) -> f32 {
    (0..count).map(|_| spacing).sum()
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
        | Options::ENABLE_TASKLISTS
        | MATH_OPTIONS;
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
                        info: info_from_kind(&kind),
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
                    if let Some(Container::CodeBlock {
                        info,
                        language,
                        code,
                    }) = stack.pop()
                    {
                        push_to_parent(
                            &mut stack,
                            RichTextElement::Code {
                                info,
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
            // `$…$` and `$$…$$`. These only arrive when the parser is built
            // with `ENABLE_MATH`, which happens only under this feature, so
            // without it a dollar sign stays ordinary text.
            #[cfg(feature = "markdown-math")]
            Event::InlineMath(source) => {
                push_inline_element(
                    &mut stack,
                    RichTextElement::Math {
                        source: Str::from(source.as_ref().to_string()),
                        block: false,
                    },
                );
            }
            #[cfg(feature = "markdown-math")]
            Event::DisplayMath(source) => {
                push_inline_element(
                    &mut stack,
                    RichTextElement::Math {
                        source: Str::from(source.as_ref().to_string()),
                        block: true,
                    },
                );
            }
            // Without the feature the parser is built without `ENABLE_MATH`,
            // so these cannot be produced. Saying so is better than a silent
            // arm that would quietly render a formula as its own source if the
            // options above ever changed.
            #[cfg(not(feature = "markdown-math"))]
            Event::InlineMath(_) | Event::DisplayMath(_) => {
                unreachable!(
                    "pulldown-cmark emitted a math event, but the parser is built without \
                     ENABLE_MATH; enable the `markdown-math` feature to render formulas"
                )
            }
            Event::Html(text) | Event::FootnoteReference(text) | Event::InlineHtml(text) => {
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

/// The fence's info token as the author wrote it.
///
/// `language_from_kind` throws this away whenever no [`Language`] answers to
/// it, which is exactly the case a realization needs to see: a ` ```mermaid `
/// fence and an untagged one both resolve to [`Language::Plaintext`] and are
/// told apart only by this token.
fn info_from_kind(kind: &CodeBlockKind) -> Option<Str> {
    match kind {
        CodeBlockKind::Fenced(info) => info
            .split_whitespace()
            .next()
            .map(|token| Str::from(token.to_owned())),
        CodeBlockKind::Indented => None,
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
        info: Option<Str>,
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

    /// Collects a document's elements into a shape that is easy to assert on.
    fn plain_text_of(elements: &[RichTextElement]) -> String {
        elements.iter().map(element_to_plain_text).collect()
    }

    /// With the renderer present, `$…$` becomes a formula rather than prose.
    #[cfg(feature = "markdown-math")]
    #[test]
    fn inline_and_display_math_become_math_elements() {
        let elements = parse_markdown("before $e^{i\\pi}+1=0$ after\n\n$$\\frac{a}{b}$$");

        let mut inline = 0;
        let mut block = 0;
        fn count(elements: &[RichTextElement], inline: &mut usize, block: &mut usize) {
            for element in elements {
                match element {
                    RichTextElement::Math { block: true, .. } => *block += 1,
                    RichTextElement::Math { block: false, .. } => *inline += 1,
                    RichTextElement::Group { elements, .. }
                    | RichTextElement::Quote { content: elements } => {
                        count(elements, inline, block);
                    }
                    _ => {}
                }
            }
        }
        count(&elements, &mut inline, &mut block);

        assert_eq!(inline, 1, "expected one inline formula in {elements:?}");
        assert_eq!(block, 1, "expected one display formula in {elements:?}");
    }

    /// The formula's LaTeX must not also appear as prose. Rendering the source
    /// alongside, or instead of, the formula is the defect this replaces.
    #[cfg(feature = "markdown-math")]
    #[test]
    fn math_source_does_not_leak_into_the_text() {
        let elements = parse_markdown("value $x^2$ end");
        let prose = plain_text_of(&elements);
        assert!(
            !prose.contains("x^2"),
            "the LaTeX source must not be rendered as text, got {prose:?}"
        );
    }

    /// Without the renderer the parser has no math extension, so a dollar sign
    /// is an ordinary character and nothing panics on prose that contains one.
    #[cfg(not(feature = "markdown-math"))]
    #[test]
    fn a_dollar_sign_is_ordinary_text_without_the_math_feature() {
        let elements = parse_markdown("it costs $5 and $10, or $x$ if you prefer");
        let prose = plain_text_of(&elements);
        assert!(
            prose.contains("$5") && prose.contains("$10"),
            "prices must survive as written, got {prose:?}"
        );
    }

    struct MockTableCell {
        size: Size,
    }

    impl SubView for MockTableCell {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            let width = proposal
                .width
                .map_or(self.size.width, |width| self.size.width.min(width));
            ViewDimensions::new(Size::new(width, self.size.height))
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "Expected {expected}, got {actual}"
        );
    }

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

    /// The fence's info token, not the [`Language`] it resolved to.
    ///
    /// `mermaid` is the case that matters: no [`Language`] answers to it, so
    /// the resolved language is [`Language::Plaintext`] — exactly what an
    /// untagged fence resolves to. Only the preserved token tells the two
    /// apart, and a realization has nothing else to dispatch on.
    fn only_code_block(markdown: &str) -> (Option<Str>, Language) {
        RichText::from_markdown(markdown)
            .elements()
            .iter()
            .find_map(|el| match el {
                RichTextElement::Code { info, language, .. } => {
                    Some((info.clone(), language.clone()))
                }
                _ => None,
            })
            .expect("expected a code block")
    }

    #[test]
    fn an_unrecognised_info_token_survives_as_written() {
        let (info, language) = only_code_block("```mermaid\nflowchart TD\n  A --> B\n```\n");
        assert_eq!(info.as_deref(), Some("mermaid"));
        assert_eq!(language, Language::Plaintext);
    }

    #[test]
    fn a_recognised_info_token_is_kept_alongside_its_language() {
        let (info, language) = only_code_block("```rust\nfn main() {}\n```\n");
        assert_eq!(info.as_deref(), Some("rust"));
        assert_eq!(language, Language::Rust);
    }

    #[test]
    fn an_indented_block_has_no_info_token() {
        let (info, language) = only_code_block("    fn main() {}\n");
        assert_eq!(info, None);
        assert_eq!(language, Language::Plaintext);
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
    fn parses_markdown_table_column_alignments() {
        let rich = RichText::from_markdown(
            "| Left | Center | Right |\n| :--- | :----: | ----: |\n| A | B | C |\n",
        );
        let alignments = rich
            .elements()
            .iter()
            .find_map(|element| match element {
                RichTextElement::Table { alignments, .. } => Some(alignments),
                _ => None,
            })
            .expect("Expected a parsed table");

        assert_eq!(
            alignments,
            &[
                MarkdownTableAlignment::Left,
                MarkdownTableAlignment::Center,
                MarkdownTableAlignment::Right,
            ]
        );
    }

    #[test]
    fn markdown_table_layout_shares_column_origins_across_rows() {
        let layout = MarkdownTableLayout::new(
            NonZeroUsize::new(3).unwrap(),
            vec![
                MarkdownTableAlignment::None,
                MarkdownTableAlignment::None,
                MarkdownTableAlignment::None,
            ],
        );
        let cells = [
            MockTableCell {
                size: Size::new(80.0, 20.0),
            },
            MockTableCell {
                size: Size::new(60.0, 20.0),
            },
            MockTableCell {
                size: Size::new(40.0, 20.0),
            },
            MockTableCell {
                size: Size::new(0.0, 1.0),
            },
            MockTableCell {
                size: Size::new(30.0, 20.0),
            },
            MockTableCell {
                size: Size::new(20.0, 20.0),
            },
            MockTableCell {
                size: Size::new(10.0, 20.0),
            },
        ];
        let children = cells
            .iter()
            .map(|cell| cell as &dyn SubView)
            .collect::<Vec<_>>();

        let size = layout.size_that_fits(ProposalSize::new(Some(300.0), None), children.as_slice());
        let placements = layout.place(Rect::from_size(size), children.as_slice());

        assert_eq!(size, Size::new(300.0, 53.0));
        assert_close(placements[0].x(), placements[4].x());
        assert_close(placements[1].x(), placements[5].x());
        assert_close(placements[2].x(), placements[6].x());
        assert_close(placements[3].width(), 300.0);
    }

    #[test]
    fn markdown_table_layout_honors_column_alignments() {
        let layout = MarkdownTableLayout::new(
            NonZeroUsize::new(3).unwrap(),
            vec![
                MarkdownTableAlignment::Left,
                MarkdownTableAlignment::Center,
                MarkdownTableAlignment::Right,
            ],
        );
        let cells = [
            MockTableCell {
                size: Size::new(80.0, 20.0),
            },
            MockTableCell {
                size: Size::new(80.0, 20.0),
            },
            MockTableCell {
                size: Size::new(80.0, 20.0),
            },
            MockTableCell {
                size: Size::new(0.0, 1.0),
            },
            MockTableCell {
                size: Size::new(20.0, 20.0),
            },
            MockTableCell {
                size: Size::new(20.0, 20.0),
            },
            MockTableCell {
                size: Size::new(20.0, 20.0),
            },
        ];
        let children = cells
            .iter()
            .map(|cell| cell as &dyn SubView)
            .collect::<Vec<_>>();
        let size = layout.size_that_fits(ProposalSize::new(Some(300.0), None), children.as_slice());
        let placements = layout.place(Rect::from_size(size), children.as_slice());

        assert_close(placements[4].x(), 0.0);
        assert_close(placements[5].x(), 122.0);
        assert_close(placements[6].x(), 244.0);
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
    #[allow(
        clippy::items_after_statements,
        reason = "test-local helper fn defined next to its single use"
    )]
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
