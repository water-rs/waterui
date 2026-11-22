//! Asynchronous content loading with Suspense components.
//!
//! This module provides a suspense mechanism similar to React Suspense,
//! enabling components to display loading states while asynchronous content
//! is being fetched or prepared. The [`Suspense`] component automatically handles
//! the transition from loading to loaded states, providing a smooth user experience
//! for async operations.
//!
//! # Key Components
//!
//! - [`Suspense`] - Main component for wrapping async content with loading states
//! - [`SuspendedView`] - Trait for views that load asynchronously
//! - [`suspense`] - Helper creating a [`Suspense`] with the default [`loading()`] indicator
//!
//! # Basic Usage
//!
//! ```rust,no_run
//! use waterui::widget::suspense::Suspense;
//! use waterui::component::text::Text;
//!
//! async fn fetch_user_data() -> impl waterui_core::View {
//!     Text::new("User data")
//! }
//!
//! // With custom loading view
//! let view = Suspense::new(fetch_user_data())
//!     .loading(Text::new("Loading user data..."));
//!
//! // With default loading spinner
//! let view = Suspense::new(fetch_user_data());
//! ```

use core::future::Future;

use waterui_capture::ContentLedger;
use waterui_core::{Environment, View};
use waterui_text::Text;

use crate::{
    OneOf2, ViewExt,
    component::{Dynamic, Progress, loading},
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
/// use waterui::widget::suspense::Suspense;
/// use waterui::component::text::Text;
///
/// async fn fetch_data() -> Text {
///     // Simulate async data fetching
///     Text::new("Data loaded!")
/// }
///
/// let view = Suspense::new(fetch_data())
///     .loading(Text::new("Loading data..."));
/// ```
///
/// ## With Default Loading Indicator
///
/// ```rust,no_run
/// use waterui::widget::suspense::Suspense;
/// use waterui::component::text::Text;
///
/// async fn fetch_data() -> Text {
///     // Simulate async data fetching
///     Text::new("Data loaded!")
/// }
///
/// // Uses the platform indeterminate loading spinner
/// let view = Suspense::new(fetch_data());
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
/// use waterui::component::text::Text;
/// use waterui::widget::suspense::Suspense;
///
/// async fn load_user_profile() -> Text {
///     // Fetch user data from API
///     Text::new("John Doe")
/// }
///
/// // This works because Future<Output = impl View> implements SuspendedView
/// let suspense = Suspense::new(load_user_profile());
/// ```
///
/// ## Custom implementation
///
/// ```rust,no_run
/// use waterui::widget::suspense::SuspendedView;
/// use waterui_core::{Environment, View};
/// use waterui::component::text::Text;
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

impl<V: SuspendedView> Suspense<V, Progress<Text, Text>> {
    /// Creates a new `Suspense` component with the given content and the default loading spinner.
    ///
    /// # Arguments
    ///
    /// * `content` - The suspended view to be displayed when loaded
    #[must_use]
    pub fn new(content: V) -> Self {
        Self {
            content,
            loading: loading(),
        }
    }
}

impl<V, Loading> Suspense<V, Loading> {
    /// Creates a new `Suspense` component with an explicit typed `loading` view.
    #[must_use]
    pub const fn with_loading(content: V, loading: Loading) -> Self
    where
        V: SuspendedView,
        Loading: View,
    {
        Self { content, loading }
    }

    /// Sets a custom loading view to display while content is loading.
    ///
    /// # Arguments
    ///
    /// * `loading` - The view to show while content is loading
    ///
    /// # Returns
    ///
    /// A new `Suspense` with the specified loading view
    #[must_use]
    pub fn loading<Loading2: View>(self, loading: Loading2) -> Suspense<V, Loading2> {
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
        handler.set(OneOf2::A(self.loading));

        // A capture waits for the loaded content, not the loading view: the
        // entry is owed from mount and resolves once the content is swapped
        // in — after it mounted, so content that owes something of its own
        // is already recorded when this entry resolves.
        let owed = ContentLedger::owe::<Self>(env, core::any::type_name::<V>());
        let new_env = env.clone();
        // The view retains the loading task, so unmounting it cancels the
        // future instead of leaving it polled until it completes.
        view.task(async move {
            let content = SuspendedView::body(self.content, new_env).await;
            handler.set(OneOf2::B(content));
            if let Some(owed) = owed {
                owed.fulfil();
            }
        })
    }
}

/// Creates a [`Suspense`] view with the default loading indicator.
#[must_use]
pub fn suspense<V: SuspendedView>(content: V) -> Suspense<V, Progress<Text, Text>> {
    Suspense::new(content)
}
