use crate::{AnyView, Environment, View};
use alloc::boxed::Box;
use core::{
    any::TypeId,
    fmt::{Debug, Display},
    ops::Deref,
};

pub use std::error::Error as StdError;
pub struct Error {
    inner: Box<dyn ErrorImpl>,
}

pub type BoxedStdError = Box<dyn StdError>;
pub type ErrorViewBuilder = Box<dyn Fn(BoxedStdError) -> AnyView>;

trait ErrorImpl: Debug + Display + 'static {
    fn body<'a>(self: Box<Self>, _env: &'a Environment) -> AnyView
    where
        Self: 'a;
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

impl<E: StdError + 'static> ErrorImpl for E {
    fn body<'a>(self: Box<Self>, env: &'a Environment) -> AnyView
    where
        Self: 'a,
    {
        env.default_error_view(self)
    }
}

impl Error {
    pub fn new(error: impl StdError + 'static) -> Self {
        Self {
            inner: Box::new(error),
        }
    }

    pub fn downcast<T: 'static>(self) -> Result<Box<T>, Self> {
        if ErrorImpl::type_id(self.inner.deref()) == TypeId::of::<T>() {
            unsafe { Ok(Box::from_raw(Box::into_raw(self.inner) as *mut T)) }
        } else {
            Err(self)
        }
    }

    pub fn from_view(view: impl View) -> Self {
        Self {
            inner: Box::new(ErrorView::new(view)),
        }
    }
}

pub struct ErrorView {
    view: AnyView,
}

impl ErrorView {
    fn new(view: impl View) -> Self {
        Self {
            view: AnyView::new(view),
        }
    }
}

impl Display for ErrorView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("ErrorView<{}>", self.view.name()))
    }
}

impl Debug for ErrorView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Debug::fmt(&self, f)
    }
}
impl ErrorImpl for ErrorView {
    fn body<'a>(self: Box<Self>, _env: &'a Environment) -> AnyView
    where
        Self: 'a,
    {
        self.view
    }
}

impl View for Error {
    fn body(self, env: &Environment) -> impl View {
        self.inner.body(env)
    }
}

pub trait ResultExt<T, E> {
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, Error>
    where
        Self: Sized;
}

impl<T, E: Debug + Display + 'static> ResultExt<T, E> for Result<T, E> {
    fn error_view<V: View>(self, view: impl FnOnce(E) -> V) -> Result<T, Error>
    where
        Self: Sized,
    {
        self.map_err(|error| Error::from_view(view(error)))
    }
}

pub struct DefaultErrorView {
    pub builder: ErrorViewBuilder,
}

pub struct UseDefaultErrorView;

impl DefaultErrorView {
    pub fn new<V: View>(builder: impl 'static + Fn(BoxedStdError) -> V) -> Self {
        Self {
            builder: Box::new(move |error| AnyView::new(builder(error))),
        }
    }

    pub fn spawn(&self, error: BoxedStdError) -> AnyView {
        (self.builder)(error)
    }
}
