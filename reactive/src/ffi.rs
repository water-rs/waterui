use std::ptr::NonNull;

use crate::{Compute, watcher::Metadata};
use waterui_ffi::{Record, buffer::Buffer};

macro_rules! opaque_type {
    ($name:ident,$ty:ty) => {
        #[repr(C)]
        pub struct $name(Box<$ty>);
    };
}

opaque_type!(WuiComputed, Box<dyn WuiComputedImpl>);
opaque_type!(WuiMetadata, Metadata);

#[repr(C)]
pub struct WuiWatcher {
    data: NonNull<()>,
    f: fn(*mut (), Buffer, WuiMetadata),
}

trait WuiComputedImpl {
    fn compute(&self) -> Buffer;
    fn watch(&self, f: WuiWatcher) -> WuiWatcherGuard;
}

impl<C> WuiComputedImpl for C
where
    C: Compute,
    C::Output: Record,
{
    fn compute(&self) -> Buffer {
        todo!()
    }

    fn watch(&self, f: WuiWatcher) {}
}

impl WuiComputed {
    pub fn new<T>(computed: crate::Computed<T>) -> Self {
        todo!()
    }
}

mod test {
    #[waterui_ffi::Opaque]
    struct Metadata {
        // TODO
    }

    #[waterui_ffi::export]
    impl Metadata {
        pub fn get(&self) -> MetadataResult {
            todo!()
        }
    }

    #[waterui_ffi::Record]
    struct MetadataResult {
        // use buffer to encode/decode then transfer it
        value: String,
    }

    // Generate:
    //
    // pub unsafe extern "C" fn wui_metadata_free(metadata: *mut ()) {
    //     drop(Box::from_raw(metadata as *mut Metadata));
    // }
    //
    // pub unsafe extern "C" fn wui_metadata_get(metadata: *const ()) -> MetadataResult{
    //     (&*metadata).get()
    // }
}
