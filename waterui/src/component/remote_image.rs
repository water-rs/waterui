use core::fmt::Debug;

use url::Url;
use waterui_reactive::{Compute, Computed};

use crate::{View, ViewExt};

use super::AnyView;

#[derive(Debug)]
#[non_exhaustive]
pub struct RemoteImage {
    pub _url: Computed<Url>,
    pub _loading: AnyView,
    pub _error: AnyView,
}

raw_view!(RemoteImage); // it would be implemented in the futre, but now we define it as a raw view.

impl RemoteImage {
    pub fn new(url: impl Compute<Output = Url>) -> Self {
        Self {
            _url: url.computed(),
            _loading: ().anyview(),
            _error: ().anyview(),
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

    pub fn error(mut self, view: impl View + 'static) -> Self {
        self._error = view.anyview();
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
