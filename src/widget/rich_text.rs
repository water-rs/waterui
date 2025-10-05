use waterui_color::Blue;
use waterui_core::{Environment, View};
use waterui_layout::stack::{hstack, VStack};
use waterui_str::Str;
use waterui_text::{highlight::Language, link, styled::StyledStr, text};

use crate::{widget, ViewExt};

/// Rich text widget for displaying formatted content.
#[derive(Debug)]
pub struct RichText{
    elements: Vec<RichTextElement>
}

/// Represents different types of rich text elements.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RichTextElement{
    /// Plain text with styling.
    Text(StyledStr),
    /// A hyperlink.
    Link{
        /// The link label.
        label: StyledStr,
        /// The link URL.
        url: Str
    },
    /// An image.
    Image{
        /// Image source URL.
        src: Str,
        /// Alternative text.
        alt: Str
    },
    /// A table with headers and rows.
    Table{
        /// Table headers.
        headers: Vec<RichTextElement>,
        /// Table rows.
        rows: Vec<Vec<RichTextElement>>
    },
    /// A list of items.
    List{
        /// List items.
        items: Vec<RichTextElement>,
        /// Whether the list is ordered.
        ordered: bool
    },
    /// A code block.
    Code{
        /// The code content.
        code: Str,
        /// Optional language specification.
        language: Language
    },
    /// A quotation block.
    Quote{
        /// The quoted content.
        content: Vec<RichTextElement>
    },
}

impl View for RichTextElement{
    fn body(self,env:&Environment) -> impl View{
        match self{
            Self::Text(s) => text(s).anyview(),
            Self::Link { label, url } => link(label, url).anyview(),
            Self::Image { src, alt } => todo!(),
            Self::Table { headers, rows } => todo!(),
            Self::List { items, ordered } => render_list(items, ordered).anyview(),
            Self::Code { code, language } => widget::code(language,code).anyview(),
            Self::Quote { content } => quote(content).anyview(),
        }
    }
}

fn render_list(items:Vec<RichTextElement>,ordered:bool) -> impl View{
    if ordered{
        // Render as ordered list
        
    }else{
        // Render as unordered list
        
    }
}

fn quote(content:Vec<RichTextElement>) -> impl View{
    // Render as blockquote
    // blue marker
    let quote_marker = Blue.width(4.0).height(f32::INFINITY);
    hstack((
        quote_marker,
        VStack::from_iter(content)
    ))
}