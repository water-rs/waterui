extern crate alloc;

pub mod stack;

pub mod grid;
pub use grid::row;
pub mod overlay;
pub use overlay::overlay;
pub mod scroll;
pub use scroll::scroll;
pub mod spacer;
pub use spacer::spacer;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
pub enum Alignment {
    #[default]
    Default,
    Leading,
    Center,
    Trailing,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Deserialize, serde::Serialize))]
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

impl Default for Frame {
    fn default() -> Self {
        Self {
            width: f64::NAN,
            min_width: f64::NAN,
            max_width: f64::NAN,
            height: f64::NAN,
            min_height: f64::NAN,
            max_height: f64::NAN,
            margin: Edge::default(),
            alignment: Alignment::default(),
        }
    }
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

        Self::vertical(size).left(size).right(size)
    }
}

macro_rules! modify_field {
    ($ident:ident,$ty:ty) => {
        pub fn $ident(mut self, size: impl Into<$ty>) -> Self {
            self.$ident = size.into();
            self
        }
    };
}

impl Edge {
    modify_field!(top, f64);
    modify_field!(left, f64);
    modify_field!(right, f64);
    modify_field!(bottom, f64);
}

impl Frame {
    modify_field!(width, f64);
    modify_field!(min_width, f64);
    modify_field!(max_width, f64);
    modify_field!(height, f64);
    modify_field!(min_height, f64);
    modify_field!(max_height, f64);
    modify_field!(margin, Edge);
    modify_field!(alignment, Alignment);
}
