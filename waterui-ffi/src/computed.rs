use crate::{Data, Utf8Data};

use super::Closure;
use alloc::{string::String, vec::Vec};
use waterui_reactive::{Compute, Computed};

// WARNING: Computed<T> must be called on the Rust thread!!!

macro_rules! impl_computed {
    ($name:ident,$ty:ty,$ffi:ty,$read:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident) => {
        ffi_opaque!($name, Computed<$ty>, 2, $drop);

        #[no_mangle]
        unsafe extern "C" fn $read(computed: *const $name) -> $ffi {
            $crate::IntoFFI::into_ffi((*computed).compute())
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(computed: *const $name, subscriber: Closure) -> usize {
            (*computed)
                .register_subscriber(alloc::boxed::Box::new(move || subscriber.call()))
                .map(Into::into)
                .unwrap_or(0)
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(computed: *const $name, id: usize) {
            if let Some(id) = core::num::NonZeroUsize::new(id) {
                (*computed).cancel_subscriber(id);
            }
        }
    };
}

impl_computed!(
    ComputedData,
    Vec<u8>,
    Data,
    waterui_read_computed_data,
    waterui_subscribe_computed_data,
    waterui_unsubscribe_computed_data,
    waterui_drop_computed_data
);

impl_computed!(
    ComputedStr,
    String,
    Utf8Data,
    waterui_read_computed_str,
    waterui_subscribe_computed_str,
    waterui_unsubscribe_computed_str,
    waterui_drop_computed_str
);

impl_computed!(
    ComputedInt,
    isize,
    isize,
    waterui_read_computed_int,
    waterui_subscribe_computed_int,
    waterui_unsubscribe_computed_int,
    waterui_drop_computed_int
);

impl_computed!(
    ComputedBool,
    bool,
    bool,
    waterui_read_computed_bool,
    waterui_subscribe_computed_bool,
    waterui_unsubscribe_computed_bool,
    waterui_drop_computed_bool
);

impl_computed!(
    ComputedView,
    waterui_view::AnyView,
    crate::AnyView,
    waterui_read_computed_view,
    waterui_subscribe_computed_view,
    waterui_unsubscribe_computed_view,
    waterui_drop_computed_view
);

ffi_view!(
    Computed<waterui_view::AnyView>,
    ComputedView,
    waterui_view_force_as_computed,
    waterui_view_computed_id
);
