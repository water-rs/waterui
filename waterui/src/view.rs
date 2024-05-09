use crate::modifier::{Modifier, Padding};
pub use waterui_core::view::*;
use waterui_core::{AnyView, Environment};

use alloc::{boxed::Box, rc::Rc};

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;
pub type SharedViewBuilder = Rc<dyn Fn() -> AnyView>;

pub trait ViewExt: View {
    fn modifier(self, modifier: impl Modifier) -> impl View;
    fn padding(self) -> impl View;
    fn anyview(self) -> AnyView;
}

struct WithModifier<V, M> {
    view: V,
    modifier: M,
}

impl<V, M> View for WithModifier<V, M>
where
    V: View,
    M: Modifier,
{
    fn body(self, env: &Environment) -> impl View {
        self.modifier.modify(env, self.view)
    }
}

impl<V, M> WithModifier<V, M> {
    pub fn new(view: V, modifier: M) -> Self {
        Self { view, modifier }
    }
}

impl<V: View> ViewExt for V {
    fn modifier(self, modifier: impl Modifier) -> impl View {
        WithModifier::new(self, modifier)
    }

    fn padding(self) -> impl View {
        self.modifier(Padding::default())
    }

    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }
}
