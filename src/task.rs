use core::cell::RefCell;

use crate::ComputeExt;
use waterui_core::{Compute, View};
use waterui_reactive::watcher::WatcherGuard;
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
    pub fn new<C, F>(content: V, source: C, handler: F) -> Self
    where
        C: Compute,
        V: View,
        C::Output: PartialEq + Clone,
        F: Fn(C::Output) + 'static,
    {
        let cache: RefCell<Option<C::Output>> = RefCell::new(None);
        let guard = source.watch(move |value| {
            if let Some(cache) = &mut *cache.borrow_mut() {
                if *cache != value {
                    *cache = value.clone();
                    handler(value)
                }
            }
        });
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
