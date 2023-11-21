use crate::{attributed_string, reactive::BoxWatcher, View};
use std::{
    mem::{size_of, transmute},
    ops::Deref,
    ptr::write,
};

use crate::component;

#[repr(C)]
pub struct Text {
    text: AttributedString,
}

impl From<component::Text> for Text {
    fn from(value: component::Text) -> Self {
        let text = value.text.get().deref().clone();
        Self { text: text.into() }
    }
}

#[repr(C)]
pub struct AttributedString {
    text: Buf,
}

impl From<attributed_string::AttributedString> for AttributedString {
    fn from(value: attributed_string::AttributedString) -> Self {
        Self {
            text: value.text.into(),
        }
    }
}

#[repr(C)]
pub struct ViewObject {
    object: [u8; size_of::<*const dyn View>()],
}

#[repr(C)]
pub struct Stack {
    content: ViewObjects,
}

impl From<component::Stack> for Stack {
    fn from(value: component::Stack) -> Self {
        Self {
            content: value.content.into(),
        }
    }
}

impl ViewObject {
    pub fn from_boxed(boxed: Box<(dyn View + 'static)>) -> Self {
        let raw = Box::into_raw(boxed);
        Self {
            object: unsafe { transmute(raw) },
        }
    }

    pub fn into_boxed(self) -> Box<dyn View> {
        unsafe {
            let view: *mut dyn View = transmute(self.object);
            Box::from_raw(view)
        }
    }
}

macro_rules! view_as {
    ($name:ident,$view_type:ty,$ffi_type:ty) => {
        #[no_mangle]
        pub extern "C" fn $name(view: ViewObject, ptr: *mut $ffi_type) -> bool {
            unsafe {
                let view = view.into_boxed();
                if let Ok(text) = view.downcast::<$view_type>() {
                    write(ptr, <$ffi_type>::from(*text));
                    true
                } else {
                    false
                }
            }
        }
    };
}

view_as!(view_as_text, component::Text, Text);
view_as!(view_as_stack, component::Stack, Stack);

macro_rules! impl_buf {
    ($name:ident,$element_ty:ty) => {
        #[repr(C)]
        pub struct $name {
            head: *const $element_ty,
            len: usize,
        }

        impl $name {
            pub fn new(head: *const $element_ty, len: usize) -> Self {
                Self { head, len }
            }
        }

        impl From<Vec<$element_ty>> for $name {
            fn from(value: Vec<$element_ty>) -> Self {
                Self::new(value.as_ptr(), value.len())
            }
        }
    };
}

impl_buf!(Buf, u8);
impl_buf!(ViewObjects, ViewObject);
impl_buf!(Watchers, BoxWatcher);

impl From<String> for Buf {
    fn from(value: String) -> Self {
        Self::new(value.as_ptr(), value.len())
    }
}

impl From<Vec<Box<dyn View>>> for ViewObjects {
    fn from(value: Vec<Box<dyn View>>) -> Self {
        let head = value.as_ptr();
        unsafe {
            let head: *const ViewObject = transmute(head);
            ViewObjects::new(head, value.len())
        }
    }
}
