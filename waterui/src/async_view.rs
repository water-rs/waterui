use core::{cell::RefCell, future::Future};

use alloc::{boxed::Box, rc::Rc};

use crate::{component::AnyView, env::Environment, view::ViewBuilder, Computed, View, ViewExt};

pub struct DefaultLoadingView {
    builder: ViewBuilder,
}

impl DefaultLoadingView {
    pub fn new<V: View + 'static>(builder: impl 'static + Fn() -> V) -> Self {
        Self {
            builder: Box::new(move || builder().anyview()),
        }
    }

    pub fn spawn(&self) -> AnyView {
        (self.builder)()
    }
}

impl Default for DefaultLoadingView {
    fn default() -> Self {
        Self::new(|| {})
    }
}

pub struct DefaultErrorView {
    pub builder: Box<dyn Fn(anyhow::Error) -> AnyView>,
}

impl DefaultErrorView {
    pub fn new<V: View + 'static>(builder: impl 'static + Fn(anyhow::Error) -> V) -> Self {
        Self {
            builder: Box::new(move |error| builder(error).anyview()),
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
        let handle = Rc::new(RefCell::new(Some(Self::loading(env.clone()).anyview())));
        let (result, manager) = Computed::compute({
            let handle = handle.clone();
            move || handle.borrow_mut().take().unwrap()
        });

        env.task({
            let env = env.clone();
            async move {
                let output = self.body(env.clone()).await;
                manager.notify();
                match output {
                    Ok(view) => *handle.borrow_mut() = Some(view.anyview()),
                    Err(error) => *handle.borrow_mut() = Some(Self::error(error, env).anyview()),
                }
            }
        })
        .detach();
        result
    }
}
