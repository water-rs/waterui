use crate::{array::waterui_str, closure::waterui_fn, IntoFFI};
use alloc::{borrow::Cow, boxed::Box};
use core::{num::NonZeroUsize, ptr::drop_in_place};
use waterui_reactive::{subscriber::SubscriberId, Compute, Computed, Reactive};
ffi_type!(waterui_computed_str, Computed<Cow<'static, str>>);
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
            subscriber: waterui_fn,
        ) -> isize {
            (*computed)
                .register_subscriber(Box::new(move || (subscriber).call()).into())
                .map(|v| v.into_inner() as isize)
                .unwrap_or(-1)
        }

        #[no_mangle]
        pub unsafe extern "C" fn $unsubscribe(computed: *const $computed, id: usize) {
            (*computed).cancel_subscriber(SubscriberId::new(NonZeroUsize::new(id).unwrap()));
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
