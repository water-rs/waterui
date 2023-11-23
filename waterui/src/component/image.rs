use crate::{utils::Resource, view::Frame};

#[derive(Debug)]
pub struct Image {
    frame: Frame,
    pub resource: Resource,
}

impl Image {
    pub fn new(resource: impl Into<Resource>) -> Self {
        Self {
            frame: Frame::default(),
            resource: resource.into(),
        }
    }
}

native_implement_with_frame!(Image);
