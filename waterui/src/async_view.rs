use std::future::Future;

use waterui_reactive::binding::Binding;

use crate::{env::Environment, view::ViewBuilder, Reactive, View, ViewExt};

pub struct DefaultLoadingView {
    content: ViewBuilder,
}

pub struct DefaultErrorView {
    content: ViewBuilder,
}

pub trait AsyncView: Send + Sync {
    fn body(self, env: Environment) -> impl Future<Output = impl View> + Send;

    fn loading(env: Environment) -> impl View {
        let builder = env.get::<DefaultLoadingView>().unwrap();
        (builder.content)()
    }

    fn error(env: Environment) -> impl View {
        let builder = env.get::<DefaultErrorView>().unwrap();
        (builder.content)()
    }
}

impl<V: AsyncView + 'static> View for V {
    fn body(self, env: Environment) -> impl View {
        let binding = Binding::new(Self::loading(env.clone()).anyview());
        let handle = binding.clone();
        let output_env = env.clone();
        let output =
            Reactive::new(move || binding.replace(Self::loading(output_env.clone()).anyview()));
        let future = self.body(env.clone());
        env.task(async move {
            let view = future.await;
            handle.set(view.anyview());
        })
        .detach();
        output
    }
}
