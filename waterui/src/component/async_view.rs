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
pub struct AsyncView<MainView, LoadingViewBuilder, ErrorViewBuilder> {
    view: Binding<AsyncViewState<MainView>>,
    loading_view: LoadingViewBuilder,
    error_view: ErrorViewBuilder,
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

#[derive(Debug, Clone)]
#[view(use_core)]
struct LoadingPage;

#[view(use_core)]
impl View for LoadingPage {
    fn view(self) -> impl View {
        text("Loading...")
    }
}

impl<MainView> AsyncView<MainView, DefaultLoadingPageBuilder, ErrorPageBuilder>
where
    MainView: View + 'static,
{
    pub fn new<F, Fut>(f: F) -> Self
    where
        F: 'static + Fn() -> Fut,
        Fut: Future<Output = Result<MainView, BoxError>>,
    {
        let binding = Binding::new(AsyncViewState::Initial);
        Self {
            view: binding.clone(),
            loading_view: DefaultLoadingPageBuilder,
            error_view: ErrorPageBuilder,
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

impl<MainView, LoadingViewBuilder, ErrorViewBuilder>
    AsyncView<MainView, LoadingViewBuilder, ErrorViewBuilder>
where
    MainView: View + 'static,
    LoadingViewBuilder: ViewBuilder,
    ErrorViewBuilder: for<'a> ViewBuilder<(BoxError, &'a dyn Fn())>,
{
    pub fn loading_view<Builder: ViewBuilder>(
        self,
        builder: Builder,
    ) -> AsyncView<MainView, Builder, ErrorViewBuilder> {
        AsyncView {
            view: self.view,
            loading_view: builder,
            error_view: self.error_view,
            retry: self.retry,
        }
    }

    pub fn error_view<Builder: ViewBuilder>(
        self,
        builder: Builder,
    ) -> AsyncView<MainView, LoadingViewBuilder, Builder> {
        AsyncView {
            view: self.view,
            loading_view: self.loading_view,
            error_view: builder,
            retry: self.retry,
        }
    }
}

struct DefaultLoadingPageBuilder;

impl ViewBuilder for DefaultLoadingPageBuilder {
    type Output = LoadingPage;
    fn build(&self, _context: ()) -> Self::Output {
        LoadingPage
    }
}

struct ErrorPageBuilder;

impl ViewBuilder<(BoxError, &dyn Fn())> for ErrorPageBuilder {
    type Output = ErrorPage;
    fn build(&self, context: (BoxError, &dyn Fn())) -> Self::Output {
        ErrorPage::new(context.0)
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
    fn view(self) -> Stack {
        vstack((text("Oop! Something is wrong"), text(self.message)))
    }
}

impl<MainView, LoadingViewBuilder, ErrorViewBuilder> View
    for AsyncView<MainView, LoadingViewBuilder, ErrorViewBuilder>
where
    MainView: View + 'static,

    LoadingViewBuilder: ViewBuilder,
    ErrorViewBuilder: for<'a> ViewBuilder<(Box<dyn StdError>, &'a dyn Fn())>,
{
    fn view(self) -> BoxView {
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
