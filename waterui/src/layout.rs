use waterui_reactive::impl_constant;

impl_constant!(Frame, Edge);

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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

#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[repr(C)]
pub struct Frame {
    pub width: f64,
    pub min_width: f64,
    pub max_width: f64,
    pub height: f64,
    pub min_height: f64,
    pub max_height: f64,
    pub margin: Edge,
    pub alignment: Alignment,
}

impl Frame {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
#[repr(C)]

pub struct Edge {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Default for Edge {
    fn default() -> Self {
        Self {
            top: f64::NAN,
            right: f64::NAN,
            bottom: f64::NAN,
            left: f64::NAN,
        }
    }
}

impl Edge {
    pub fn zero() -> Self {
        Self {
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            left: 0.0,
        }
    }

    pub fn vertical(size: impl Into<f64>) -> Self {
        let size = size.into();
        Self::zero().left(size).right(size)
    }

    pub fn horizontal(size: impl Into<f64>) -> Self {
        let size = size.into();

        Self::zero().top(size).bottom(size)
    }

    pub fn round(size: impl Into<f64>) -> Self {
        let size = size.into();

        Self::zero().top(size).left(size).right(size).bottom(size)
    }

    pub fn top(mut self, size: impl Into<f64>) -> Self {
        self.top = size.into();
        self
    }

    pub fn left(mut self, size: impl Into<f64>) -> Self {
        self.left = size.into();
        self
    }
    pub fn right(mut self, size: impl Into<f64>) -> Self {
        self.right = size.into();
        self
    }
    pub fn bottom(mut self, size: impl Into<f64>) -> Self {
        self.bottom = size.into();
        self
    }
}
