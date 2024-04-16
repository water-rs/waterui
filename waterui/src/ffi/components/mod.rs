use core::mem::ManuallyDrop;

use crate::{Environment, View, ViewExt};

use super::TypeId;

mod anyview;
pub use anyview::AnyView;
mod button;
mod field;
mod stack;
mod text;

#[no_mangle]
unsafe extern "C" fn waterui_view_id(view: AnyView) -> TypeId {
    let view = ManuallyDrop::new(crate::component::AnyView::from(view));
    view.inner_type_id().into()
}

#[no_mangle]
unsafe extern "C" fn waterui_call_view(view: AnyView) -> AnyView {
    let view = crate::component::AnyView::from(view);
    view.body(Environment::builder().build()).anyview().into()
}
