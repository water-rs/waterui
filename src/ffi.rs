use crate::View;
use std::{
    mem::{size_of, transmute},
    ptr::write,
};

#[repr(C)]
pub struct Binding {}

#[repr(C)]
pub struct Text {
    text: Buf,
}

impl Text {
    pub fn new(text: crate::Text) -> Self {
        let text = text.text.into_wrapped();
        Self { text: text.into() }
    }
}

#[repr(C)]
pub struct ViewObject {
    object: [u8; size_of::<*const dyn View>()],
}

#[repr(C)]
pub struct Stack {
    content: Array<ViewObject>,
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
                    write(ptr, <$ffi_type>::new(*text));
                    true
                } else {
                    false
                }
            }
        }
    };
}

view_as!(view_as_text, crate::Text, Text);

#[repr(C)]
struct Array<T> {
    head: *const T,
    len: usize,
}

type Buf = Array<u8>;

impl<T> Array<T> {
    pub fn new(head: *const T, len: usize) -> Self {
        Self { head, len }
    }
}

impl From<Vec<u8>> for Buf {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value.as_ptr(), value.len())
    }
}

impl From<String> for Buf {
    fn from(value: String) -> Self {
        Self::new(value.as_ptr(), value.len())
    }
}
