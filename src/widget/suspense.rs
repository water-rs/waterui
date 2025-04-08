//! Provides the suspense component for handling asynchronous content loading.
//!
//! This module implements a suspense mechanism similar to React Suspense,
//! allowing components to show loading states while async content is being prepared.

use core::future::Future;

use waterui_core::{AnyView, Environment, View};
use waterui_task::LocalTask;

use crate::{
    ViewExt,
    component::Dynamic,
    view::{AnyViewBuilder, ViewBuilder},
};

/// A component that displays a loading view while waiting for content to load.
///
/// `Suspense` takes two generic parameters:
/// - `V`: The suspended view that will be shown once loaded
/// - `Loading`: The view to display while content is loading
///
/// # Example
/// ```
/// let view = Suspense::new(async_content())
///     .loading(Text::new("Loading..."));
/// ```
#[derive(Debug)]
pub struct Suspense<V, Loading> {
    content: V,
    loading: Loading,
}

/// Trait for views that can be suspended (loaded asynchronously).
///
/// Implement this trait to create content that can be loaded
/// asynchronously within a `Suspense` component.
pub trait SuspendedView: 'static {
    /// Takes an environment and returns a future that resolves to a view.
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
/// This is typically set in the environment and used by `UseDefaultLoadingView`.
#[derive(Debug)]
pub struct DefaultLoadingView(AnyViewBuilder);

/// A view that renders the default loading state from the environment.
///
/// If no default loading view is found in the environment, it renders an empty view.
#[derive(Debug)]
pub struct UseDefaultLoadingView;

impl View for UseDefaultLoadingView {
    fn body(self, env: &Environment) -> impl View {
        if let Some(builder) = env.get::<DefaultLoadingView>() {
            builder.0.view(env).anyview()
        } else {
            AnyView::new(())
        }
    }
}

impl<V: SuspendedView> Suspense<V, UseDefaultLoadingView> {
    /// Creates a new `Suspense` component with the given content and the default loading view.
    ///
    /// # Arguments
    ///
    /// * `content` - The suspended view to be displayed when loaded
    pub fn new(content: V) -> Self {
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
        LocalTask::new(async move {
            let content = SuspendedView::body(self.content, new_env).await;
            handler.set(content);
        });

        view
    }
}
