use alloc::{boxed::Box, vec::Vec};

use waterui_reactive::Compute;
use waterui_str::Str;

use crate::component::Image;

pub trait Transfer {
    fn transfer(&self) -> TransferData;
}

impl Transfer for Str {
    fn transfer(&self) -> TransferData {
        TransferData::Text(self.clone())
    }
}

impl Transfer for Image {
    fn transfer(&self) -> TransferData {
        TransferData::Photo(self.data().compute())
    }
}

impl Transfer for TransferData {
    fn transfer(&self) -> TransferData {
        self.clone()
    }
}

impl<F, T: Transfer> Transfer for F
where
    F: Fn() -> T,
{
    fn transfer(&self) -> TransferData {
        (self)().transfer()
    }
}

#[derive(Debug, Clone)]
pub enum TransferData {
    Text(Str),
    File { url: Str },
    Photo(Vec<u8>),
}

pub struct Draggble(pub Box<dyn Transfer>);
impl_debug!(Draggble);

pub struct Copyable(pub Box<dyn Transfer>);

impl_debug!(Copyable);

impl Draggble {
    pub fn new(data: impl Transfer + 'static) -> Self {
        Self(Box::new(data))
    }
}

impl Copyable {
    pub fn new(data: impl Transfer + 'static) -> Self {
        Self(Box::new(data))
    }
}
