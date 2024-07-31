use core::fmt::Debug;
use core::future::Future;

use alloc::boxed::Box;
use waterui_reactive::compute::ToComputed;
use waterui_str::Str;

use super::Text;
use crate::AnyView;
use crate::{Environment, View};
use waterui_core::raw_view;

#[non_exhaustive]
#[derive(Debug)]
pub struct Button {
    pub _label: AnyView,
    pub _action: BoxAction,
}

pub type BoxAction = Box<dyn Action>;
pub trait Action: 'static {
    fn call_action(&self, _env: &Environment);
}

impl_debug!(dyn Action);

pub trait AsyncAction: 'static {
    fn call_action(&self, _env: &Environment) -> impl Future<Output = ()> + 'static;
}

struct AsyncActionWrapper<T>(T);

impl<T: AsyncAction> Action for AsyncActionWrapper<T> {
    fn call_action(&self, env: &Environment) {
        env.task(AsyncAction::call_action(&self.0, env));
    }
}

impl<F> Action for F
where
    F: Fn() + 'static,
{
    fn call_action(&self, _env: &Environment) {
        (self)()
    }
}

impl Button {
    pub fn new(label: impl ToComputed<Str>) -> Self {
        Self::label(Text::new(label))
    }

    pub fn label(label: impl View) -> Self {
        Self {
            _label: AnyView::new(label),
            _action: Box::new(|| {}),
        }
    }

    pub fn action(mut self, action: impl Action) -> Self {
        self._action = Box::new(action);
        self
    }

    pub fn action_async<Fut>(self, action: impl AsyncAction) -> Self {
        self.action(AsyncActionWrapper(action))
    }
}

raw_view!(Button);

pub fn button(label: impl ToComputed<Str>) -> Button {
    Button::new(label)
}
