use std::{cell::RefCell, rc::Rc};

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
    pub fn new<C, F>(content: V, source: &C, handler: F) -> Self
    where
        C: Signal<Guard = G>,
        V: View,
        C::Output: PartialEq + Clone,
        F: Fn(C::Output) + 'static,
    {
        let cache = Rc::new(RefCell::new(None));
        let guard = source.watch({
            let cache = Rc::clone(&cache);
            move |context| {
                let value = context.into_value();
                let changed = cache
                    .borrow_mut()
                    .replace(value.clone())
                    .is_none_or(|cached| cached != value);
                if changed {
                    handler(value);
                }
            }
        });
        if cache.borrow().is_none() {
            cache.borrow_mut().replace(source.get());
        }
        Self { content, guard }
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

    use nami::{Binding, Signal, binding, watcher::BoxWatcherGuard};

    #[derive(Clone)]
    struct ChangesBeforeWatch {
        source: Binding<i32>,
        replacement: i32,
    }

    impl Signal for ChangesBeforeWatch {
        type Output = i32;
        type Guard = <Binding<i32> as Signal>::Guard;

        fn get(&self) -> Self::Output {
            self.source.get()
        }

        fn watch(
            &self,
            watcher: impl Fn(nami::watcher::Context<Self::Output>) + 'static,
        ) -> Self::Guard {
            self.source.set(self.replacement);
            self.source.watch(watcher)
        }
    }

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

    #[test]
    fn establishes_initial_cache_after_subscribing() {
        let source = Binding::container(0);
        let changes_before_watch = ChangesBeforeWatch {
            source: source.clone(),
            replacement: 1,
        };
        let seen = Rc::new(RefCell::new(Vec::new()));

        let _view = OnChange::new((), &changes_before_watch, {
            let seen = Rc::clone(&seen);
            move |value| seen.borrow_mut().push(value)
        });

        source.set(1);
        source.set(2);

        assert_eq!(&*seen.borrow(), &[2]);
    }
}
