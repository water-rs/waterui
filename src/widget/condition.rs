//! Provides conditional view rendering functionality through the `When` and `WhenOr` components.

use crate::{ViewExt, component::Dynamic, view::ViewBuilder};
use nami::signal::IntoComputed;
use waterui_core::{
    Environment, View,
    handler::{HandlerFn, IntoHandler},
};

/// A component that conditionally renders a view based on a boolean condition.
///
/// The `When` component allows you to render a view only when the specified condition is true.
/// It can be extended with an `or` clause to provide an alternative view when the condition is false.
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
    /// Creates a new `When` component with the given condition and view.
    ///
    /// # Arguments
    /// * `condition` - A value that can be computed into a boolean
    /// * `then` - The view to render when the condition is true
    pub const fn new(condition: Condition, then: Then) -> Self {
        Self { condition, then }
    }
}

/// Creates a new `When` component that conditionally renders a view.
///
/// This function is a convenient way to create a `When` component with a handler function.
///
/// # Arguments
/// * `condition` - A value that can be computed into a boolean
/// * `then` - A handler function that returns the view to render when the condition is true
///
/// # Returns
/// A `When` component that will conditionally render the provided view
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
    /// Adds an alternative view to render when the condition is false.
    ///
    /// # Arguments
    /// * `or` - A handler function that returns the view to render when the condition is false
    ///
    /// # Returns
    /// A `WhenOr` component that will conditionally render one of two views
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

/// A component that conditionally renders one of two views based on a boolean condition.
///
/// The `WhenOr` component is created by calling the `or` method on a `When` component.
/// It renders the first view when the condition is true, and the second view when the condition is false.
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
        Dynamic::watch(self.condition.into_signal(), move |condition| {
            if condition {
                (self.then).view(&env).anyview()
            } else {
                (self.or).view(&env).anyview()
            }
        })
    }
}
