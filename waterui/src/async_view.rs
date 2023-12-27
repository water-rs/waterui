use std::future::Future;

use waterui_reactive::binding::Binding;

use crate::{global::DEFAULT_LOADING_VIEW, task, BoxView, Reactive, View};

pub trait AsyncView: Send + Sync {
    fn body(self) -> impl Future<Output = BoxView> + Send;
    fn loading() -> BoxView {
        DEFAULT_LOADING_VIEW.g
    }
    fn error() -> BoxView;
}

impl<V: AsyncView + 'static> View for V {
    fn body(self) -> BoxView {
        let binding = Binding::new(Self::loading());
        let handle = binding.clone();
        let output = Reactive::new(move || binding.replace(Self::loading()));

        task(async move {
            let view = self.body().await;
            handle.set(view);
        });
        Box::new(output)
    }
}
