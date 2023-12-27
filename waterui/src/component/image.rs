#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub(crate) data: Vec<u8>,
}

impl Image {
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }
}

raw_view!(Image);
