use core::fmt::Debug;

use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use url::Url;
use waterui_reactive::{Compute, ComputeExt, Computed};
use waterui_view::error::BoxedStdError;

use crate::{View, ViewExt};

use crate::AnyView;

use super::text;

#[non_exhaustive]
pub struct RemoteImage {
    pub _url: Computed<String>,
    pub _loading: AnyView,
    pub _error: Box<dyn FnOnce(BoxedStdError) -> AnyView>,
}

raw_view!(RemoteImage); // it would be implemented in the futre, but now we define it as a raw view.

impl RemoteImage {
    pub fn new(url: impl Compute<Output = Url> + Clone) -> Self {
        Self {
            _url: url.transform(Into::into).computed(),
            _loading: text("Loading").anyview(),
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
