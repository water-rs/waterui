use crate::utils::task;
use crate::view;
use std::fmt::Display;
use std::future::Future;
use std::mem::take;
use std::ops::DerefMut;
use waterui_core::binding::Binding;
use waterui_core::view::{BoxView, ViewBuilder};

use crate::view::{View, ViewExt};

use std::error::Error as StdError;

use super::stack::vstack;
use super::{text, Stack};
type BoxError = Box<dyn StdError>;
#[view(use_core)]
pub struct AsyncView<MainView, LoadingView, ErrorView> {
    view: Binding<AsyncViewState<MainView>>,
    loading_view: Box<dyn ViewBuilder<LoadingView, ()>>,
    error_view: BoxErrorViewBuilder<ErrorView>,
    retry: Box<dyn Fn()>,
}

enum AsyncViewState<V> {
    Initial,
    Loading,
    Ready(V),
    Fail(BoxError),
}

impl<V> Default for AsyncViewState<V> {
    fn default() -> Self {
        Self::Initial
    }
}

type BoxErrorViewBuilder<V> = Box<dyn for<'a> ViewBuilder<V, (BoxError, &'a dyn Fn())>>;

#[derive(Debug, Clone)]
#[view(use_core)]
struct LoadingPage;

#[view(use_core)]
impl View for LoadingPage {
    fn view(&self) -> impl View {
        text("Loading...")
    }
}

impl<MainView> AsyncView<MainView, LoadingPage, ErrorPage>
where
    MainView: View + 'static,
{
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: 'static + Fn() -> Fut,
        Fut: Future<Output = Result<MainView, BoxError>> + 'static,
    {
        let binding = Binding::new(AsyncViewState::Initial);
        Self {
            view: binding.clone(),
            loading_view: Box::new(|| LoadingPage),
            error_view: Box::new(|error, _retry: &dyn Fn()| ErrorPage::new(error)),
            retry: Box::new(move || {
                task(async {
                    binding.set(AsyncViewState::Loading);
                    let result = f().await;
                    match result {
                        Ok(view) => binding.set(AsyncViewState::Ready(view)),
                        Err(error) => binding.set(AsyncViewState::Fail(error)),
                    }
                })
            }),
        }
    }
}

#[view(use_core)]
struct ErrorPage {
    message: String,
}

impl ErrorPage {
    pub fn new(message: impl Display) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

#[view(use_core)]
impl View for ErrorPage {
    fn view(&self) -> Stack {
        vstack((text("Oop! Something is wrong"), text(&self.message)))
    }
}

#[view(use_core)]
impl<MainView: View + 'static, LoadingView: View + 'static, ErrorView: View + 'static> View
    for AsyncView<MainView, LoadingView, ErrorView>
{
    fn view(&self) -> BoxView {
        match take(self.view.get_mut().deref_mut()) {
            AsyncViewState::Initial => {
                (self.retry)();
                self.loading_view.build(()).boxed()
            }
            AsyncViewState::Loading => self.loading_view.build(()).boxed(),
            AsyncViewState::Ready(view) => view.boxed(),
            AsyncViewState::Fail(error) => self.error_view.build((error, &self.retry)).boxed(),
        }
    }
}
