use crate::component::AnyView;
use crate::View;

ffi_opaque!(crate::component::AnyView, ViewObject, 2);

impl ViewObject {
    pub fn new(view: impl View + 'static) -> Self {
        Self::from(AnyView::new(view))
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_view_force_as_anyview(view: ViewObject) -> ViewObject {
    let view: AnyView = view.into();
    (*view.downcast_unchecked::<AnyView>()).into()
}
#[no_mangle]
unsafe extern "C" fn waterui_view_anyview_id() -> super::TypeId {
    std::any::TypeId::of::<AnyView>().into()
}
