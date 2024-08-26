use crate::{component::Dynamic, view::ViewBuilder};
use waterui_core::{Environment, View};
use waterui_reactive::{compute::ToComputed, Compute, ComputeExt};
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
        let (handler, dynamic) = Dynamic::new();
        let condition = self.condition.to_compute();
        if condition.compute() {
            handler.set((self.then).view(env.clone()));
        } else {
            handler.set((self.or).view(env.clone()));
        }
        condition
            .watch(move |c| {
                if c {
                    handler.set((self.then).view(env.clone()));
                } else {
                    handler.set((self.or).view(env.clone()));
                }
            })
            .leak();

        dynamic
    }
}

pub fn when<Condition, Then>(condition: Condition, then: Then) -> When<Condition, Then> {
    When { condition, then }
}
