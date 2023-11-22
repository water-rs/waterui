use crate::view::{Alignment, BoxView, Frame};

pub struct Stack {
    frame: Frame,
    pub alignment: Alignment,
    pub mode: DisplayMode,
    pub content: Vec<BoxView>,
}

#[repr(C)]
#[derive(Debug, Clone)]
pub enum DisplayMode {
    Vertical,
    Horizontal,
}

impl Stack {
    pub fn new(content: Vec<BoxView>, mode: DisplayMode) -> Self {
        Self {
            content,
            alignment: Alignment::Default,
            mode,
            frame: Default::default(),
        }
    }

    pub fn vertical(mut self) -> Self {
        self.mode = DisplayMode::Vertical;
        self
    }

    pub fn horizontal(mut self) -> Self {
        self.mode = DisplayMode::Horizontal;
        self
    }

    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }
}

#[macro_export]
macro_rules! vstack {
    ($($view:expr),*) => {
        {
            #[allow(unused_mut)]
            let mut content=Vec::new();
            $(
                let view:Box<dyn crate::View>=Box::new($view);
                content.push(view);
            )*
            crate::component::Stack::new(content,crate::component::stack::DisplayMode::Vertical)
        }
    };
}

#[macro_export]
macro_rules! hstack {
    ($($view:expr),*) => {
        {
            #[allow(unused_mut)]
            let mut content=Vec::new();
            $(
                let view:Box<dyn crate::View>=Box::new($view);
                content.push(view);
            )*
            crate::component::Stack::new(content,crate::component::stack::DisplayMode::Horizontal)
        }
    };
}

native_implement_with_frame!(Stack);
