use crate::{AnyView, View};
use alloc::boxed::Box;
use core::fmt::{Debug, Display};

pub use anyhow::Error;

impl View for Error {
    fn body(self, env: crate::Environment) -> impl View {
        env.get::<DefaultErrorView>().unwrap().spawn(self)
    }
}

pub trait ResultExt<T, E> {
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, V>
    where
        Self: Sized;
}

impl<T, E: Debug + Display + 'static> ResultExt<T, E> for Result<T, E> {
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, V>
    where
        Self: Sized,
    {
        self.map_err(view)
    }
}

pub struct DefaultErrorView {
    pub builder: Box<dyn Fn(Error) -> AnyView>,
}

impl DefaultErrorView {
    pub fn new<V: View + 'static>(builder: impl 'static + Fn(Error) -> V) -> Self {
        Self {
            builder: Box::new(move |error| AnyView::new(builder(error))),
        }
    }

    pub fn spawn(&self, error: Error) -> AnyView {
        (self.builder)(error)
    }
}
