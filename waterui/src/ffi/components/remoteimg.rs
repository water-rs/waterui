use alloc::boxed::Box;
use waterui_view::error::BoxedStdError;

use crate::{
    component,
    ffi::{computed::ComputedStr, error::Error, AnyView},
};

#[repr(C)]
pub struct RemoteImage {
    url: ComputedStr,
    loading: AnyView,
    error: ErrorViewBuilder,
}

impl From<component::RemoteImage> for RemoteImage {
    fn from(value: component::RemoteImage) -> Self {
        Self {
            url: value._url.into(),
            loading: value._loading.into(),
            error: value._error.into(),
        }
    }
}

impl_view!(
    component::RemoteImage,
    RemoteImage,
    waterui_view_force_as_remoteimg,
    waterui_view_remoteimg_id
);

ffi_opaque!(
    Box<dyn FnOnce(BoxedStdError) -> crate::AnyView>,
    ErrorViewBuilder,
    2
);

#[no_mangle]
unsafe extern "C" fn waterui_build_error_view(error: Error, builder: ErrorViewBuilder) -> AnyView {
    (builder.into_ty())(Box::new(error)).into()
}
