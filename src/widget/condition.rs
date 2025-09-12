//! Conditional view rendering components for reactive UI programming.
//!
//! This module provides the `When` and `WhenOr` components that enable conditional rendering
//! of views based on reactive boolean conditions. These components are essential for building
//! dynamic user interfaces that respond to changing application state.
//!
//! # Basic Usage
//!
//! ```rust
//! use waterui::when;
//! use nami::binding;
//!
//! let is_visible = binding(true);
//!
//! when(is_visible, || text!("This text is visible"))
//!     .or(|| text!("This text is shown when hidden"));
//!
//! // Binding implements Not trait - no need to wrap with s!()
//! when(!is_visible, || text!("This text is hidden"));
//! ```

use crate::{ViewExt, component::Dynamic, view::ViewBuilder};
use nami::signal::IntoComputed;
use waterui_core::{
    Environment, View,
    handler::{HandlerFn, IntoHandler},
};

/// A component that conditionally renders a view based on a reactive boolean condition.
///
/// The `When` component enables conditional rendering by evaluating a boolean condition
/// and rendering the associated view only when the condition is `true`. When the condition
/// is `false`, nothing is rendered unless extended with an `or` clause.
///
/// This component is particularly useful for:
/// - Showing/hiding UI elements based on application state
/// - Implementing feature flags or user permissions
/// - Creating responsive layouts that adapt to different conditions
///
/// # Examples
///
/// ```rust
/// use waterui::{when, text};
/// use nami::binding;
///
/// let show_message = binding(true);
///
/// // Simple conditional rendering
/// when(show_message, || text!("Hello, World!"));
///
/// // Using negation (Binding implements Not)
/// when(!show_message, || text!("Message is hidden"));
///
/// // With an alternative view
/// when(show_message, || text!("Logged in"))
///     .or(|| text!("Please log in"));
/// ```
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
    /// Creates a new `When` component with the given condition and view builder.
    ///
    /// This constructor is typically not used directly. Instead, use the [`when`] function
    /// for a more ergonomic API that accepts handler functions.
    ///
    /// # Arguments
    /// * `condition` - A reactive value that can be computed into a boolean
    /// * `then` - The view builder to execute when the condition is `true`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::When;
    /// use nami::binding;
    ///
    /// let condition = binding(true);
    /// let when_component = When::new(condition, || text!("Visible"));
    ///
    /// // Using negation
    /// let when_not = When::new(!condition, || text!("Hidden"));
    /// ```
    pub const fn new(condition: Condition, then: Then) -> Self {
        Self { condition, then }
    }
}

/// Creates a new `When` component for conditional view rendering.
///
/// This is the primary function for creating conditional views in WaterUI. It accepts
/// a reactive boolean condition and a closure that returns a view to render when
/// the condition is `true`.
///
/// The condition is reactive, meaning the UI will automatically update when the
/// condition changes. This is achieved through WaterUI's integration with the
/// [`nami`] reactive system.
///
/// # Arguments
/// * `condition` - A reactive value that evaluates to a boolean (e.g., `Signal<bool>`)
/// * `then` - A closure that returns the view to render when the condition is `true`
///
/// # Returns
/// A `When` component that can be extended with `.or()` for alternative rendering
///
/// # Examples
///
/// ```rust
/// use waterui::{when, text, vstack, button};
/// use nami::binding;
///
/// let is_logged_in = binding(false);
///
/// // Basic conditional rendering
/// when(is_logged_in, || {
///     vstack((
///         text!("Welcome back!"),
///         button("Logout", || {}),
///     ))
/// });
///
/// // Using negation directly (no s!() needed)
/// when(!is_logged_in, || text!("Please log in"));
///
/// // With alternative view
/// when(is_logged_in, || text!("Dashboard"))
///     .or(|| text!("Please log in"));
/// ```
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
    /// Adds an alternative view to render when the condition is `false`.
    ///
    /// This method transforms a `When` component into a `WhenOr` component that
    /// provides complete conditional rendering with both true and false branches.
    ///
    /// # Arguments
    /// * `or` - A closure that returns the view to render when the condition is `false`
    ///
    /// # Returns
    /// A `WhenOr` component that renders one of two views based on the condition
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::{when, text};
    /// use nami::binding;
    ///
    /// let has_data = binding(false);
    ///
    /// when(has_data, || text!("Data loaded"))
    ///     .or(|| text!("Loading..."));
    ///
    /// // Equivalent using negation
    /// when(!has_data, || text!("Loading..."))
    ///     .or(|| text!("Data loaded"));
    /// ```
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

/// A component that conditionally renders one of two views based on a reactive boolean condition.
///
/// The `WhenOr` component is created by calling the [`or`](When::or) method on a [`When`] component.
/// It provides complete conditional rendering by rendering the "then" view when the condition is `true`,
/// and the "or" view when the condition is `false`.
///
/// This component ensures that exactly one of the two views is always rendered, making it
/// ideal for implementing UI states like loading/loaded, authenticated/unauthenticated,
/// or any other binary state presentation.
///
/// # Reactivity
///
/// The `WhenOr` component is fully reactive. When the condition changes, the UI will
/// automatically switch between the two views without manual intervention.
///
/// # Examples
///
/// ```rust
/// use waterui::{when, text, vstack};
/// use nami::binding;
///
/// let is_loading = binding(true);
///
/// when(!is_loading, || {
///     vstack((
///         text!("Welcome!"),
///         text!("Your data is ready."),
///     ))
/// }).or(|| {
///     vstack((
///         text!("Loading..."),
///         // Could include a spinner component here
///     ))
/// });
/// ```
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
