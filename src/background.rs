use waterui_core::{Color, Computed};
use waterui_reactive::compute::IntoComputed;

#[derive(Debug)]
pub enum Background {
    Color(Computed<Color>),
}

impl Background {
    pub fn color(color: impl IntoComputed<Color>) -> Self {
        Self::Color(color.into_computed())
    }
}

#[derive(Debug)]
pub struct ForegroundColor {
    pub color: Computed<Color>,
}

impl ForegroundColor {
    pub fn new(color: impl IntoComputed<Color>) -> Self {
        Self {
            color: color.into_computed(),
        }
    }
}
