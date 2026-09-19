use std::{cell::RefCell, fmt, rc::Rc};

use nami::Signal;
use waterui_core::handler::{BoxedEventAction, EventHandler, boxed_event_handler};
use waterui_core::{Environment, Metadata, Retain, View};

/// A view that runs an [`EventHandler`] whenever a signal's value changes.
///
/// The handler receives the new value first and any extractor arguments after
/// it — `State<T>`, `Environment`, … — resolved from the environment the view
/// is rendered in. A `Binding` the handler writes is injected with
/// `.state(&binding)` on the view and read back as
/// `State(binding): State<Binding<T>>`, never captured by the closure.
///
/// The watcher subscribes in [`View::body`], where that environment is known,
/// and stays alive through a [`Retain`] on the content for as long as the view
/// is mounted. Consecutive equal values are collapsed: the handler runs only
/// when the value differs from the last one it saw.
pub struct OnChange<V, C: Signal> {
    content: V,
    source: C,
    handler: BoxedEventAction<C::Output>,
}

impl<V, C: Signal> fmt::Debug for OnChange<V, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnChange")
            .field("content", &std::any::type_name::<V>())
            .field("source", &std::any::type_name::<C>())
            .finish_non_exhaustive()
    }
}

impl<V, C> OnChange<V, C>
where
    V: View,
    C: Signal,
    C::Output: PartialEq + Clone,
{
    /// Creates an `OnChange` view that runs `handler` whenever `source`'s
    /// value changes.
    ///
    /// # Arguments
    ///
    /// * `content` - The view to render
    /// * `source` - The signal to watch for changes
    /// * `handler` - The [`EventHandler`] to run with each new value
    pub fn new<H, A>(content: V, source: &C, handler: H) -> Self
    where
        H: EventHandler<C::Output, A>,
    {
        Self {
            content,
            source: source.clone(),
            handler: boxed_event_handler(handler),
        }
    }
}

impl<V, C> View for OnChange<V, C>
where
    V: View,
    C: Signal,
    C::Output: PartialEq + Clone,
{
    fn body(self, env: &Environment) -> impl View {
        let Self {
            content,
            source,
            handler,
        } = self;
        let env = env.clone();
        let handler = RefCell::new(handler);
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
                    handler.borrow_mut()(value, &env);
                }
            }
        });
        if cache.borrow().is_none() {
            cache.borrow_mut().replace(source.get());
        }
        Metadata::new(content, Retain::new(guard))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use nami::{Binding, Signal, binding};
    use waterui_core::extract::State;
    use waterui_core::{Environment, View};

    use super::OnChange;

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

    /// Mounts `view` into `env`, keeping the body (and its retained watcher)
    /// alive for the returned guard's lifetime.
    fn mount(view: impl View, env: &Environment) -> impl View {
        view.body(env)
    }

    #[test]
    fn fires_on_first_update() {
        let source = Binding::i32(0);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let env = Environment::new();

        let _mounted = mount(
            OnChange::new((), &source, {
                let seen = Rc::clone(&seen);
                move |value: i32| seen.borrow_mut().push(value)
            }),
            &env,
        );

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1]);

        source.set(1);
        assert_eq!(&*seen.borrow(), &[1]);

        source.set(2);
        assert_eq!(&*seen.borrow(), &[1, 2]);
    }

    #[test]
    fn establishes_initial_cache_after_subscribing() {
        let source = Binding::i32(0);
        let changes_before_watch = ChangesBeforeWatch {
            source: source.clone(),
            replacement: 1,
        };
        let seen = Rc::new(RefCell::new(Vec::new()));
        let env = Environment::new();

        let _mounted = mount(
            OnChange::new((), &changes_before_watch, {
                let seen = Rc::clone(&seen);
                move |value: i32| seen.borrow_mut().push(value)
            }),
            &env,
        );

        source.set(1);
        source.set(2);

        assert_eq!(&*seen.borrow(), &[2]);
    }

    #[test]
    fn handler_extracts_injected_state() {
        let source = binding(0_i32);
        let total = Binding::i32(0);
        let mut env = Environment::new();
        env.insert(State(total.clone()));

        let _mounted = mount(
            OnChange::new(
                (),
                &source,
                |value: i32, State(total): State<Binding<i32>>| total.add_assign(value),
            ),
            &env,
        );

        source.set(2);
        source.set(3);

        assert_eq!(total.get(), 5);
    }

    #[test]
    fn nothing_fires_before_the_view_is_mounted() {
        let source = Binding::i32(0);
        let seen = Rc::new(RefCell::new(Vec::new()));

        let view = OnChange::new((), &source, {
            let seen = Rc::clone(&seen);
            move |value: i32| seen.borrow_mut().push(value)
        });
        source.set(1);
        assert!(seen.borrow().is_empty());

        let env = Environment::new();
        let _mounted = mount(view, &env);
        source.set(2);
        assert_eq!(&*seen.borrow(), &[2]);
    }
}
