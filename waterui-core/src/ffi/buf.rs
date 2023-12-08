#[derive(Debug)]
#[repr(C)]
pub struct Buf {
    head: *const u8,
    len: usize,
}

impl From<String> for Buf {
    fn from(value: String) -> Self {
        value.into_bytes().into()
    }
}

impl From<Vec<u8>> for Buf {
    fn from(value: Vec<u8>) -> Self {
        let len = value.len();
        let boxed = value.into_boxed_slice();
        Self {
            head: Box::into_raw(boxed) as *const u8,
            len,
        }
    }
}
