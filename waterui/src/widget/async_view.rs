use crate::task;
use crate::{View, ViewExt};
use std::future::Future;

use waterui_core::view::BoxView;
use waterui_core::view::IntoView;
use waterui_core::Reactive;

use crate::widget::text;

pub struct AsyncView<F, LoadingViewBuilder, ErrorViewBuilder> {
    f: F,
    loading_view: LoadingViewBuilder,
    error_view: ErrorViewBuilder,
}

impl<F> AsyncView<F, (), ()> {
    pub fn new<Fut, V>(f: F) -> Self
    where
        F: 'static + Fn() -> Fut,
        V: IntoView,
        Fut: Future<Output = Result<V, anyhow::Error>> + 'static,
    {
        Self {
            f,

            loading_view: (),
            error_view: (),
        }
    }

    pub fn loading<Builder, V>(self, builder: Builder) -> AsyncView<F, Builder, ()>
    where
        Builder: Fn() -> V,
        V: IntoView,
    {
        AsyncView {
            loading_view: builder,
            error_view: (),
            f: self.f,
        }
    }

    pub fn error<Builder, V>(self, builder: Builder) -> AsyncView<F, (), Builder>
    where
        Builder: Fn() -> V,
        V: IntoView,
    {
        AsyncView {
            loading_view: (),
            error_view: builder,
            f: self.f,
        }
    }
}

impl<F, ErrorViewBuilder, ErrorView> AsyncView<F, (), ErrorViewBuilder>
where
    ErrorViewBuilder: Fn() -> ErrorView,
    ErrorView: IntoView,
{
    pub fn loading<Builder, V>(self, builder: Builder) -> AsyncView<F, Builder, ErrorViewBuilder>
    where
        Builder: Fn() -> V,
        V: IntoView,
    {
        AsyncView {
            loading_view: builder,
            error_view: self.error_view,
            f: self.f,
        }
    }
}

impl<F, LoadingViewBuilder, LoadingView> AsyncView<F, LoadingViewBuilder, ()>
where
    LoadingViewBuilder: Fn() -> LoadingView,
    LoadingView: IntoView,
{
    pub fn error<Builder, V>(self, builder: Builder) -> AsyncView<F, LoadingViewBuilder, Builder>
    where
        Builder: Fn() -> V,
        V: IntoView,
    {
        AsyncView {
            loading_view: self.loading_view,
            error_view: builder,
            f: self.f,
        }
    }
}

impl<F, Content, Fut> View for AsyncView<F, (), ()>
where
    F: 'static + Send + Sync + Fn() -> Fut,
    Content: IntoView,
    Fut: Send + Future<Output = Result<Content, anyhow::Error>> + 'static,
{
    fn body(self) -> BoxView {
        let view = Reactive::new_with_updater(move || default_loading_view().boxed());
        let handler = view.clone();
        let retry = move || {
            task(async move {
                let view = match (self.f)().await {
                    Ok(content) => content.into_boxed_view(),
                    Err(error) => default_error_view(error).boxed(),
                };
                handler.set(view);
            })
        };

        retry();

        view.boxed()
    }
}

fn default_loading_view() -> impl View {
    text("loading...")
}

fn default_error_view(error: anyhow::Error) -> impl View {
    text(format!("Error :{error}"))
}
