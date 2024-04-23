use super::{Closure, Int, Utf8Data};
use alloc::{boxed::Box, string::String};
use waterui_reactive::Binding;

// WARNING: Binding<T> must be called on the Rust thread!!!

macro_rules! impl_binding {
    ($read:ident,$write:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident,$binding_ty:ident,$ty:ty,$output_ty:ty) => {
        ffi_opaque!(Binding<$ty>, $binding_ty, 1);

        #[no_mangle]
        unsafe extern "C" fn $read(binding: *const $binding_ty) -> $output_ty {
            (*binding).get().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $write(binding: *const $binding_ty, value: $output_ty) {
            (*binding).set(value);
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(binding: *const $binding_ty, subscriber: Closure) -> usize {
            (*binding)
                .register_subscriber(Box::new(move || subscriber.call()))
                .into()
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(binding: *const $binding_ty, id: usize) {
            if let Some(id) = core::num::NonZeroUsize::new(id) {
                (*binding).cancel_subscriber(id);
            }
        }

        #[no_mangle]
        unsafe extern "C" fn $drop(binding: $binding_ty) {
            let _ = binding;
        }
    };
}

impl_binding!(
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    waterui_unsubscribe_binding_str,
    waterui_drop_binding_str,
    BindingStr,
    String,
    Utf8Data
);

impl_binding!(
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    waterui_unsubscribe_binding_int,
    waterui_drop_binding_int,
    BindingInt,
    Int,
    Int
);
