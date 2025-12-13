use std::cell::RefCell;

use nami::{Signal, watcher::WatcherGuard};
use waterui_core::{Metadata, Retain, View};

/// A view that executes a callback when a computed value changes.
#[derive(Debug)]
pub struct OnChange<V, G> {
    content: V,
    guard: G,
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
        let cache: RefCell<Option<C::Output>> = RefCell::new(Some(source.get()));
        let guard = source.watch(move |context| {
            let value = context.into_value();
            let mut cache_ref = cache.borrow_mut();
            match cache_ref.as_ref() {
                Some(cached) if *cached != value => {
                    handler(value.clone());
                }
                Some(_) => {
                    // Value unchanged, do nothing
                }
                None => {
                    handler(value.clone());
                }
            }
            *cache_ref = Some(value);
        });
        OnChange { content, guard }
    }
}

impl<V: View, G: WatcherGuard> View for OnChange<V, G> {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        Metadata::new(self.content, Retain::new(self.guard))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nami::binding;
    use nami::watcher::BoxWatcherGuard;

    use super::OnChange;

    #[test]
    fn fires_on_first_update() {
        let source = binding(0);
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _view: OnChange<(), BoxWatcherGuard> =
            OnChange::<(), BoxWatcherGuard>::new((), &source, {
                let seen = Rc::clone(&seen);
                move |value| seen.borrow_mut().push(value)
            });

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1]);

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1]);

        source.set(2);
        assert_eq!(&*seen.borrow(), &[1, 2]);
    }
}
