use crate::task;
use crate::view;
use std::future::Future;
use std::mem::take;
use std::ops::DerefMut;
use waterui_core::binding::Binding;
use waterui_core::view::BoxView;
use waterui_core::view::IntoView;

use crate::{View, ViewExt};

use crate::widget::text;

#[view(use_core)]
pub struct AsyncView<LoadingViewBuilder, ErrorViewBuilder> {
    #[state]
    content: AsyncViewState,
    loading_view: LoadingViewBuilder,
    error_view: ErrorViewBuilder,
    retry: Box<dyn Fn()>,
}

enum AsyncViewState {
    Initial,
    Loading,
    Ready(BoxView),
    Fail(anyhow::Error),
}

impl Default for AsyncViewState {
    fn default() -> Self {
        Self::Initial
    }
}

impl AsyncView<(), ()> {
    pub fn new<F, Fut, V>(f: F) -> Self
    where
        F: 'static + Fn() -> Fut,
        V: IntoView,
        Fut: Future<Output = Result<V, anyhow::Error>> + 'static,
    {
        let binding = Binding::new(AsyncViewState::Initial);
        Self {
            content: binding.clone(),
            loading_view: (),
            error_view: (),
            retry: Box::new(move || {
                let binding = binding.clone();
                let fut = f();
                binding.set(AsyncViewState::Loading);
                task(async move {
                    let result = fut.await;
                    match result {
                        Ok(view) => binding.set(AsyncViewState::Ready(view.into_boxed_view())),
                        Err(error) => binding.set(AsyncViewState::Fail(error)),
                    }
                });
            }),
        }
    }
}

impl<LoadingView, LoadingViewBuilder, ErrorView, ErrorViewBuilder> View
    for AsyncView<LoadingViewBuilder, ErrorViewBuilder>
where
    LoadingView: IntoView,
    LoadingViewBuilder: Fn() -> LoadingView,
    ErrorView: IntoView,
    ErrorViewBuilder: Fn(anyhow::Error) -> ErrorView,
{
    fn view(&self) -> BoxView {
        let state = take(self.content.get_mut().deref_mut());
        match state {
            AsyncViewState::Initial => {
                (self.retry)();
                (self.loading_view)().into_boxed_view()
            }
            AsyncViewState::Loading => (self.loading_view)().into_boxed_view(),
            AsyncViewState::Ready(content) => content,
            AsyncViewState::Fail(error) => (self.error_view)(error).into_boxed_view(),
        }
    }
}

impl View for AsyncView<(), ()> {
    fn view(&self) -> BoxView {
        let state = take(self.content.get_mut().deref_mut());

        match state {
            AsyncViewState::Initial => {
                (self.retry)();
                default_loading_view().boxed()
            }
            AsyncViewState::Loading => default_loading_view().boxed(),
            AsyncViewState::Ready(content) => content,
            AsyncViewState::Fail(error) => default_error_view(error).boxed(),
        }
    }
}

fn default_loading_view() -> impl View {
    text("loading...")
}

fn default_error_view(error: anyhow::Error) -> impl View {
    text(format!("Error :{error}"))
}
