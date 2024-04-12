use crate::component::AnyView;
use crate::{Compute, View, ViewExt};

use super::Text;
#[non_exhaustive]
pub struct Button {
    pub _label: AnyView,
    pub _action: Box<dyn Fn() + Send + Sync>,
}

impl Button {
    pub fn new(
        label: impl Compute<Output = String>,
        action: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        Self {
            _label: Text::new(label).anyview(),
            _action: Box::new(action),
        }
    }

    pub fn action(action: impl Fn() + Send + Sync + 'static) -> Self {
        Self::new("", action)
    }

    pub fn label(mut self, label: impl View + 'static) -> Self {
        self._label = label.anyview();
        self
    }
}

raw_view!(Button);
impl_debug!(Button);

pub fn button(
    label: impl Compute<Output = String>,
    action: impl Fn() + Send + Sync + 'static,
) -> Button {
    Button::new(label, action)
}
