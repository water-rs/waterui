use core::future::Future;

use waterui_core::{Environment, View};

use crate::component::Dynamic;

#[derive(Debug)]
pub struct Suspense<V, Loading> {
    content: V,
    loading: Loading,
}

pub trait SuspendedView: 'static {
    fn body(self, _env: &Environment) -> impl Future<Output = impl View>;
}

impl<Fut, V> SuspendedView for Fut
where
    Fut: Future<Output = V> + 'static,
    V: View,
{
    fn body(self, _env: &Environment) -> impl Future<Output = impl View> {
        self
    }
}

pub struct DefaultLoadingView;
impl View for DefaultLoadingView {
    fn body(self, env: &Environment) -> impl View {
        env.default_loading_view()
    }
}

impl<V: SuspendedView> Suspense<V, DefaultLoadingView> {
    pub fn new(content: V) -> Self {
        Self {
            content,
            loading: DefaultLoadingView,
        }
    }
}

impl<V, Loading> Suspense<V, Loading> {
    pub fn loading<Loading2, Output: View>(self, loading: Loading2) -> Suspense<V, Loading2> {
        Suspense {
            content: self.content,
            loading,
        }
    }
}

impl<V, Loading> View for Suspense<V, Loading>
where
    V: SuspendedView,
    Loading: View,
{
    fn body(self, env: &Environment) -> impl View {
        let (view, handler) = Dynamic::new();
        handler.set(self.loading);

        let new_env = env.clone();
        env.task(async move {
            let content = SuspendedView::body(self.content, &new_env).await;
            handler.set(content);
        })
        .detach();

        view
    }
}
