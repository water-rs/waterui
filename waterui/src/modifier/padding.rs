use crate::layout::Edge;
#[repr(C)]
#[derive(Default)]
pub struct Padding(pub Edge);
impl Padding {
    pub fn new(padding: Edge) -> Self {
        Self(padding)
    }
}

with_modifier!(Padding);
