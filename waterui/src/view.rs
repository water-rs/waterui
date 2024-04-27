use core::{any::Any, future::Future};

use crate::{
    modifier::{Modifier, Padding},
    AnyView,
};

pub use waterui_view::view::*;

use alloc::{boxed::Box, rc::Rc};
use waterui_view::Environment;

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;
pub type SharedViewBuilder = Rc<dyn Fn() -> AnyView>;

struct UseEnv<V, F> {
    view: V,
    f: F,
}

impl<V, F> View for UseEnv<V, F>
where
    V: View,
    F: FnOnce(&Environment),
{
    fn body(self, env: Environment) -> impl View {
        (self.f)(&env);
        self.view
    }
}

pub trait ViewExt: View {
    fn use_env(self, f: impl FnOnce(&Environment)) -> impl View;
    fn modifier(self, modifier: impl Modifier) -> impl View;
    fn padding(self) -> impl View;
    fn task<Fut>(self, fut: Fut) -> impl View
    where
        Fut: Future + 'static,
        Fut::Output: 'static;
    fn anyview(self) -> AnyView;
}

struct WithModifier<V, M> {
    view: V,
    modifier: M,
}

impl<V, M> View for WithModifier<V, M>
where
    V: View + 'static,
    M: Modifier,
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
    fn use_env(self, f: impl FnOnce(&Environment)) -> impl View {
        UseEnv { view: self, f }
    }
    fn modifier(self, modifier: impl Modifier) -> impl View {
        WithModifier::new(self, modifier)
    }

    fn padding(self) -> impl View {
        self.modifier(Padding::default())
    }

    fn task<Fut>(self, fut: Fut) -> impl View
    where
        Fut: Future + 'static,
        Fut::Output: 'static,
    {
        self.use_env(move |env| {
            env.task(fut).detach();
        })
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
