use crate::ComputeExt;
use waterui_core::View;
use waterui_reactive::{
    compute::{ComputeResult, IntoCompute},
    watcher::WatcherGuard,
};
pub use waterui_task::*;

/// A view that executes a callback when a computed value changes.
#[derive(Debug)]
pub struct OnChange<V> {
    content: V,
    _guard: WatcherGuard,
}

impl<V> OnChange<V> {
    /// Creates a new OnChange view that will execute the provided handler
    /// whenever the source value changes.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to render
    /// * `source` - The computed value to watch for changes
    /// * `handler` - The callback to execute when the value changes
    pub fn new<T: ComputeResult>(
        content: V,
        source: impl IntoCompute<T>,
        handler: impl Fn(T) + 'static,
    ) -> Self {
        let guard = source.into_compute().watch(handler);
        Self {
            content,
            _guard: guard,
        }
    }
}

impl<V: View> View for OnChange<V> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.content
    }
}
