use std::mem::{size_of, transmute};

use crate::{
    component::AnyView,
    utils::{self, Color},
};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ViewObject {
    inner: [u8; size_of::<AnyView>()], // *const dyn AnyViewTrait
}

impl From<AnyView> for ViewObject {
    fn from(value: AnyView) -> Self {
        unsafe { transmute(value) }
    }
}

impl ViewObject {
    /// # Safety
    /// `EventObject` must be valid
    pub unsafe fn into_anyview(&self) -> AnyView {
        transmute(self.inner)
    }
}

#[repr(C)]
pub struct EventObject {
    inner: [usize; 2], // *const dyn Fn()
}

impl EventObject {
    /// # Safety
    /// `EventObject` must be valid
    pub unsafe fn as_ref(&self) -> &(dyn Fn() + 'static) {
        transmute(self.inner)
    }
}

impl From<Box<dyn Fn() + Send + Sync + 'static>> for EventObject {
    fn from(value: Box<dyn Fn() + Send + Sync + 'static>) -> Self {
        unsafe { transmute(value) }
    }
}

#[derive(Debug)]
#[repr(C)]
pub enum Background {
    Default,
    Color(Color),
}

impl From<utils::Background> for Background {
    fn from(value: utils::Background) -> Self {
        match value {
            utils::Background::Default => Background::Default,
            utils::Background::Color(color) => Background::Color(color),
        }
    }
}
