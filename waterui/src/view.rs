pub use waterui_core::view::*;
use waterui_core::{components::Text, AnyView, Environment};

use alloc::boxed::Box;

use crate::navigation::NavigationView;

pub trait ViewExt: View + Sized {
    fn env(self, env: Environment) -> WithEnv;
    fn title(self, title: impl Into<Text>) -> NavigationView;
    fn anyview(self) -> AnyView;
}

pub trait ConfigViewExt: ConfigurableView + Sized {
    fn modifier(self, modifier: impl Into<Modifier<Self>>) -> impl View;
}

impl<V: ConfigurableView> ConfigViewExt for V {
    fn modifier(self, modifier: impl Into<Modifier<Self>>) -> impl View {
        modifier.into().modify(Environment::new(), self.config())
    }
}

pub trait ViewBuilder: 'static {
    fn view(&self, env: Environment) -> impl View;
}

impl<F, V> ViewBuilder for F
where
    F: Fn() -> V + 'static,
    V: View,
{
    fn view(&self, _env: Environment) -> impl View {
        (self)()
    }
}
pub struct AnyViewBuilder(Box<dyn Fn(Environment) -> AnyView>);

impl AnyViewBuilder {
    pub fn new(builder: impl ViewBuilder + 'static) -> Self {
        Self(Box::new(move |env| builder.view(env).anyview()))
    }
}

impl ViewBuilder for AnyViewBuilder {
    fn view(&self, env: Environment) -> impl View {
        (self.0)(env)
    }
}

impl<V: View> ViewExt for V {
    fn env(self, env: Environment) -> WithEnv {
        WithEnv::new(self, env)
    }

    fn title(self, title: impl Into<Text>) -> NavigationView {
        NavigationView::new(title, self)
    }

    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }
}

#[derive(Debug)]
pub struct TaggedView<T, V> {
    pub tag: T,
    pub view: V,
}

impl<T, V: View> TaggedView<T, V> {
    pub fn new(tag: T, view: V) -> Self {
        Self { tag, view }
    }

    pub fn map<F, T2>(self, f: F) -> TaggedView<T2, V>
    where
        F: Fn(T) -> T2,
    {
        TaggedView {
            tag: f(self.tag),
            view: self.view,
        }
    }

    pub fn erase(self) -> TaggedView<T, AnyView> {
        TaggedView {
            tag: self.tag,
            view: self.view.anyview(),
        }
    }
}
