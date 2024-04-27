use alloc::boxed::Box;
use waterui_reactive::compute::IntoComputed;
use waterui_reactive::CowStr;

use crate::AnyView;
use crate::{Environment, View, ViewExt};

use super::Text;

pub struct Button<Label> {
    label: Label,
    action: Box<dyn Fn(&Environment)>,
}

pub trait Action {
    fn call_action(&self, env: &Environment);
}

impl<F: Fn(&Environment)> Action for F {
    fn call_action(&self, env: &Environment) {
        (self)(env)
    }
}

#[non_exhaustive]
pub struct RawButton {
    pub _label: AnyView,
    pub _action: Box<dyn Fn(&Environment)>,
}

impl<Label: View + 'static> Button<Label> {
    pub fn label(label: Label) -> Self {
        Self {
            label,
            action: Box::new(|_| {}),
        }
    }

    pub fn action(self, action: impl Fn() + 'static) -> Self {
        self.action_env(move |_| action())
    }

    pub fn action_env(mut self, action: impl Fn(&Environment) + 'static) -> Self {
        self.action = Box::new(move |env| action(env));
        self
    }

    #[cfg(feature = "async")]
    pub fn action_async<Fut>(self, action: impl 'static + Fn() -> Fut) -> Self
    where
        Fut: core::future::Future<Output = ()> + 'static,
    {
        self.action_env(move |env| env.task(action()).detach())
    }
}

impl Button<Text> {
    pub fn new(label: impl IntoComputed<CowStr>) -> Self {
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

pub fn button(label: impl IntoComputed<CowStr>) -> Button<Text> {
    Button::new(label)
}
