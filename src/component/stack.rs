use crate::{
    view::{Alignment, BoxView, Frame, ViewExt},
    View,
};

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

impl From<Vec<BoxView>> for Stack {
    fn from(value: Vec<BoxView>) -> Self {
        Self {
            content: value,
            alignment: Alignment::Default,
            mode: DisplayMode::Vertical,
            frame: Default::default(),
        }
    }
}

impl Stack {
    pub fn new<Iter>(content: Iter) -> Self
    where
        Iter: IntoIterator,
        Iter::Item: View,
    {
        let content: Vec<BoxView> = content.into_iter().map(|v| v.into_boxed()).collect();
        Self::from(content)
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

    pub fn mode(mut self, mode: DisplayMode) -> Self {
        self.mode = mode;
        self
    }
}

#[macro_export]
macro_rules! vstack {
    ($($view:expr),*) => {
        {
            #[allow(unused_mut)]
            let mut content:Vec<Box<dyn crate::View>>=Vec::new();
            $(
                let view:Box<dyn crate::View>=Box::new($view);
                content.push(view);
            )*
            crate::component::Stack::new(content).vertical()
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
