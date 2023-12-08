use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[repr(C)]
pub enum Size {
    Default,
    Size(f64),
}

impl Default for Size {
    fn default() -> Self {
        Self::Default
    }
}

impl From<f64> for Size {
    fn from(value: f64) -> Self {
        Self::Size(value)
    }
}

impl From<u64> for Size {
    fn from(value: u64) -> Self {
        (value as f64).into()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[repr(C)]
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
        Self::new().left(size).right(size)
    }

    pub fn horizontal(size: impl Into<Size>) -> Self {
        let size = size.into();

        Self::new().top(size).bottom(size)
    }

    pub fn round(size: impl Into<Size>) -> Self {
        let size = size.into();

        Self::new().top(size).left(size).right(size).bottom(size)
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
