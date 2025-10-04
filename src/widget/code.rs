use core::error::Error;

use waterui_core::View;
use waterui_layout::{scroll, spacer, stack::{hstack, vstack}};
use waterui_str::Str;
use waterui_text::{highlight::{highlight_text, DefaultHighlighter, Language}, text};

use crate::widget::suspense::suspense;


#[derive(Debug, Clone,PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code{
    language:Language,
    content:Str
}

impl Code{
    pub fn new(language:impl TryInto<Language, Error:Error>,content:impl Into<Str>) -> Self{
        Self{
            language:language.try_into().expect("Invalid language"),
            content:content.into()
        }
    }
}


impl View for Code{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        scroll(vstack((
            hstack((
                text!("{}",self.language).bold(),
                spacer(),
                text("Copy"),
            )),
            suspense(highlight_text(self.language,self.content,DefaultHighlighter::new()))
        )))
    }
}

pub fn code(language:impl TryInto<Language, Error:Error>,content:impl Into<Str>) -> Code{
    Code::new(language,content)
}