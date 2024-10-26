use alloc::vec::Vec;
use waterui_reactive::{compute::ToComputed, Computed};

pub type Data = Vec<u8>;

#[derive(Debug)]
pub struct ImageConfig {
    pub data: Computed<Data>,
}

configurable!(Image, ImageConfig);

impl Image {
    pub fn new(data: impl ToComputed<Data>) -> Self {
        Self(ImageConfig {
            data: data.to_computed(),
        })
    }
}

#[cfg(feature = "std")]
mod std_on {
    use super::Image;
    use async_fs::read;
    use std::{io, path::Path};

    impl Image {
        pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
            Ok(Self::new(read(path).await?))
        }
    }
}
