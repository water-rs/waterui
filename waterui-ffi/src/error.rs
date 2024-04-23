use core::{fmt::Display, ops::Deref};

use alloc::boxed::Box;
use waterui_view::error::StdError;

use crate::{AnyView, IntoFFI, IntoRust};

use super::Utf8Data;
#[repr(C)]
#[derive(Debug)]
pub struct Error {
    msg: Utf8Data,
}

impl Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.msg.deref())
    }
}

impl StdError for Error {}

#[no_mangle]
extern "C" fn waterui_error(msg: Utf8Data) -> Error {
    Error { msg }
}

ffi_opaque!(ErrorViewBuilder, waterui_view::error::ErrorViewBuilder, 2);
ffi_opaque!(
    OnceErrorViewBuilder,
    waterui_view::error::OnceErrorViewBuilder,
    2
);

#[no_mangle]
unsafe extern "C" fn waterui_build_error_view(
    error: Error,
    builder: *const ErrorViewBuilder,
) -> AnyView {
    (*builder)(Box::new(error)).into_ffi()
}

#[no_mangle]
unsafe extern "C" fn waterui_build_once_error_view(
    error: Error,
    builder: OnceErrorViewBuilder,
) -> AnyView {
    (builder.into_rust())(Box::new(error)).into_ffi()
}
