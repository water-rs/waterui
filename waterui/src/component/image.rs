use alloc::vec::Vec;

#[derive(Clone, PartialEq)]
#[non_exhaustive]
pub struct Image {
    pub _data: Vec<u8>,
}

impl_debug!(Image);

impl Image {
    pub fn new(data: Vec<u8>) -> Self {
        Self { _data: data }
    }

    pub fn data(&self) -> &[u8] {
        &self._data
    }
}

raw_view!(Image);
