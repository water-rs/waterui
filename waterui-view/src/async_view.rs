use core::future::Future;

use alloc::boxed::Box;

use crate::{env::Environment, view::ViewBuilder, AnyView, DynamicView, View};
pub struct DefaultLoadingView {
    builder: ViewBuilder,
}

impl DefaultLoadingView {
    pub fn new<V: View + 'static>(builder: impl 'static + Fn() -> V) -> Self {
        Self {
            builder: Box::new(move || AnyView::new(builder())),
        }
    }

    pub fn spawn(&self) -> AnyView {
        (self.builder)()
    }
}

pub trait AsyncView {
    fn body(self, env: Environment) -> impl Future<Output = impl View>;

    fn loading(env: Environment) -> impl View {
        env.default_loading_view()
    }
}

impl<V: AsyncView + 'static> View for V {
    fn body(self, env: Environment) -> impl View {
        let (view, handle) = {
            let env = env.clone();
            DynamicView::new(move || V::loading(env.clone()))
        };
        let fut;

        {
            let env = env.clone();
            fut = async move { handle.set(self.body(env.clone()).await) };
        }

        env.executor().spawn(fut).detach();
        view
    }
}
