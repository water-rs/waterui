use core::cell::RefCell;

use nami::watcher::WatcherGuard;
pub use native_executor::*;
use waterui_core::{Signal, View};

/// A view that executes a callback when a computed value changes.
#[derive(Debug)]
pub struct OnChange<V, G> {
    content: V,
    _guard: G,
}

impl<V, G> OnChange<V, G> {
    /// Creates a new `OnChange` view that will execute the provided handler
    /// whenever the source value changes.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to render
    /// * `source` - The computed value to watch for changes
    /// * `handler` - The callback to execute when the value changes
    pub fn new<C, F>(content: V, source: &C, handler: F) -> OnChange<V, C::Guard>
    where
        C: Signal,
        V: View,
        C::Output: PartialEq + Clone,
        F: Fn(C::Output) + 'static,
    {
        let cache: RefCell<Option<C::Output>> = RefCell::new(None);
        let guard = source.watch(move |context| {
            let value = context.value;
            if let Some(cache) = &mut *cache.borrow_mut()
                && *cache != value
            {
                *cache = value.clone();
                handler(value);
            }
        });
        OnChange {
            content,
            _guard: guard,
        }
    }
}

impl<V: View, G: WatcherGuard> View for OnChange<V, G> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        self.content
    }
}
