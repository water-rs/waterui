use std::future::Future;

use waterui_reactive::binding::Binding;

use crate::{env::Environment, task, view::BoxViewBuilder, BoxView, Reactive, View};

pub struct DefaultLoadingView {
    content: BoxViewBuilder,
}

pub struct DefaultErrorView {
    content: BoxViewBuilder,
}

pub trait AsyncView: Send + Sync {
    fn body(self, env: Environment) -> impl Future<Output = BoxView> + Send;
    fn loading(env: Environment) -> BoxView {
        let builder = env.get::<DefaultLoadingView>().unwrap();
        (builder.content)()
    }
    fn error(env: Environment) -> BoxView {
        let builder = env.get::<DefaultErrorView>().unwrap();
        (builder.content)()
    }
}

impl<V: AsyncView + 'static> View for V {
    fn body(self, env: Environment) -> BoxView {
        let binding = Binding::new(Self::loading(env.clone()));
        let handle = binding.clone();
        let output_env = env.clone();
        let output = Reactive::new(move || binding.replace(Self::loading(output_env.clone())));
        let future = self.body(env);
        task(async move {
            let view = future.await;
            handle.set(view);
        });
        Box::new(output)
    }
}
