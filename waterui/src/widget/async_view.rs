use crate::task;
use crate::view;
use std::future::Future;
use std::mem::take;
use std::ops::DerefMut;
use waterui_core::binding::Binding;
use waterui_core::view::BoxView;

use crate::view::{View, ViewExt};

use crate::widget::text;

#[view(use_core)]
pub struct AsyncView<LoadingViewBuilder, ErrorViewBuilder> {
    #[state]
    content: AsyncViewState,
    loading_view: LoadingViewBuilder,
    error_view: ErrorViewBuilder,
    retry: Box<dyn FnMut()>,
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
    pub fn new<F, Fut, V>(mut f: F) -> Self
    where
        F: 'static + FnMut() -> Fut,
        V: View + 'static,
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
                        Ok(view) => binding.set(AsyncViewState::Ready(view.boxed())),
                        Err(error) => binding.set(AsyncViewState::Fail(error)),
                    }
                });
            }),
        }
    }

    pub fn once<F, Fut, V>(f: F)
    where
        F: 'static + FnOnce() -> Fut,
        V: View + 'static,
        Fut: Future<Output = Result<V, anyhow::Error>> + 'static,
    {
        let mut f = Some(f);
        Self::new(move || {
            let f = f.take();
            async move {
                if let Some(f) = f {
                    f().await
                } else {
                    Err(anyhow::Error::msg("Once async view cannot retry"))
                }
            }
        });
    }
}

impl<LoadingView, LoadingViewBuilder, ErrorView, ErrorViewBuilder> View
    for AsyncView<LoadingViewBuilder, ErrorViewBuilder>
where
    LoadingView: View + 'static,
    LoadingViewBuilder: Fn() -> LoadingView,
    ErrorView: View + 'static,
    ErrorViewBuilder: Fn(anyhow::Error) -> ErrorView,
{
    fn view(&mut self) -> BoxView {
        let state = take(self.content.get_mut().deref_mut());
        match state {
            AsyncViewState::Initial => {
                (self.retry)();
                (self.loading_view)().boxed()
            }
            AsyncViewState::Loading => (self.loading_view)().boxed(),
            AsyncViewState::Ready(content) => content,
            AsyncViewState::Fail(error) => (self.error_view)(error).boxed(),
        }
    }
}

impl View for AsyncView<(), ()> {
    fn view(&mut self) -> BoxView {
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
