mod macros;
pub use waterui_ffi::*;

use crate::{AnyView, handler::BoxHandler};

impl OpaqueType for AnyView {}
impl OpaqueType for BoxHandler<()> {}
