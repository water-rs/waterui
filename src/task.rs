use waterui_core::View;
use waterui_reactive::{
    compute::{ComputeResult, IntoCompute},
    watcher::WatcherGuard,
};

use crate::ComputeExt;

#[derive(Debug)]
pub struct OnChange<V> {
    content: V,
    _guard: WatcherGuard,
}

impl<V> OnChange<V> {
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
