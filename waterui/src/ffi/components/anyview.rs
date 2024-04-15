use crate::component;
use crate::View;
ffi_opaque!(component::AnyView, AnyView, 2);

impl AnyView {
    pub fn new(view: impl View + 'static) -> Self {
        Self::from(component::AnyView::new(view))
    }
}

#[no_mangle]
unsafe extern "C" fn waterui_view_force_as_anyview(view: AnyView) -> AnyView {
    let view: component::AnyView = view.into();
    (*view.downcast_unchecked::<component::AnyView>()).into()
}

#[no_mangle]
unsafe extern "C" fn waterui_view_anyview_id() -> super::TypeId {
    std::any::TypeId::of::<component::AnyView>().into()
}
