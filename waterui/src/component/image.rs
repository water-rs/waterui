use crate::widget;

use crate::utils::Resource;

#[derive(Debug)]
#[widget]
pub struct Image {
    pub resource: Resource,
}

native_implement!(Image);
