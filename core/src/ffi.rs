pub use waterui_ffi::*;

use crate::{AnyView, handler::BoxHandler};

impl OpaqueType for AnyView {}
impl OpaqueType for BoxHandler<()> {}

pub use crate::color::ffi::*;
