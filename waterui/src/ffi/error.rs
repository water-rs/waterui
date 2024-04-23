use core::{fmt::Display, ops::Deref};

use waterui_view::error::StdError;

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
