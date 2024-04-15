#[repr(C)]
pub struct Utf8Data {
    head: *mut u8,
    len: usize,
}

impl_array!(Data, u8, u8);

impl Utf8Data {
    pub fn into_data(self) -> Data {
        Data {
            head: self.head,
            len: self.len,
        }
    }
}

impl From<Utf8Data> for String {
    fn from(val: Utf8Data) -> Self {
        unsafe { String::from_utf8_unchecked(val.into_data().into()) }
    }
}

impl From<String> for Utf8Data {
    fn from(value: String) -> Self {
        let data = Data::from(value.into_bytes());
        Self {
            head: data.head,
            len: data.len,
        }
    }
}

#[repr(C)]
pub struct Subscriber {
    data: *mut (),
    call: unsafe extern "C" fn(*const ()),
    free: unsafe extern "C" fn(*mut ()),
}

unsafe impl Send for Subscriber {}
unsafe impl Sync for Subscriber {}

impl Subscriber {
    pub fn call(&self) {
        unsafe { (self.call)(self.data) }
    }
}

impl Drop for Subscriber {
    fn drop(&mut self) {
        unsafe { (self.free)(self.data) }
    }
}

#[repr(C)]
pub struct TypeId {
    inner: [u64; 2],
    _marker: std::marker::PhantomData<(*const (), std::marker::PhantomPinned)>,
}

impl From<std::any::TypeId> for TypeId {
    fn from(value: std::any::TypeId) -> Self {
        unsafe {
            Self {
                inner: std::mem::transmute(value),
                _marker: std::marker::PhantomData,
            }
        }
    }
}
impl From<TypeId> for std::any::TypeId {
    fn from(value: TypeId) -> Self {
        unsafe { std::mem::transmute(value.inner) }
    }
}

ffi_opaque!(Box<dyn Fn() + Send + Sync>, Action, 2);
