use serde::{Deserialize, Serialize};

use crate::widget;

use crate::utils::Resource;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[widget]
pub struct Image {
    pub resource: Resource,
}

native_implement!(Image);

impl Image {
    pub fn new(resource: impl Into<Resource>) -> Self {
        Self {
            resource: resource.into(),
        }
    }
}
