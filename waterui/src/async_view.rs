use std::{
    future::Future,
    sync::{Arc, RwLock},
};

use crate::{
    component::AnyView, env::Environment, utils::task, view::ViewBuilder, Computed, View, ViewExt,
};

pub struct DefaultLoadingView {
    builder: ViewBuilder,
}

impl DefaultLoadingView {
    pub fn new<V: View + 'static>(builder: impl Send + Sync + 'static + Fn() -> V) -> Self {
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
    pub builder: Box<dyn Send + Sync + Fn(anyhow::Error) -> AnyView>,
}

impl DefaultErrorView {
    pub fn new<V: View + 'static>(
        builder: impl Send + Sync + 'static + Fn(anyhow::Error) -> V,
    ) -> Self {
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

pub trait AsyncView: Send + Sync {
    fn body(
        self,
        env: Environment,
    ) -> impl Future<Output = Result<impl View, anyhow::Error>> + Send;

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
        let handle = Arc::new(RwLock::new(Some(Self::loading(env.clone()).anyview())));
        let (result, manager) = Computed::compute({
            let handle = handle.clone();
            move || handle.write().unwrap().take().unwrap()
        });

        task({
            let env = env.clone();
            async move {
                let output = self.body(env.clone()).await;
                manager.notify();
                match output {
                    Ok(view) => *handle.write().unwrap() = Some(view.anyview()),
                    Err(error) => {
                        *handle.write().unwrap() = Some(Self::error(error, env).anyview())
                    }
                }
            }
        })
        .detach();
        result
    }
}
