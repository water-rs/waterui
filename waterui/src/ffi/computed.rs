use super::{Closure, Data, Int, Utf8Data};
use alloc::{string::String, vec::Vec};
use waterui_reactive::{Compute, Computed};

// WARNING: Computed<T> must be called on the Rust thread!!!

macro_rules! impl_computed {
    ($read:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident,$name:ident,$ty:ty,$ffi_ty:ty) => {
        ffi_opaque!(Computed<$ty>, $name, 2);

        #[no_mangle]
        unsafe extern "C" fn $read(computed: *const $name) -> $ffi_ty {
            (*computed).compute().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(computed: *const $name, subscriber: Closure) -> usize {
            (*computed).register_subscriber(alloc::boxed::Box::new(move || subscriber.call()))
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(computed: *const $name, id: usize) {
            (*computed).cancel_subscriber(id);
        }

        #[no_mangle]
        unsafe extern "C" fn $drop(computed: $name) {
            let _ = computed;
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

impl_computed!(
    waterui_read_computed_bool,
    waterui_subscribe_computed_bool,
    waterui_unsubscribe_computed_bool,
    waterui_drop_computed_bool,
    ComputedBool,
    bool,
    bool
);

impl_computed!(
    waterui_read_computed_view,
    waterui_subscribe_computed_view,
    waterui_unsubscribe_computed_view,
    waterui_drop_computed_view,
    ComputedView,
    crate::component::AnyView,
    crate::ffi::AnyView
);

impl_view!(
    crate::Computed<crate::component::AnyView>,
    ComputedView,
    waterui_view_force_as_dynamic_view,
    waterui_view_dynamic_view_id
);
