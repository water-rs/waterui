use waterui_core::{AnyView, View};
use waterui_reactive::{Compute, Computed, compute::IntoComputed};

use waterui_core::Color;

#[derive(Debug)]
pub struct BadgeConfig {
    pub value: Computed<i32>,
    pub content: AnyView,
}

configurable!(Badge, BadgeConfig);

impl Badge {
    pub fn new(value: impl IntoComputed<i32>, content: impl View) -> Self {
        //Self { value: (), color: Color:: }
        todo!()
    }

    pub fn color(self, color: impl Compute<Output = Color>) -> Self {
        //Self { value: (), color: Color:: }
        todo!()
    }
}
