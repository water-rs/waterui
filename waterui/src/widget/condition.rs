use crate::{component::Dynamic, view::ViewBuilder, ViewExt};
use waterui_core::{Environment, View};
use waterui_reactive::compute::ToComputed;
pub struct When<Condition, Then> {
    condition: Condition,
    then: Then,
}

impl<Condition, Then> View for When<Condition, Then>
where
    Condition: ToComputed<bool>,
    Then: ViewBuilder,
{
    fn body(self, _env: Environment) -> impl View {
        self.or(|| {})
    }
}

impl<Condition, Then> When<Condition, Then> {
    pub fn or<Or>(self, or: Or) -> WhenOr<Condition, Then, Or> {
        WhenOr {
            condition: self.condition,
            then: self.then,
            or,
        }
    }
}

pub struct WhenOr<Condition, Then, Or> {
    condition: Condition,
    then: Then,
    or: Or,
}

impl<Condition, Then, Or> View for WhenOr<Condition, Then, Or>
where
    Condition: ToComputed<bool>,
    Then: ViewBuilder,
    Or: ViewBuilder,
{
    fn body(self, env: Environment) -> impl View {
        Dynamic::watch(self.condition.to_compute(), move |condition| {
            if condition {
                (self.then).view(env.clone()).anyview()
            } else {
                (self.or).view(env.clone()).anyview()
            }
        })
    }
}

pub fn when<Condition, Then>(condition: Condition, then: Then) -> When<Condition, Then> {
    When { condition, then }
}
