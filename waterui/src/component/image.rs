use alloc::boxed::Box;

use alloc::vec::Vec;
use waterui_core::AnyView;
use waterui_reactive::{compute::IntoComputed, Computed};
use waterui_str::Str;

pub type Data = Vec<u8>;

#[derive(Debug)]
pub struct ImageConfig {
    pub data: Computed<Data>,
}

configurable!(Image, ImageConfig);

impl Image {
    pub fn new(data: impl IntoComputed<Data>) -> Self {
        Self(ImageConfig {
            data: data.into_computed(),
        })
    }

    pub fn data(&self) -> Computed<Data> {
        self.0.data.clone()
    }
}

#[cfg(feature = "std")]
mod std_on {
    #[cfg(feature = "std")]
    extern crate std;
    use super::Image;
    use async_fs::read;
    use std::{io, path::Path};

    impl Image {
        pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
            Ok(Self::new(read(path).await?))
        }
    }
}

pub struct RemoteImageConfig {
    pub url: Computed<Str>,
    pub placeholder: AnyView,
    pub callback: Box<dyn FnOnce(bool)>,
}

configurable!(RemoteImage, RemoteImageConfig);

impl RemoteImage {}

impl_debug!(RemoteImageConfig);
