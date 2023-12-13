macro_rules! impl_array {
    ($name:ident,$item:ty,$ffi_item:ty) => {
        #[derive(Debug)]
        #[repr(C)]
        pub struct $name {
            head: *mut $ffi_item,
            len: usize,
        }

        impl From<Vec<$item>> for $name {
            fn from(value: Vec<$item>) -> Self {
                let value: Vec<$ffi_item> = value.into_iter().map(<$ffi_item>::from).collect();
                let len = value.len();
                let boxed = value.into_boxed_slice();
                Self {
                    head: Box::into_raw(boxed) as *mut $ffi_item,
                    len,
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
}
impl From<Vec<u8>> for Buf {
    fn from(value: Vec<u8>) -> Self {
        let len = value.len();
        let boxed = value.into_boxed_slice();
        Self {
            head: Box::into_raw(boxed) as *mut u8,
            len,
        }
    }
}

impl From<String> for Buf {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}
