use crate::{View, ViewExt};

use super::TypeId;

mod anyview;
pub use anyview::AnyView;
mod button;
mod field;
mod stack;
mod text;

#[no_mangle]
unsafe extern "C" fn waterui_view_id(view: *const AnyView) -> TypeId {
    (*view).type_id().into()
}

#[no_mangle]
unsafe extern "C" fn waterui_call_view(view: AnyView, env: crate::ffi::Environment) -> AnyView {
    let view = crate::AnyView::from(view);
    view.body(env.into_ty()).anyview().into()
}

#[no_mangle]
unsafe extern "C" fn waterui_view_empty_id() -> super::TypeId {
    core::any::TypeId::of::<()>().into()
}
