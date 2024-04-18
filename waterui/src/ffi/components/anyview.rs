use crate::View;
ffi_opaque!(crate::AnyView, AnyView, 2);

impl AnyView {
    pub fn new(view: impl View + 'static) -> Self {
        Self::from(crate::AnyView::new(view))
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_view_force_as_anyview(view: AnyView) -> AnyView {
    let view: crate::AnyView = view.into();
    (*view.downcast_unchecked::<crate::AnyView>()).into()
}

#[no_mangle]
unsafe extern "C" fn waterui_view_anyview_id() -> super::TypeId {
    core::any::TypeId::of::<crate::AnyView>().into()
}
