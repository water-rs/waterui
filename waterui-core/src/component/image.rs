use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RawImage {
    pub(crate) data: Vec<u8>,
}

impl RawImage {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

raw_view!(RawImage);
