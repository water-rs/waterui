use core::future::Future;

use alloc::boxed::Box;

use crate::{env::Environment, AnyView, DynamicView, View, ViewBuilder};
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

pub struct DefaultErrorView {
    pub builder: Box<dyn Fn(anyhow::Error) -> AnyView>,
}

impl DefaultErrorView {
    pub fn new<V: View + 'static>(builder: impl 'static + Fn(anyhow::Error) -> V) -> Self {
        Self {
            builder: Box::new(move |error| AnyView::new(builder(error))),
        }
    }

    pub fn spawn(&self, error: anyhow::Error) -> AnyView {
        (self.builder)(error)
    }
}

impl Default for DefaultErrorView {
    fn default() -> Self {
        Self::new(|_| {})
    }
}

pub trait AsyncView {
    fn body(self, env: Environment) -> impl Future<Output = Result<impl View, anyhow::Error>>;

    fn loading(env: Environment) -> impl View {
        let builder = env.get::<DefaultLoadingView>().unwrap();
        builder.spawn()
    }

    fn error(error: anyhow::Error, env: Environment) -> impl View {
        let builder = env.get::<DefaultErrorView>().unwrap();
        builder.spawn(error)
    }
}

impl<V: AsyncView + 'static> View for V {
    fn body(self, env: Environment) -> impl View {
        let (view, handle) = {
            let env = env.clone();
            DynamicView::new(move || V::loading(env.clone()))
        };

        let executor = env.executor();
        executor
            .spawn(async move {
                match self.body(env.clone()).await {
                    Ok(view) => handle.set(view),
                    Err(error) => handle.set(V::error(error, env)),
                }
            })
            .detach();
        view
    }
}
