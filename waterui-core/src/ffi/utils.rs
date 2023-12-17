use std::mem::transmute;

use crate::{
    utils::{self, Color},
    BoxView, View,
};

#[repr(C)]
pub struct ViewObject {
    inner: [usize; 2], // Box<dyn View>
}

impl From<BoxView> for ViewObject {
    fn from(value: BoxView) -> Self {
        unsafe { transmute(value) }
    }
}

impl ViewObject {
    /// # Safety
    /// `EventObject` must be valid
    pub unsafe fn into_box(&self) -> BoxView {
        transmute(self.inner)
    }

    pub fn into_ptr(self) -> *const dyn View {
        unsafe { transmute(self) }
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

impl From<Box<dyn Fn() + 'static>> for EventObject {
    fn from(value: Box<dyn Fn() + 'static>) -> Self {
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
