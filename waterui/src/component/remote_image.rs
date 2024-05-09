use core::fmt::Debug;

use crate::{Computed, CowStr};
use crate::{View, ViewExt};
use alloc::boxed::Box;
use alloc::format;
use alloc::string::String;
use url::Url;
use waterui_core::error::BoxedStdError;
use waterui_core::raw_view;
use waterui_reactive::compute::IntoComputed;
use waterui_reactive::{Compute, ComputeExt};

use crate::AnyView;

use super::{text, Progress};

#[non_exhaustive]
pub struct RemoteImage {
    pub _url: Computed<CowStr>,
    pub _loading: AnyView,
    pub _error: Box<dyn FnOnce(BoxedStdError) -> AnyView>,
}

raw_view!(RemoteImage); // it would be implemented in the futre, but now we define it as a raw view.

impl RemoteImage {
    pub fn new(url: impl Compute<Output = Url> + 'static) -> Self {
        Self {
            _url: url.map(String::from).into_computed(),
            _loading: Progress::infinity("").anyview(),
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

    pub fn loading(mut self, view: impl View) -> Self {
        self._loading = view.anyview();
        self
    }

    pub fn error<V: View>(mut self, builder: impl 'static + FnOnce(BoxedStdError) -> V) -> Self {
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
