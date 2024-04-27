use crate::{array::waterui_str, IntoFFI};
use core::{num::NonZeroUsize, ptr::drop_in_place};

use crate::closure::waterui_closure;
use alloc::boxed::Box;
use waterui::{Compute, Computed, CowStr, Reactive};

ffi_type!(waterui_computed_str, Computed<CowStr>);
ffi_type!(waterui_computed_int, Computed<i32>);
ffi_type!(waterui_computed_bool, Computed<bool>);

// WARNING: Computed<T> must be called on the Rust thread!!!
macro_rules! impl_computed {
    ($computed:ty,$ffi:ty,$read:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(computed: *const $computed) -> $ffi {
            (*computed).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $subscribe(
            computed: *const $computed,
            subscriber: waterui_closure,
        ) -> isize {
            (*computed)
                .register_subscriber(Box::new(move || subscriber.call()))
                .map(|v| v.get() as isize)
                .unwrap_or(-1)
        }

        #[no_mangle]
        pub unsafe extern "C" fn $unsubscribe(computed: *const $computed, id: usize) {
            if let Some(id) = NonZeroUsize::new(id) {
                (*computed).cancel_subscriber(id);
            }
        }

        #[no_mangle]
        pub unsafe extern "C" fn $drop(computed: *mut $computed) {
            drop_in_place(computed);
        }
    };
}

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
    i32,
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
