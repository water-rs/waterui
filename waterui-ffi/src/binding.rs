use crate::{Closure, Utf8Data};
use alloc::{boxed::Box, string::String};
use waterui_reactive::Binding;

// WARNING: Binding<T> must be called on the Rust thread!!!

macro_rules! impl_binding {
    ($name:ident,$ty:ty,$ffi:ty,$read:ident,$write:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident) => {
        ffi_opaque!($name, Binding<$ty>, 1, $drop);

        #[no_mangle]
        unsafe extern "C" fn $read(binding: *const $name) -> $ffi {
            $crate::IntoFFI::into_ffi((*binding).get())
        }

        #[no_mangle]
        unsafe extern "C" fn $write(binding: *const $name, value: $ffi) {
            let value = $crate::IntoRust::into_rust(value);
            (*binding).set(value);
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(binding: *const $name, subscriber: Closure) -> usize {
            (*binding)
                .register_subscriber(Box::new(move || subscriber.call()))
                .into()
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(binding: *const $name, id: usize) {
            if let Some(id) = core::num::NonZeroUsize::new(id) {
                (*binding).cancel_subscriber(id);
            }
        }
    };
}

impl_binding!(
    BindingStr,
    String,
    Utf8Data,
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    waterui_unsubscribe_binding_str,
    waterui_drop_binding_str
);

impl_binding!(
    BindingInt,
    isize,
    isize,
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    waterui_unsubscribe_binding_int,
    waterui_drop_binding_int
);

impl_binding!(
    BindingBool,
    bool,
    bool,
    waterui_read_binding_bool,
    waterui_write_binding_bool,
    waterui_subscribe_binding_bool,
    waterui_unsubscribe_binding_bool,
    waterui_drop_binding_bool
);
