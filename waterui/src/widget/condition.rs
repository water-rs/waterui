use crate::{component::Dynamic, view::ViewBuilder, ViewExt};
use waterui_core::{
    handler::{HandlerFn, IntoHandler},
    Environment, View,
};
use waterui_reactive::compute::IntoComputed;
#[derive(Debug)]
pub struct When<Condition, Then> {
    condition: Condition,
    then: Then,
}

impl<Condition, Then> When<Condition, Then>
where
    Condition: IntoComputed<bool>,
    Then: ViewBuilder,
{
    pub fn new(condition: Condition, then: Then) -> Self {
        Self { condition, then }
    }
}

pub fn when<Condition, P, Then, V>(
    condition: Condition,
    then: Then,
) -> When<Condition, IntoHandler<Then, P, V>>
where
    Condition: IntoComputed<bool>,
    Then: HandlerFn<P, V>,
    V: View,
    P: 'static,
{
    When::new(condition, IntoHandler::new(then))
}

impl<Condition, Then> View for When<Condition, Then>
where
    Condition: IntoComputed<bool>,
    Then: ViewBuilder,
{
    fn body(self, _env: &Environment) -> impl View {
        self.or(|| {})
    }
}

impl<Condition, Then> When<Condition, Then> {
    pub fn or<P, Or, V>(self, or: Or) -> WhenOr<Condition, Then, IntoHandler<Or, P, V>>
    where
        Condition: IntoComputed<bool>,
        Or: HandlerFn<P, V>,
        V: View,
    {
        WhenOr {
            condition: self.condition,
            then: self.then,
            or: IntoHandler::new(or),
        }
    }
}

#[derive(Debug)]
pub struct WhenOr<Condition, Then, Or> {
    condition: Condition,
    then: Then,
    or: Or,
}

impl<Condition, Then, Or> View for WhenOr<Condition, Then, Or>
where
    Condition: IntoComputed<bool>,
    Then: ViewBuilder,
    Or: ViewBuilder,
{
    fn body(self, env: &Environment) -> impl View {
        let env = env.clone();
        Dynamic::watch(self.condition.into_compute(), move |condition| {
            if condition {
                (self.then).view(&env).anyview()
            } else {
                (self.or).view(&env).anyview()
            }
        })
    }
}
