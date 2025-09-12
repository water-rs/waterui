//! Asynchronous content loading with Suspense components.
//!
//! This module provides a suspense mechanism similar to React Suspense,
//! enabling components to display loading states while asynchronous content
//! is being fetched or prepared. The `Suspense` component automatically handles
//! the transition from loading to loaded states, providing a smooth user experience
//! for async operations.
//!
//! # Key Components
//!
//! - [`Suspense`] - Main component for wrapping async content with loading states
//! - [`SuspendedView`] - Trait for views that load asynchronously
//! - [`UseDefaultLoadingView`] - Uses environment-provided default loading view
//! - [`DefaultLoadingView`] - Container for setting default loading view in environment
//!
//! # Basic Usage
//!
//! ```rust,no_run
//! use waterui::Suspense;
//! use waterui_text::Text;
//!
//! // With custom loading view
//! let view = Suspense::new(fetch_user_data())
//!     .loading(Text::new("Loading user data..."));
//!
//! // With default loading view from environment
//! let view = Suspense::new(fetch_user_data());
//! ```

use core::future::Future;

use executor_core::{Task, spawn_local};
use waterui_core::{AnyView, Environment, View};

use crate::{
    ViewExt,
    component::Dynamic,
    view::{AnyViewBuilder, ViewBuilder},
};

/// A component that manages asynchronous content loading with loading states.
///
/// `Suspense` wraps async content and displays a loading view while the content
/// is being prepared. Once the async operation completes, it automatically switches
/// to showing the loaded content.
///
/// # Type Parameters
///
/// - `V`: The suspended view implementing [`SuspendedView`] that will be shown once loaded
/// - `Loading`: The view to display while content is loading
///
/// # Examples
///
/// ## With Custom Loading View
///
/// ```rust,no_run
/// use waterui::Suspense;
/// use waterui_text::Text;
///
/// async fn fetch_data() -> Text {
///     // Simulate async data fetching
///     Text::new("Data loaded!")
/// }
///
/// let view = Suspense::new(fetch_data)
///     .loading(Text::new("Loading data..."));
/// ```
///
/// ## With Default Loading View
///
/// ```rust,no_run
/// use waterui::Suspense;
///
/// // Uses the default loading view from environment
/// let view = Suspense::new(fetch_data);
/// ```
#[derive(Debug)]
pub struct Suspense<V, Loading> {
    content: V,
    loading: Loading,
}

/// Trait for views that can be loaded asynchronously within a `Suspense` component.
///
/// This trait defines how content should be loaded asynchronously. Any type that
/// implements this trait can be used as the content in a [`Suspense`] component.
///
/// The trait is automatically implemented for any `Future` that resolves to a `View`,
/// making it easy to use async functions directly with `Suspense`.
///
/// # Examples
///
/// ## Using with async functions
///
/// ```rust,no_run
/// use waterui_text::Text;
///
/// async fn load_user_profile() -> Text {
///     // Fetch user data from API
///     Text::new("John Doe")
/// }
///
/// // This works because Future<Output = impl View> implements SuspendedView
/// let suspense = Suspense::new(load_user_profile);
/// ```
///
/// ## Custom implementation
///
/// ```rust,no_run
/// use waterui::SuspendedView;
/// use waterui_core::{Environment, View};
/// use waterui_text::Text;
///
/// struct UserLoader {
///     user_id: u32,
/// }
///
/// impl SuspendedView for UserLoader {
///     async fn body(self, _env: Environment) -> impl View {
///         // Custom loading logic
///         Text::new(format!("User {}", self.user_id))
///     }
/// }
/// ```
pub trait SuspendedView: 'static {
    /// Takes an environment and returns a future that resolves to a view.
    ///
    /// This method is called when the suspense component needs to load the content.
    /// The returned future will be executed asynchronously, and its result will be
    /// displayed once it completes.
    ///
    /// # Parameters
    ///
    /// * `_env` - The current environment context, which can be used to access
    ///   shared state or configuration during loading
    fn body(self, _env: Environment) -> impl Future<Output = impl View>;
}

impl<Fut, V> SuspendedView for Fut
where
    Fut: Future<Output = V> + 'static,
    V: View,
{
    fn body(self, _env: Environment) -> impl Future<Output = impl View> {
        self
    }
}

/// Container for the default loading view builder.
///
/// This type is used to store a default loading view in the [`Environment`] that can be
/// accessed by [`UseDefaultLoadingView`]. Applications can set this in their environment
/// to provide a consistent loading experience across all suspense components.
///
/// # Example
///
/// ```rust,no_run
/// use waterui::{DefaultLoadingView, Environment};
/// use waterui_text::Text;
/// use waterui_core::view::ViewBuilder;
///
/// // Using ViewBuilder for lazy initialization
/// let loading_view = ViewBuilder::new(|| Text::new("Loading..."));
/// let env = Environment::new().with(DefaultLoadingView(loading_view.anybuilder()));
///
/// // Or using FnOnce directly (also implements View for lazy initialization)
/// let env = Environment::new().with(DefaultLoadingView(
///     ViewBuilder::new(|| Text::new("Loading...")).anybuilder()
/// ));
/// ```
#[derive(Debug)]
pub struct DefaultLoadingView(AnyViewBuilder);

/// A view that renders the default loading state from the environment.
///
/// This component looks for a [`DefaultLoadingView`] in the current environment
/// and renders it. If no default loading view is configured, it renders an empty view.
/// This provides a fallback mechanism for suspense components that don't specify
/// a custom loading view.
///
/// # Example
///
/// ```rust,no_run
/// use waterui::{Suspense, UseDefaultLoadingView};
///
/// // This will use the default loading view from environment
/// let view = Suspense::new(async_content());
/// // Equivalent to:
/// let view = Suspense::new(async_content()).loading(UseDefaultLoadingView);
/// ```
#[derive(Debug)]
pub struct UseDefaultLoadingView;

impl View for UseDefaultLoadingView {
    fn body(self, env: &Environment) -> impl View {
        env.get::<DefaultLoadingView>()
            .map_or_else(|| AnyView::new(()), |builder| builder.0.view(env).anyview())
    }
}

impl<V: SuspendedView> Suspense<V, UseDefaultLoadingView> {
    /// Creates a new `Suspense` component with the given content and the default loading view.
    ///
    /// # Arguments
    ///
    /// * `content` - The suspended view to be displayed when loaded
    pub const fn new(content: V) -> Self {
        Self {
            content,
            loading: UseDefaultLoadingView,
        }
    }
}

impl<V, Loading> Suspense<V, Loading> {
    /// Sets a custom loading view to display while content is loading.
    ///
    /// # Arguments
    ///
    /// * `loading` - The view to show while content is loading
    ///
    /// # Returns
    ///
    /// A new `Suspense` with the specified loading view
    pub fn loading<Loading2, Output: View>(self, loading: Loading2) -> Suspense<V, Loading2> {
        Suspense {
            content: self.content,
            loading,
        }
    }
}

impl<V, Loading> View for Suspense<V, Loading>
where
    V: SuspendedView,
    Loading: View,
{
    fn body(self, env: &Environment) -> impl View {
        let (handler, view) = Dynamic::new();
        handler.set(self.loading);

        let new_env = env.clone();
        spawn_local(async move {
            let content = SuspendedView::body(self.content, new_env).await;
            handler.set(content);
        })
        .detach();

        view
    }
}
