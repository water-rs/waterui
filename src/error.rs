use waterui_core::{AnyView, Environment, View};

use crate::ViewExt;

pub struct ErrorView {
    inner: BoxedError,
}

impl<E> From<E> for ErrorView
where
    E: std::error::Error + 'static,
{
    fn from(error: E) -> Self {
        Self {
            inner: Box::new(error),
        }
    }
}

type BoxedError = Box<dyn core::error::Error>;
#[allow(clippy::type_complexity)]
pub struct ErrorViewBuilder(Box<dyn Fn(BoxedError, &Environment) -> AnyView>);

impl View for ErrorView {
    fn body(self, env: &waterui_core::Environment) -> impl View {
        if let Some(builder) = env.get::<ErrorViewBuilder>() {
            (builder.0)(self.inner, env)
        } else {
            format!("{}", self.inner).anyview()
        }
    }
}

impl ErrorViewBuilder {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(BoxedError, &Environment) -> AnyView + 'static,
    {
        Self(Box::new(f))
    }
}
