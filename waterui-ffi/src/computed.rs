use crate::{
    array::{waterui_data, waterui_str},
    IntoFFI, IntoRust,
};
use core::{num::NonZeroUsize, ptr::drop_in_place};

use crate::closure::waterui_closure;
use alloc::{borrow::Cow, boxed::Box, vec::Vec};
use waterui_reactive::{Compute, Computed, Int, Reactive};

pub type waterui_computed<T> = Computed<T>;
type waterui_int = Int;
pub type waterui_computed_str = waterui_computed<Cow<'static, str>>;
pub type waterui_computed_int = waterui_computed<Int>;
pub type waterui_computed_bool = waterui_computed<bool>;
pub type waterui_computed_data = waterui_computed<Vec<u8>>;
impl<T> IntoFFI for Computed<T> {
    type FFI = *mut waterui_computed<T>;
    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(self))
    }
}

impl<T> IntoRust for *mut waterui_computed<T> {
    type Rust = Computed<T>;
    unsafe fn into_rust(self) -> Self::Rust {
        *Box::from_raw(self)
    }
}

// WARNING: Computed<T> must be called on the Rust thread!!!
macro_rules! impl_computed {
    ($name:ty,$ffi:ty,$read:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(computed: *const $name) -> $ffi {
            (*computed).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $subscribe(
            computed: *const $name,
            subscriber: waterui_closure,
        ) -> Int {
            (*computed)
                .register_subscriber(Box::new(move || subscriber.call()))
                .map(|v| v.get() as Int)
                .unwrap_or(-1)
        }

        #[no_mangle]
        pub unsafe extern "C" fn $unsubscribe(computed: *const $name, id: usize) {
            if let Some(id) = NonZeroUsize::new(id) {
                (*computed).cancel_subscriber(id);
            }
        }

        #[no_mangle]
        pub unsafe extern "C" fn $drop(computed: *mut $name) {
            drop_in_place(computed);
        }
    };
}

impl_computed!(
    waterui_computed_data,
    waterui_data,
    waterui_read_computed_data,
    waterui_subscribe_computed_data,
    waterui_unsubscribe_computed_data,
    waterui_drop_computed_data
);

impl_computed!(
    waterui_computed_str,
    waterui_str,
    waterui_read_computed_str,
    waterui_subscribe_computed_str,
    waterui_unsubscribe_computed_str,
    waterui_drop_computed_str
);

impl_computed!(
    waterui_computed_int,
    waterui_int,
    waterui_read_computed_int,
    waterui_subscribe_computed_int,
    waterui_unsubscribe_computed_int,
    waterui_drop_computed_int
);

impl_computed!(
    waterui_computed_bool,
    bool,
    waterui_read_computed_bool,
    waterui_subscribe_computed_bool,
    waterui_unsubscribe_computed_bool,
    waterui_drop_computed_bool
);
