use alloc::boxed::Box;
use alloc::string::String;

use crate::component::AnyView;
use crate::{Compute, View, ViewExt};

use super::Text;

#[non_exhaustive]
pub struct Button<Label> {
    label: Label,
    action: Box<dyn Fn()>,
}

#[non_exhaustive]
pub struct RawButton {
    pub _label: AnyView,
    pub _action: Box<dyn Fn()>,
}

impl<Label: View + 'static> Button<Label> {
    pub fn label(label: Label) -> Self {
        Self {
            label,
            action: Box::new(|| {}),
        }
    }

    pub fn action(mut self, action: impl Fn() + 'static) -> Self {
        self.action = Box::new(action);
        self
    }
}

impl Button<Text> {
    pub fn new(label: impl Compute<Output = String>) -> Self {
        Self::label(Text::new(label))
    }
}

impl_label!(Button);

impl<Label: View + 'static> View for Button<Label> {
    fn body(self, _env: crate::Environment) -> impl View {
        RawButton {
            _label: self.label.anyview(),
            _action: self.action,
        }
    }
}

raw_view!(RawButton);
impl_debug!(RawButton);
