macro_rules! impl_array {
    ($name:ident,$item:ty,$ffi_item:ty) => {
        #[derive(Debug)]
        #[repr(C)]
        pub struct $name {
            head: *mut $ffi_item,
            len: usize,
            capacity: usize,
        }

        impl From<Vec<$item>> for $name {
            fn from(value: Vec<$item>) -> Self {
                use std::mem::ManuallyDrop;
                let value: Vec<$ffi_item> = value.into_iter().map(<$ffi_item>::from).collect();
                let mut value = ManuallyDrop::new(value);
                Self {
                    head: value.as_mut_ptr() as *mut $ffi_item,
                    len: value.len(),
                    capacity: value.capacity(),
                }
            }
        }
    };
}

#[derive(Debug)]
#[repr(C)]
pub struct Buf {
    head: *mut u8,
    len: usize,
    capacity: usize,
}
impl From<Vec<u8>> for Buf {
    fn from(value: Vec<u8>) -> Self {
        use std::mem::ManuallyDrop;

        let mut value = ManuallyDrop::new(value);
        Self {
            head: value.as_mut_ptr(),
            len: value.len(),
            capacity: value.capacity(),
        }
    }
}

impl From<String> for Buf {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}

impl From<Buf> for Vec<u8> {
    fn from(val: Buf) -> Self {
        unsafe { Self::from_raw_parts(val.head, val.len, val.capacity) }
    }
}
