use core::fmt::Debug;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use url::Url;
use waterui_reactive::compute::ComputedStr;
use waterui_reactive::{Compute, ComputeExt};
use waterui_view::error::{BoxedStdError, OnceErrorViewBuilder};

use crate::{View, ViewExt};

use crate::AnyView;

use super::{text, Progress};

#[non_exhaustive]
pub struct RemoteImage {
    pub _url: ComputedStr,
    pub _loading: AnyView,
    pub _error: OnceErrorViewBuilder,
}

raw_view!(RemoteImage); // it would be implemented in the futre, but now we define it as a raw view.

impl RemoteImage {
    pub fn new(url: impl Compute<Output = Url> + Clone) -> Self {
        Self {
            _url: url.transform(|u| String::from(u).into()).computed(),
            _loading: Progress::infinity().anyview(),
            _error: Box::new(|error| {
                if cfg!(debug_assertions) {
                    text(format!("Error: {error}")).anyview()
                } else {
                    text("Error").anyview()
                }
            }),
        }
    }

    pub fn url<U>(url: U) -> Self
    where
        U: TryInto<Url>,
        U::Error: Debug,
    {
        Self::new(url.try_into().unwrap())
    }

    pub fn loading(mut self, view: impl View + 'static) -> Self {
        self._loading = view.anyview();
        self
    }

    pub fn error<V: View + 'static>(
        mut self,
        builder: impl 'static + FnOnce(BoxedStdError) -> V,
    ) -> Self {
        self._error = Box::new(move |error| builder(error).anyview());
        self
    }
}

pub fn remoteimg<U>(url: U) -> RemoteImage
where
    U: TryInto<Url>,
    U::Error: Debug,
{
    RemoteImage::url(url)
}

mod ffi {
    use waterui_ffi::{
        computed::ComputedStr, error::OnceErrorViewBuilder, ffi_view, AnyView, IntoFFI,
    };

    #[repr(C)]
    pub struct RemoteImage {
        url: ComputedStr,
        loading: AnyView,
        error: OnceErrorViewBuilder,
    }

    impl IntoFFI for super::RemoteImage {
        type FFI = RemoteImage;
        fn into_ffi(self) -> Self::FFI {
            RemoteImage {
                url: self._url.into_ffi(),
                loading: self._loading.into_ffi(),
                error: self._error.into_ffi(),
            }
        }
    }

    ffi_view!(
        super::RemoteImage,
        RemoteImage,
        waterui_view_force_as_remoteimg,
        waterui_view_remoteimg_id
    );
}
