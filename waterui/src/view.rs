use core::{any::Any, future::Future};

use crate::{
    layout::{Alignment, Frame, Size},
    modifier::{Display, Modifier, ViewModifier},
    AnyView, Compute, ComputeExt,
};

pub use waterui_view::view::*;

use alloc::{boxed::Box, rc::Rc};

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;
pub type SharedViewBuilder = Rc<dyn Fn() -> AnyView>;

pub trait ViewExt: View {
    fn modifier<T: ViewModifier>(self, modifier: impl Compute<Output = T>) -> Modifier<T>;
    fn width(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn height(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn show(self, condition: impl Compute<Output = bool> + Clone) -> Modifier<Display>;
    fn leading(self) -> Modifier<Frame>;
    fn task<Fut>(self, fut: Fut) -> WithTask<Self, Fut>
    where
        Self: Sized,
        Fut: Future + 'static,
        Fut::Output: 'static;
    fn anyview(self) -> AnyView;
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

impl<V: View + 'static> ViewExt for V {
    fn modifier<T: ViewModifier>(self, modifier: impl Compute<Output = T>) -> Modifier<T> {
        Modifier::new(self.anyview(), modifier)
    }

    fn width(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.anyview(), Frame::default().width(size))
    }

    fn height(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.anyview(), Frame::default().height(size))
    }

    fn show(self, condition: impl Compute<Output = bool> + Clone) -> Modifier<Display> {
        self.modifier(condition.transform(Display::new))
    }

    fn leading(self) -> Modifier<Frame> {
        Modifier::new(
            self.anyview(),
            Frame::default().alignment(Alignment::Leading),
        )
    }

    fn task<Fut>(self, fut: Fut) -> WithTask<Self, Fut>
    where
        Self: Sized,
        Fut: Future + 'static,
        Fut::Output: 'static,
    {
        WithTask {
            view: self,
            task: fut,
        }
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
