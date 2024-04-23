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

trait ErrorImpl: Debug + Display + 'static {
    fn body(self: Box<Self>, _env: Environment) -> AnyView;
    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

impl<E: StdError + 'static> ErrorImpl for E {
    fn body(self: Box<Self>, env: Environment) -> AnyView {
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

    pub fn from_view(view: impl View + 'static) -> Self {
        Self {
            inner: Box::new(ErrorView::new(view)),
        }
    }
}

pub struct ErrorView {
    view: AnyView,
}

impl ErrorView {
    fn new(view: impl View + 'static) -> Self {
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
    fn body(self: Box<Self>, _env: Environment) -> AnyView {
        self.view
    }
}

impl View for Error {
    fn body(self, env: crate::Environment) -> impl View {
        self.inner.body(env)
    }
}

pub trait ResultExt<T, E> {
    fn error_view<V: View + 'static>(self, view: impl FnOnce(E) -> V) -> Result<T, Error>
    where
        Self: Sized;
}

impl<T, E: Debug + Display + 'static> ResultExt<T, E> for Result<T, E> {
    fn error_view<V: View + 'static>(self, view: impl FnOnce(E) -> V) -> Result<T, Error>
    where
        Self: Sized,
    {
        self.map_err(|error| Error::from_view(view(error)))
    }
}

pub struct DefaultErrorView {
    pub builder: Box<dyn Fn(BoxedStdError) -> AnyView>,
}

pub struct UseDefaultErrorView;

impl DefaultErrorView {
    pub fn new<V: View + 'static>(builder: impl 'static + Fn(BoxedStdError) -> V) -> Self {
        Self {
            builder: Box::new(move |error| AnyView::new(builder(error))),
        }
    }

    pub fn spawn(&self, error: BoxedStdError) -> AnyView {
        (self.builder)(error)
    }
}
