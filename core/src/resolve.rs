//! # The Resolve Pattern
//!
//! The resolve pattern is WaterUI's core abstraction for **dynamic, reactive configuration**.
//! Instead of hard-coding values, types implement [`Resolvable`] to look up their actual
//! values from an [`Environment`] at runtime, returning a **reactive signal** that
//! automatically updates when the environment changes.
//!
//! ## For Users
//!
//! ### What is Resolvable?
//!
//! When you use a `Color` or `Font` in WaterUI, you're not specifying a fixed value—you're
//! specifying something that will be **resolved** against the current environment. This
//! enables powerful features like theming and dark mode.
//!
//! ```ignore
//! // This color isn't "#FF0000" - it's "whatever the Accent color is in this environment"
//! use waterui::theme::color::Accent;
//! text("Hello").foreground(Accent)
//! ```
//!
//! ### Why Reactive?
//!
//! The key insight is that `resolve()` returns a [`Signal`](nami::Signal), not a plain value.
//! This means:
//!
//! 1. **Native backends can inject reactive signals** - The iOS/Android runtime can inject
//!    system colors that update when the user toggles dark mode.
//! 2. **Views automatically re-render** - When the signal's value changes, any view using
//!    that resolved value will update without manual intervention.
//! 3. **No rebuild required** - Theme changes propagate instantly through the entire UI.
//!
//! ### The Flow
//!
//! ```text
//! ┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
//! │  Native Backend │     │   Environment   │     │      View       │
//! │  (iOS/Android)  │     │                 │     │                 │
//! └────────┬────────┘     └────────┬────────┘     └────────┬────────┘
//!          │                       │                       │
//!          │ 1. Create reactive    │                       │
//!          │    signal (Computed)  │                       │
//!          │──────────────────────>│                       │
//!          │                       │                       │
//!          │ 2. Install into env   │                       │
//!          │    via Theme::install │                       │
//!          │──────────────────────>│                       │
//!          │                       │                       │
//!          │                       │ 3. View resolves      │
//!          │                       │    Accent.resolve(env)│
//!          │                       │<──────────────────────│
//!          │                       │                       │
//!          │                       │ 4. Returns Computed   │
//!          │                       │    (reactive signal)  │
//!          │                       │──────────────────────>│
//!          │                       │                       │
//!          │ 5. User toggles       │                       │
//!          │    dark mode          │                       │
//!          │──────────────────────>│ 6. Signal updates     │
//!          │                       │──────────────────────>│
//!          │                       │    View re-renders    │
//! ```
//!
//! ## For Maintainers
//!
//! ### Implementing Resolvable
//!
//! To make a type resolvable, implement the [`Resolvable`] trait:
//!
//! ```ignore
//! use waterui_core::{Environment, resolve::Resolvable};
//! use nami::{Computed, Signal};
//!
//! #[derive(Debug, Clone, Copy)]
//! pub struct MyToken;
//!
//! impl Resolvable for MyToken {
//!     type Resolved = MyResolvedValue;
//!     
//!     fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
//!         // Option 1: Look up a signal from environment
//!         env.query::<Self, Computed<Self::Resolved>>()
//!             .cloned()
//!             .unwrap_or_else(|| {
//!                 // Option 2: Return a constant fallback
//!                 Computed::constant(MyResolvedValue::default())
//!             })
//!     }
//! }
//! ```
//!
//! ### Key Types
//!
//! - [`Resolvable`] - The core trait. Implementations look up values from the environment.
//! - [`AnyResolvable<T>`] - Type-erased wrapper for storing heterogeneous resolvables.
//! - [`Map<R, F>`] - Transforms a resolvable's output (e.g., adjust opacity on a color).
//!
//! ### Integration with Theme System
//!
//! The theme system uses this pattern to inject platform-specific colors and fonts:
//!
//! 1. Native backend creates `Computed<ResolvedColor>` signals from system palette
//! 2. `Theme::install()` stores these signals in the environment keyed by token type
//! 3. Token types (e.g., `color::Foreground`) implement `Resolvable` to query these signals
//! 4. When the native signal updates, all views using that token automatically update
//!
//! ### The nami Signal System
//!
//! This module integrates with [nami](https://github.com/aspect-rs/nami), WaterUI's reactive
//! primitives library. Key concepts:
//!
//! - `Signal` - A trait for values that can change over time
//! - `Computed<T>` - A cached, reactive value that re-evaluates when dependencies change
//! - `.computed()` - Converts any `impl Signal` into a `Computed` for storage/cloning

use alloc::boxed::Box;
use core::fmt::Debug;

use nami::{Computed, Signal, SignalExt};

use crate::Environment;

/// A trait for types that can be resolved to a reactive value from an environment.
///
/// This is the core abstraction for WaterUI's dynamic configuration system. Types that
/// implement `Resolvable` don't hold their final value directly—instead, they know how
/// to look up or compute that value from an [`Environment`].
///
/// # Contract
///
/// - The same `Resolvable` instance resolved against the same `Environment` should return
///   a signal that produces equivalent values (though the signal itself may be a new instance).
/// - The returned signal is **reactive**: if the underlying data in the environment changes,
///   the signal will emit updated values.
///
/// # Example
///
/// ```ignore
/// use waterui_core::{Environment, resolve::Resolvable};
/// use nami::{Computed, Signal};
///
/// /// A token representing the primary brand color.
/// #[derive(Debug, Clone, Copy)]
/// pub struct BrandColor;
///
/// impl Resolvable for BrandColor {
///     type Resolved = ResolvedColor;
///     
///     fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
///         // Query the environment for a pre-installed signal
///         env.query::<Self, Computed<ResolvedColor>>()
///             .cloned()
///             .unwrap_or_else(|| Computed::constant(ResolvedColor::default()))
///     }
/// }
/// ```
pub trait Resolvable: Debug + Clone {
    /// The concrete type produced after resolution.
    type Resolved;

    /// Resolves this value in the given environment, returning a reactive signal.
    ///
    /// The returned signal will emit the current resolved value and any future updates.
    /// Callers typically use `.get()` for one-shot reads or subscribe for continuous updates.
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved>;
}

trait ResolvableImpl<T>: Debug {
    fn resolve(&self, env: &Environment) -> Computed<T>;
    fn clone_box(&self) -> Box<dyn ResolvableImpl<T>>;
}

impl<R: Resolvable + 'static> ResolvableImpl<R::Resolved> for R {
    fn resolve(&self, env: &Environment) -> Computed<R::Resolved> {
        self.resolve(env).computed()
    }

    fn clone_box(&self) -> Box<dyn ResolvableImpl<R::Resolved>> {
        Box::new(self.clone())
    }
}

/// A type-erased wrapper for any resolvable value.
///
/// `AnyResolvable<T>` allows storing different `Resolvable` implementations that all
/// resolve to the same output type `T`. This is essential for types like `Color` and
/// `Font` which can be constructed from many different sources (hex strings, theme
/// tokens, computed values) but all resolve to the same concrete type.
///
/// # Example
///
/// ```ignore
/// use waterui_core::resolve::AnyResolvable;
///
/// // These all resolve to ResolvedColor, but come from different sources
/// let from_hex = AnyResolvable::new(Srgb::from_hex("#FF0000"));
/// let from_token = AnyResolvable::new(theme::color::Accent);
/// let from_computed = AnyResolvable::new(some_color.lighten(0.2));
/// ```
#[derive(Debug)]
pub struct AnyResolvable<T> {
    inner: Box<dyn ResolvableImpl<T>>,
}

impl<T> Resolvable for AnyResolvable<T>
where
    T: 'static + Debug,
{
    type Resolved = T;
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        self.inner.resolve(env)
    }
}

impl<T> Clone for AnyResolvable<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone_box(),
        }
    }
}
impl<T> AnyResolvable<T> {
    /// Creates a new type-erased resolvable value.
    ///
    /// # Arguments
    /// * `value` - The resolvable value to wrap
    pub fn new(value: impl Resolvable<Resolved = T> + 'static) -> Self {
        Self {
            inner: Box::new(value),
        }
    }

    /// Resolves this value in the given environment.
    ///
    /// # Arguments
    /// * `env` - The environment to resolve in
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<T> {
        self.inner.resolve(env)
    }
}

/// A mapping type that transforms a resolvable value using a function.
///
/// `Map` wraps an existing `Resolvable` and applies a transformation function to its
/// resolved output. This enables fluent APIs like `color.lighten(0.2)` or
/// `font.with_weight(Bold)` without losing reactivity.
///
/// # Example
///
/// ```ignore
/// use waterui_core::resolve::Map;
///
/// // Create a lighter version of the accent color
/// let lighter_accent = Map::new(
///     theme::color::Accent,
///     |color| color.with_lightness(color.lightness() + 0.2)
/// );
/// ```
///
/// The transformation is applied lazily when the signal emits, so if the underlying
/// `Accent` color changes (e.g., dark mode toggle), the lighter version updates too.
#[derive(Clone)]
pub struct Map<R, F> {
    resolvable: R,
    func: F,
}

impl<R: Debug, F> Debug for Map<R, F> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("With")
            .field("resolvable", &self.resolvable)
            .field("func", &"Fn(...)")
            .finish()
    }
}

impl<R, F> Map<R, F> {
    /// Creates a new mapping that transforms the resolved value using the given function.
    #[must_use]
    pub const fn new<T, U>(resolvable: R, func: F) -> Self
    where
        R: Resolvable<Resolved = T>,
        F: Fn(T) -> U + Clone + 'static,
        T: 'static,
        U: 'static,
    {
        Self { resolvable, func }
    }
}

impl<R, F, T, U> Resolvable for Map<R, F>
where
    R: Resolvable<Resolved = T>,
    F: Fn(T) -> U + Clone + 'static,
    T: 'static,
    U: 'static,
{
    type Resolved = U;
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let func = self.func.clone();
        self.resolvable.resolve(env).map(func)
    }
}
