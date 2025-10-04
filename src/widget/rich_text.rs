use waterui_str::Str;
use waterui_text::styled::{Style, StyledStr};

pub struct RichText{

}

#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum RichTextElement{
    Text(StyledStr),
    Link{label: StyledStr, url: Str},
    Image{src: Str, alt: Str},
    Table{headers: Vec<RichTextElement>, rows: Vec<Vec<RichTextElement>>},
    List{items: Vec<RichTextElement>, ordered: bool},
    Code{code: Str, language: Option<Str>},
    Quote{content: Vec<RichTextElement>},
}