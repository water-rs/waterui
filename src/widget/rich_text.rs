use waterui_color::Blue;
use waterui_core::{Environment, View};
use waterui_layout::stack::{VStack, hstack};
use waterui_str::Str;
use waterui_text::{highlight::Language, link, styled::StyledStr, text};

use crate::{ViewExt, widget};

/// Rich text widget for displaying formatted content.
#[derive(Debug)]
pub struct RichText {
    elements: Vec<RichTextElement>,
}

/// Represents different types of rich text elements.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RichTextElement {
    /// Plain text with styling.
    Text(StyledStr),
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
        headers: Vec<RichTextElement>,
        /// Table rows.
        rows: Vec<Vec<RichTextElement>>,
    },
    /// A list of items.
    List {
        /// List items.
        items: Vec<RichTextElement>,
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
        content: Vec<RichTextElement>,
    },
}

impl View for RichTextElement {
    fn body(self, _env: &Environment) -> impl View {
        match self {
            Self::Text(s) => text(s).anyview(),
            Self::Link { label, url } => link(label, url).anyview(),
            Self::Image { src: _, alt: _ } => todo!(),
            Self::Table {
                headers: _,
                rows: _,
            } => todo!(),
            Self::List { items, ordered } => render_list(items.as_slice(), ordered).anyview(),
            Self::Code { code, language } => widget::code(language, code).anyview(),
            Self::Quote { content } => quote(content).anyview(),
        }
    }
}

impl View for RichText {
    fn body(self, _env: &Environment) -> impl View {
        VStack::from_iter(self.elements)
    }
}

fn render_list(items: &[RichTextElement], ordered: bool) -> impl View {
    let rows = items.iter().enumerate().map(|(index, item)| {
        let marker_label: Str = if ordered {
            format!("{}.", index + 1).into()
        } else {
            "•".into()
        };
        let marker = text(marker_label);

        hstack((marker.padding(), item.clone()))
    });

    rows.collect::<VStack>()
}

fn quote(content: Vec<RichTextElement>) -> impl View {
    // Render as blockquote
    // blue marker
    let quote_marker = Blue.width(4.0).height(f32::INFINITY);
    hstack((quote_marker, VStack::from_iter(content)))
}
