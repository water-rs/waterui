//! Conditional view rendering components for reactive UI programming.
//!
//! This module provides the `When` component that enables conditional rendering
//! of views based on reactive boolean conditions.
//!
//! # Basic Usage
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::widget::condition::when;
//!
//! let is_visible = Binding::bool(true);
//!
//! // Simple if-else
//! when(is_visible.clone(), || "Visible")
//!     .otherwise(|| "Hidden");
//!
//! // Multiple conditions (if-elif-else)
//! let state = Binding::i32(0);
//! when(state.equal_to(0), || "Loading...")
//!     .or(state.equal_to(1), || "Ready")
//!     .or(state.equal_to(2), || "Error")
//!     .otherwise(|| "Unknown");
//! ```

use core::any::Any;
use std::rc::Rc;

use crate::ViewExt;
use nami::{Computed, SignalExt, signal::IntoComputed};
use waterui_core::{AnyView, Dynamic, Environment, View, handler::ViewBuilder};

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
/// use waterui::widget::condition::when;
/// use waterui_text::text;
/// use nami::binding;
///
/// let show_message = binding(true);
///
/// // Simple conditional rendering
/// when(show_message.clone(), || "Hello, World!");
///
/// // Using negation (Binding implements Not)
/// when(!show_message.clone(), || "Message is hidden");
///
/// // With an alternative view
/// when(show_message, || "Logged in")
///     .otherwise(|| "Please log in");
/// ```
#[derive(Debug)]
pub struct When<Condition, Then> {
    condition: Condition,
    then: Then,
}

impl<Condition, Then> When<Condition, Then>
where
    Condition: IntoComputed<bool>,
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
    /// use waterui::widget::condition::When;
    /// use waterui_text::text;
    /// use nami::binding;
    ///
    /// let condition = binding(true);
    /// let when_component = When::new(condition.clone(), || text("Visible"));
    ///
    /// // Using negation
    /// let when_not = When::new(!condition, || "Hidden");
    /// ```
    pub const fn new(condition: Condition, then: Then) -> Self
    where
        Then: ViewBuilder,
    {
        Self { condition, then }
    }
}

/// Creates a new `When` component for conditional view rendering.
///
/// This is the primary function for creating conditional views in `WaterUI`. It accepts
/// a reactive boolean condition and a closure that returns a view to render when
/// the condition is `true`.
///
/// The condition is reactive, meaning the UI will automatically update when the
/// condition changes. This is achieved through `WaterUI`'s integration with the
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
/// use waterui::widget::condition::when;
/// use waterui_text::text;
/// use waterui_layout::stack::vstack;
/// use waterui::component::button;
/// use nami::binding;
///
/// let is_logged_in = binding(false);
///
/// // Basic conditional rendering
/// when(is_logged_in.clone(), || {
///     vstack((
///         "Welcome back!",
///         button("Logout"),
///     ))
/// });
///
/// // Using negation directly (no s!() needed)
/// when(!is_logged_in.clone(), || "Please log in");
///
/// // With alternative view
/// when(is_logged_in, || "Dashboard")
///     .otherwise(|| "Please log in");
/// ```
pub const fn when<Condition, Then>(condition: Condition, then: Then) -> When<Condition, Then>
where
    Condition: IntoComputed<bool>,
    Then: ViewBuilder,
{
    When::new(condition, then)
}

impl<Condition, Then> View for When<Condition, Then>
where
    Condition: IntoComputed<bool> + Clone,
    Then: ViewBuilder,
{
    fn body(self, _env: &Environment) -> impl View {
        self.otherwise(|| {})
    }
}

impl<Condition, Then> When<Condition, Then>
where
    Condition: IntoComputed<bool>,
    Then: ViewBuilder,
{
    /// Adds another conditional branch.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::widget::condition::when;
    /// use nami::binding;
    ///
    /// let state = binding(0);
    /// when(state.equal_to(0), || "Loading")
    ///     .or(state.equal_to(1), || "Ready")
    ///     .otherwise(|| "Error");
    /// ```
    pub const fn or<C, V>(self, condition: C, then: V) -> WhenChain<Self, C, V>
    where
        C: IntoComputed<bool>,
        V: ViewBuilder,
    {
        WhenChain {
            prev: self,
            condition,
            then,
        }
    }

    /// Adds a fallback view when the condition is `false`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::prelude::*;
    /// use waterui::widget::condition::when;
    ///
    /// let is_visible = Binding::bool(true);
    /// when(is_visible, || "Visible").otherwise(|| "Hidden");
    /// ```
    pub const fn otherwise<V>(self, otherwise: V) -> WhenComplete<Self, V>
    where
        V: ViewBuilder,
    {
        WhenComplete {
            chain: self,
            otherwise,
        }
    }
}

/// A chain of conditional branches that can be extended with `.or()`.
///
/// Created by calling [`When::or`]. Complete the chain with [`.otherwise()`](WhenChain::otherwise).
pub struct WhenChain<Prev, Condition, Then> {
    prev: Prev,
    condition: Condition,
    then: Then,
}

impl<Prev, Condition, Then> core::fmt::Debug for WhenChain<Prev, Condition, Then> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WhenChain").finish_non_exhaustive()
    }
}

impl<Prev, Condition, Then> WhenChain<Prev, Condition, Then>
where
    Condition: IntoComputed<bool>,
    Then: ViewBuilder,
{
    /// Adds another conditional branch.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use waterui::widget::condition::when;
    /// use nami::binding;
    ///
    /// let state = binding(0);
    /// when(state.equal_to(0), || "Loading")
    ///     .or(state.equal_to(1), || "Ready")
    ///     .or(state.equal_to(2), || "Warning")
    ///     .otherwise(|| "Error");
    /// ```
    pub const fn or<C, V>(self, condition: C, then: V) -> WhenChain<Self, C, V>
    where
        C: IntoComputed<bool>,
        V: ViewBuilder,
    {
        WhenChain {
            prev: self,
            condition,
            then,
        }
    }

    /// Completes the chain with a fallback view.
    ///
    /// This method must be called to finalize the conditional chain.
    pub const fn otherwise<V>(self, otherwise: V) -> WhenComplete<Self, V>
    where
        V: ViewBuilder,
    {
        WhenComplete {
            chain: self,
            otherwise,
        }
    }
}

/// A complete conditional chain with a fallback.
///
/// Created by calling [`.otherwise()`](WhenChain::otherwise) on a [`When`] or [`WhenChain`].
pub struct WhenComplete<Chain, Otherwise> {
    chain: Chain,
    otherwise: Otherwise,
}

impl<Chain, Otherwise> core::fmt::Debug for WhenComplete<Chain, Otherwise> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("WhenComplete").finish_non_exhaustive()
    }
}

/// Helper trait to evaluate a condition chain.
trait EvalChain: 'static {
    /// Check if all conditions in this chain are static bools.
    fn all_static(&self) -> bool;

    /// Evaluate the chain statically (when all conditions are static).
    /// Returns `Some(view)` if a condition matched, `None` otherwise.
    fn eval_static(&self) -> Option<AnyView>;

    /// Create a combined signal that returns which branch index matched (if any).
    fn make_combined(&self) -> Computed<Option<usize>>;

    /// Build the view for a given branch index.
    fn build_branch(&self, index: usize) -> AnyView;
}

impl<C, T> EvalChain for When<C, T>
where
    C: IntoComputed<bool> + Clone,
    T: ViewBuilder,
{
    fn all_static(&self) -> bool {
        let signal = self.condition.clone().into_signal();
        let any: &dyn Any = &signal;
        any.downcast_ref::<bool>().is_some()
    }

    fn eval_static(&self) -> Option<AnyView> {
        let signal = self.condition.clone().into_signal();
        let any: &dyn Any = &signal;
        if let Some(&cond) = any.downcast_ref::<bool>()
            && cond
        {
            return Some(self.then.build().anyview());
        }
        None
    }

    fn make_combined(&self) -> Computed<Option<usize>> {
        let cond = self.condition.clone().into_computed();
        cond.map(|v| if v { Some(0) } else { None }).computed()
    }

    fn build_branch(&self, index: usize) -> AnyView {
        debug_assert_eq!(index, 0);
        self.then.build().anyview()
    }
}

impl<Prev, C, T> EvalChain for WhenChain<Prev, C, T>
where
    Prev: EvalChain + BranchCount,
    C: IntoComputed<bool> + Clone,
    T: ViewBuilder,
{
    fn all_static(&self) -> bool {
        if !self.prev.all_static() {
            return false;
        }
        let signal = self.condition.clone().into_signal();
        let any: &dyn Any = &signal;
        any.downcast_ref::<bool>().is_some()
    }

    fn eval_static(&self) -> Option<AnyView> {
        // Try previous branches first
        if let Some(view) = self.prev.eval_static() {
            return Some(view);
        }
        // Check this branch
        let signal = self.condition.clone().into_signal();
        let any: &dyn Any = &signal;
        if let Some(&cond) = any.downcast_ref::<bool>()
            && cond
        {
            return Some(self.then.build().anyview());
        }
        None
    }

    fn make_combined(&self) -> Computed<Option<usize>> {
        let prev_combined = self.prev.make_combined();
        let this_cond = self.condition.clone().into_computed();
        // Get prev len at creation time so we know the index for this branch
        let this_index = self.prev.branch_count();

        prev_combined
            .zip(&this_cond)
            .map(move |(prev, cond): (Option<usize>, bool)| {
                // If any previous branch matched, use that
                if prev.is_some() {
                    return prev;
                }
                // Check this branch
                if cond {
                    return Some(this_index);
                }
                None
            })
            .computed()
    }

    fn build_branch(&self, index: usize) -> AnyView {
        let prev_len = self.prev.branch_count();
        if index < prev_len {
            self.prev.build_branch(index)
        } else {
            debug_assert_eq!(index, prev_len);
            self.then.build().anyview()
        }
    }
}

/// Extension trait for getting branch count.
trait BranchCount {
    fn branch_count(&self) -> usize;
}

impl<C, T> BranchCount for When<C, T> {
    fn branch_count(&self) -> usize {
        1
    }
}

impl<Prev: BranchCount, C, T> BranchCount for WhenChain<Prev, C, T> {
    fn branch_count(&self) -> usize {
        self.prev.branch_count() + 1
    }
}

impl<Chain, Otherwise> View for WhenComplete<Chain, Otherwise>
where
    Chain: EvalChain,
    Otherwise: ViewBuilder,
{
    fn body(self, _env: &Environment) -> impl View {
        let Self { chain, otherwise } = self;

        // Check if all conditions are static bools for compile-time optimization
        if chain.all_static() {
            // Static optimization: evaluate at build time
            if let Some(view) = chain.eval_static() {
                return view;
            }
            return otherwise.build().anyview();
        }

        let chain = Rc::new(chain);
        let otherwise = Rc::new(otherwise);

        Dynamic::watch(chain.make_combined(), move |index| {
            index.map_or_else(|| otherwise.build().anyview(), |i| chain.build_branch(i))
        })
        .anyview()
    }
}
