use crate::view::ViewExt;
use waterui_view::{AnyView, Environment, View};
pub struct WithValue<T> {
    pub _content: AnyView,
    pub _value: T,
}

impl<T> WithValue<T> {
    pub fn new(content: impl View + 'static, value: T) -> Self {
        Self {
            _content: content.anyview(),
            _value: value,
        }
    }
}

impl<T> View for WithValue<T> {
    fn body(self, _env: Environment) -> impl View {
        panic!(
            "You cannot call `body` for a raw view, may you need to handle this view `{}` manually",
            core::any::type_name::<Self>()
        );
    }
}

#[doc(hidden)]
pub mod ffi {
    use waterui_ffi::{waterui_anyview, IntoFFI};

    #[repr(C)]
    pub struct WithValue<T> {
        content: *mut waterui_anyview,
        value: T,
    }

    impl<T: IntoFFI> IntoFFI for super::WithValue<T> {
        type FFI = WithValue<T::FFI>;
        fn into_ffi(self) -> Self::FFI {
            WithValue {
                content: self._content.into_ffi(),
                value: self._value.into_ffi(),
            }
        }
    }
}
