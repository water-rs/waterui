use super::{Closure, Data, Int, Utf8Data};
use alloc::{string::String, vec::Vec};
use waterui_reactive::{Compute, Computed};

// WARNING: Computed<T> must be called on the Rust thread!!!

macro_rules! impl_computed {
    ($read:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident,$computed_ty:ident,$ty:ty,$output_ty:ty) => {
        ffi_opaque!(Computed<$ty>, $computed_ty, 2);

        #[no_mangle]
        unsafe extern "C" fn $read(computed: *const $computed_ty) -> $output_ty {
            (*computed).compute().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(
            computed: *const $computed_ty,
            subscriber: Closure,
        ) -> usize {
            (*computed).register_subscriber(alloc::boxed::Box::new(move || subscriber.call()))
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(computed: *const $computed_ty, id: usize) {
            (*computed).cancel_subscriber(id);
        }

        #[no_mangle]
        unsafe extern "C" fn $drop(computed: $computed_ty) {
            let _ = computed.into_ty();
        }
    };
}

impl_computed!(
    waterui_read_computed_data,
    waterui_subscribe_computed_data,
    waterui_unsubscribe_computed_data,
    waterui_drop_computed_data,
    ComputedData,
    Vec<u8>,
    Data
);

impl_computed!(
    waterui_read_computed_str,
    waterui_subscribe_computed_str,
    waterui_unsubscribe_computed_str,
    waterui_drop_computed_str,
    ComputedStr,
    String,
    Utf8Data
);

impl_computed!(
    waterui_read_computed_int,
    waterui_subscribe_computed_int,
    waterui_unsubscribe_computed_int,
    waterui_drop_computed_int,
    ComputedInt,
    Int,
    Int
);
