use crate::utils::task;
use crate::view;
use std::future::Future;
use std::mem::take;
use std::ops::DerefMut;
use waterui_core::binding::Binding;
use waterui_core::view::BoxView;

use crate::view::{View, ViewExt};

use std::error::Error as StdError;

use crate::component::text;
type BoxError = Box<dyn StdError>;

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
    Fail(BoxError),
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
        V: View + 'static,
        Fut: Future<Output = Result<V, BoxError>>,
    {
        let binding = Binding::new(AsyncViewState::Initial);
        Self {
            content: binding.clone(),
            loading_view: (),
            error_view: (),
            retry: Box::new(move || {
                task(async {
                    binding.set(AsyncViewState::Loading);
                    let result = f().await;
                    match result {
                        Ok(view) => binding.set(AsyncViewState::Ready(view.boxed())),
                        Err(error) => binding.set(AsyncViewState::Fail(error)),
                    }
                })
            }),
        }
    }
}

impl<LoadingView, LoadingViewBuilder, ErrorView, ErrorViewBuilder> View
    for AsyncView<LoadingViewBuilder, ErrorViewBuilder>
where
    LoadingView: View + 'static,
    LoadingViewBuilder: Fn() -> LoadingView,
    ErrorView: View + 'static,
    ErrorViewBuilder: Fn(BoxError) -> ErrorView,
{
    fn view(&self) -> BoxView {
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

fn default_error_view(error: BoxError) -> impl View {
    text(format!("Error :{error}"))
}
