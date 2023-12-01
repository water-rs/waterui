use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[repr(u8)]
pub enum Alignment {
    Default,
    Leading,
    Center,
    Trailing,
}

impl Default for Alignment {
    fn default() -> Self {
        Self::Default
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[repr(C)]
pub enum Size {
    Default,
    Px(usize),
    Percent(f64),
}

impl Size {
    pub fn to_px(self, parent: usize) -> Option<usize> {
        match self {
            Size::Default => None,
            Size::Px(px) => Some(px),
            Size::Percent(percent) => Some((parent as f64 * percent) as usize),
        }
    }
}

impl Default for Size {
    fn default() -> Self {
        Self::Default
    }
}

impl_from!(Size, usize, Px);

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[repr(C)]

pub struct Frame {
    pub width: Size,
    pub min_width: Size,
    pub max_width: Size,
    pub height: Size,
    pub min_height: Size,
    pub max_height: Size,
    pub margin: Edge,
    pub alignment: Alignment,
}

impl Frame {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
#[repr(C)]

pub struct Edge {
    pub top: Size,
    pub right: Size,
    pub bottom: Size,
    pub left: Size,
}

impl Edge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn vertical(size: impl Into<Size>) -> Self {
        let size = size.into();
        Self::new().left(size.clone()).right(size)
    }

    pub fn horizontal(size: impl Into<Size>) -> Self {
        let size = size.into();

        Self::new().top(size.clone()).bottom(size)
    }

    pub fn round(size: impl Into<Size>) -> Self {
        let size = size.into();

        Self::new()
            .top(size.clone())
            .left(size.clone())
            .right(size.clone())
            .bottom(size)
    }

    pub fn top(mut self, size: impl Into<Size>) -> Self {
        self.top = size.into();
        self
    }

    pub fn left(mut self, size: impl Into<Size>) -> Self {
        self.left = size.into();
        self
    }
    pub fn right(mut self, size: impl Into<Size>) -> Self {
        self.right = size.into();
        self
    }
    pub fn bottom(mut self, size: impl Into<Size>) -> Self {
        self.bottom = size.into();
        self
    }
}
