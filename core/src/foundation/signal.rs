//! Signal composition utilities.

use alloc::rc::Rc;
use core::cell::{Cell, RefCell};

use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use nami::{Computed, Signal};

#[derive(Clone)]
struct FlattenSignal<S> {
    nested: S,
}

struct FlattenSignalWatchGuard<G>
where
    G: WatcherGuard,
{
    _outer: G,
    _inner: Rc<RefCell<Option<BoxWatcherGuard>>>,
}

impl<G: WatcherGuard> WatcherGuard for FlattenSignalWatchGuard<G> {}

impl<S, T> Signal for FlattenSignal<S>
where
    S: Signal<Output = Computed<T>> + Clone + 'static,
    T: Clone + 'static,
{
    type Output = T;
    type Guard = FlattenSignalWatchGuard<S::Guard>;

    fn get(&self) -> Self::Output {
        self.nested.get().get()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let watcher = Rc::new(watcher);
        let inner = Rc::new(RefCell::new(None));
        let outer_revision = Rc::new(Cell::new(0_usize));

        let outer = self.nested.watch({
            let watcher = watcher.clone();
            let inner = inner.clone();
            let outer_revision = outer_revision.clone();
            move |ctx: Context<Computed<T>>| {
                let revision = outer_revision.get() + 1;
                outer_revision.set(revision);
                let next = ctx.value().clone();
                let inner_emitted = Rc::new(Cell::new(false));
                let guard = next.watch({
                    let watcher = watcher.clone();
                    let inner_emitted = inner_emitted.clone();
                    move |ctx| {
                        inner_emitted.set(true);
                        watcher(ctx);
                    }
                });
                if outer_revision.get() == revision {
                    *inner.borrow_mut() = Some(guard);
                    if !inner_emitted.get() {
                        watcher(Context::new(next.get(), ctx.metadata().clone()));
                    }
                }
            }
        });

        if outer_revision.get() == 0 {
            let initial = self.nested.get();
            let guard = initial.watch({
                let watcher = watcher;
                move |ctx| watcher(ctx)
            });
            if outer_revision.get() == 0 {
                *inner.borrow_mut() = Some(guard);
            }
        }

        FlattenSignalWatchGuard {
            _outer: outer,
            _inner: inner,
        }
    }
}

/// Flattens a signal whose current value is another computed signal.
///
/// The returned signal tracks both replacement of the inner signal and updates
/// emitted by whichever inner signal is currently active.
#[must_use]
pub fn flatten_signal<S, T>(nested: S) -> Computed<T>
where
    S: Signal<Output = Computed<T>> + Clone + 'static,
    T: Clone + 'static,
{
    Computed::new(FlattenSignal { nested })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use nami::{Binding, SignalExt};

    #[derive(Clone)]
    struct ReplacesBeforeWatch {
        source: Binding<Computed<i32>>,
        replacement: Computed<i32>,
    }

    impl Signal for ReplacesBeforeWatch {
        type Output = Computed<i32>;
        type Guard = <Binding<Computed<i32>> as Signal>::Guard;

        fn get(&self) -> Self::Output {
            self.source.get()
        }

        fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
            self.source.set(self.replacement.clone());
            self.source.watch(watcher)
        }
    }

    #[test]
    fn watches_only_the_current_inner_signal() {
        let first = Binding::container(1);
        let second = Binding::container(10);
        let outer = Binding::container(first.computed());
        let flattened = flatten_signal(outer.clone());
        let updates = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&updates);
        let _guard = flattened.watch(move |context| {
            captured.borrow_mut().push(context.into_value());
        });

        first.set(2);
        outer.set(second.computed());
        first.set(3);
        second.set(11);

        assert_eq!(*updates.borrow(), vec![2, 10, 11]);
        assert_eq!(flattened.get(), 11);
    }

    #[test]
    fn subscribes_to_outer_before_reading_its_initial_inner_signal() {
        let first = Binding::container(1);
        let second = Binding::container(10);
        let source = Binding::container(first.computed());
        let flattened = flatten_signal(ReplacesBeforeWatch {
            source,
            replacement: second.computed(),
        });
        let updates = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&updates);
        let _guard = flattened.watch(move |context| {
            captured.borrow_mut().push(context.into_value());
        });

        first.set(2);
        second.set(11);

        assert_eq!(*updates.borrow(), vec![11]);
        assert_eq!(flattened.get(), 11);
    }
}
