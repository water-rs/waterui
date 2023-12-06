use waterui_core::view::BoxView;

use crate::utils::Background;

pub struct BackgroundModifier {
    pub(crate) background: Background,
    pub(crate) content: BoxView,
}

native_implement!(BackgroundModifier);

impl BackgroundModifier {
    pub fn new(content: BoxView, background: Background) -> Self {
        Self {
            background,
            content,
        }
    }
}
