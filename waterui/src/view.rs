pub use waterui_core::view::*;
use waterui_core::{AnyView, Environment};

use alloc::boxed::Box;

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;
pub trait ViewExt: View + Sized {
    fn env(self, env: Environment) -> WithEnv;
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

impl<V: View> ViewExt for V {
    fn env(self, env: Environment) -> WithEnv {
        WithEnv::new(self, env)
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
