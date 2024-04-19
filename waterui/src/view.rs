use core::{any::Any, future::Future};

use crate::{
    modifier::{self, Modifer},
    AnyView,
};

pub use waterui_view::view::*;

use alloc::{boxed::Box, rc::Rc};
use waterui_view::Environment;

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;
pub type SharedViewBuilder = Rc<dyn Fn() -> AnyView>;

pub trait ViewExt: View {
    fn modifier(self, modifier: impl Modifer) -> impl View;
    fn task<Fut>(self, fut: Fut) -> impl View
    where
        Fut: Future + 'static,
        Fut::Output: 'static;
    fn anyview(self) -> AnyView;
}

pub struct MapEnv<V, F> {
    view: V,
    f: F,
}

impl<V, F> MapEnv<V, F> {
    pub fn new(view: V, f: F) -> Self {
        Self { view, f }
    }
}

impl<V, F> View for MapEnv<V, F>
where
    V: View,
    F: FnOnce(Environment) -> Environment,
{
    fn body(self, env: Environment) -> impl View {
        self.view.body((self.f)(env))
    }
}

pub struct WithTask<V, Fut> {
    view: V,
    task: Fut,
}

impl<V, Fut> View for WithTask<V, Fut>
where
    V: View,
    Fut: Future + 'static,
    Fut::Output: 'static,
{
    fn body(self, env: waterui_view::Environment) -> impl View {
        env.task(self.task).detach();
        self.view
    }
}

struct WithModifier<V, M> {
    view: V,
    modifier: M,
}

impl<V, M> View for WithModifier<V, M>
where
    V: View,
    M: Modifer,
{
    fn body(self, env: Environment) -> impl View {
        self.modifier.modify(env, self.view)
    }
}

impl<V, M> WithModifier<V, M> {
    pub fn new(view: V, modifier: M) -> Self {
        Self { view, modifier }
    }
}

impl<V: View + 'static> ViewExt for V {
    fn modifier(self, modifier: impl Modifer) -> impl View {
        WithModifier::new(self, modifier)
    }

    fn task<Fut>(self, fut: Fut) -> impl View
    where
        Fut: Future + 'static,
        Fut::Output: 'static,
    {
        self.modifier(modifier::Task::new(fut))
    }

    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }
}

pub fn downcast<V: 'static>(view: impl View + 'static) -> Option<V> {
    let any = &mut Some(view) as &mut dyn Any;
    let any = any.downcast_mut::<Option<V>>();
    any.map(|v| v.take().unwrap())
}
