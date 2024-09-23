use crate::{AnyView, Environment, View};
use alloc::boxed::Box;
use core::{
    any::TypeId,
    fmt::{Debug, Display},
    ops::Deref,
};

pub use core::error::Error as StdError;

pub struct Error {
    inner: Box<dyn ErrorImpl>,
}

pub type BoxedStdError = Box<dyn StdError>;
pub type ErrorViewBuilder = Box<dyn Fn(BoxedStdError) -> AnyView>;

trait ErrorImpl: Debug + Display + 'static {
    fn body(self: Box<Self>, _env: Environment) -> AnyView;

    fn type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

impl<E: StdError + 'static> ErrorImpl for E {
    fn body(self: Box<Self>, _env: Environment) -> AnyView {
        AnyView::new(UseDefaultErrorView::new(self))
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

pub struct ErrorView(AnyView);

impl ErrorView {
    fn new(view: impl View) -> Self {
        Self(AnyView::new(view))
    }
}

impl Display for ErrorView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_fmt(format_args!("ErrorView<{}>", self.0.name()))
    }
}

impl Debug for ErrorView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        Debug::fmt(&self, f)
    }
}

impl ErrorImpl for ErrorView {
    fn body(self: Box<Self>, _env: Environment) -> AnyView {
        self.0
    }
}

impl View for Error {
    fn body(self, env: Environment) -> impl View {
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

pub struct UseDefaultErrorView(BoxedStdError);

impl From<BoxedStdError> for UseDefaultErrorView {
    fn from(value: BoxedStdError) -> Self {
        Self(value)
    }
}

impl UseDefaultErrorView {
    pub fn new(error: impl StdError + 'static) -> Self {
        let boxed: BoxedStdError = Box::new(error);
        Self::from(boxed)
    }
}

impl View for UseDefaultErrorView {
    fn body(self, env: Environment) -> impl View {
        if let Some(builder) = env.try_get::<DefaultErrorView>() {
            builder.build(self.0)
        } else {
            AnyView::new(())
        }
    }
}

pub struct DefaultErrorView(ErrorViewBuilder);

impl DefaultErrorView {
    pub fn new<V: View>(builder: impl 'static + Fn(BoxedStdError) -> V) -> Self {
        Self(Box::new(move |error| AnyView::new(builder(error))))
    }

    pub fn build(&self, error: BoxedStdError) -> AnyView {
        (self.0)(error)
    }
}
