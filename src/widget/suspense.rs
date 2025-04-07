use core::future::Future;

use waterui_core::{AnyView, Environment, View};
use waterui_task::LocalTask;

use crate::{
    ViewExt,
    component::Dynamic,
    view::{AnyViewBuilder, ViewBuilder},
};

#[derive(Debug)]
pub struct Suspense<V, Loading> {
    content: V,
    loading: Loading,
}

pub trait SuspendedView: 'static {
    fn body(self, _env: Environment) -> impl Future<Output = impl View>;
}

impl<Fut, V> SuspendedView for Fut
where
    Fut: Future<Output = V> + 'static,
    V: View,
{
    fn body(self, _env: Environment) -> impl Future<Output = impl View> {
        self
    }
}

#[derive(Debug)]
pub struct DefaultLoadingView(AnyViewBuilder);
#[derive(Debug)]
pub struct UseDefaultLoadingView;

impl View for UseDefaultLoadingView {
    fn body(self, env: &Environment) -> impl View {
        if let Some(builder) = env.get::<DefaultLoadingView>() {
            builder.0.view(env).anyview()
        } else {
            AnyView::new(())
        }
    }
}

impl<V: SuspendedView> Suspense<V, UseDefaultLoadingView> {
    pub fn new(content: V) -> Self {
        Self {
            content,
            loading: UseDefaultLoadingView,
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
        let (handler, view) = Dynamic::new();
        handler.set(self.loading);

        let new_env = env.clone();
        LocalTask::new(async move {
            let content = SuspendedView::body(self.content, new_env).await;
            handler.set(content);
        });

        view
    }
}
