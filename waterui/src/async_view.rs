use std::{
    future::Future,
    mem::replace,
    ops::DerefMut,
    sync::{Arc, RwLock},
};

use waterui_reactive::binding::Binding;

use crate::{env::Environment, utils::task, view::ViewBuilder, Reactive, View, ViewExt};

pub struct DefaultLoadingView {
    content: ViewBuilder,
}

impl DefaultLoadingView {
    pub fn new<V: View + 'static>(builder: impl Send + Sync + 'static + Fn() -> V) -> Self {
        Self {
            content: Box::new(move || builder().anyview()),
        }
    }
}

impl Default for DefaultLoadingView {
    fn default() -> Self {
        Self::new(|| {})
    }
}

pub struct DefaultErrorView {
    content: ViewBuilder,
}

impl DefaultErrorView {
    pub fn new<V: View + 'static>(builder: impl Send + Sync + 'static + Fn() -> V) -> Self {
        Self {
            content: Box::new(move || builder().anyview()),
        }
    }
}

impl Default for DefaultErrorView {
    fn default() -> Self {
        Self::new(|| {})
    }
}

pub trait AsyncView: Send + Sync {
    fn body(
        self,
        env: Environment,
    ) -> impl Future<Output = Result<impl View, anyhow::Error>> + Send;

    fn loading(env: Environment) -> impl View {
        /*let builder = env.get::<DefaultLoadingView>().unwrap();
        (builder.content)()*/
    }

    fn error(error: anyhow::Error, env: Environment) -> impl View {
        /*let builder = env.get::<DefaultErrorView>().unwrap();
        (builder.content)()*/
    }
}

impl<V: AsyncView + 'static> View for V {
    fn body(self, env: Environment) -> impl View {
        let handle = Arc::new(RwLock::new(Self::loading(env.clone()).anyview()));

        let output = Reactive::new({
            let handle = handle.clone();
            let env = env.clone();
            move || {
                replace(
                    handle.write().unwrap().deref_mut(),
                    Self::loading(env.clone()).anyview(),
                )
            }
        });

        let future = {
            let env = env.clone();
            self.body(env)
        };

        task({
            let output = output.clone();
            async move {
                let view = future.await;

                let view = view
                    .map(|v| v.anyview())
                    .unwrap_or_else(|error| Self::error(error, env).anyview());
                *handle.write().unwrap() = view;
                output.need_update();
            }
        })
        .detach();
        output
    }
}
