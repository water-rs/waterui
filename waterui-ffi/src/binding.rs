use crate::{array::waterui_str, closure::waterui_closure, IntoFFI, IntoRust};
use alloc::{borrow::Cow, boxed::Box};
use core::ptr::drop_in_place;
use waterui::{Binding, Compute};
use waterui_reactive::Reactive;
// WARNING: Binding<T> must be called on the Rust thread!!!

ffi_type!(waterui_binding_str, Binding<Cow<'static, str>>);
ffi_type!(waterui_binding_int, Binding<i32>);
ffi_type!(waterui_binding_bool, Binding<bool>);

macro_rules! impl_binding {
    ($binding:ty,$ffi:ty,$read:ident,$write:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(binding: *const $binding) -> $ffi {
            (*binding).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $write(binding: *const $binding, value: $ffi) {
            (*binding).set(value.into_rust());
        }

        #[no_mangle]
        pub unsafe extern "C" fn $subscribe(
            binding: *const $binding,
            subscriber: waterui_closure,
        ) -> isize {
            (*binding)
                .register_subscriber(Box::new(move || subscriber.call()))
                .map(|v| v.get() as isize)
                .unwrap_or(-1)
        }

        #[no_mangle]
        pub unsafe extern "C" fn $unsubscribe(binding: *const $binding, id: usize) {
            let id = core::num::NonZeroUsize::new(id).unwrap();
            (*binding).cancel_subscriber(id);
        }

        #[no_mangle]
        pub unsafe extern "C" fn $drop(binding: *mut $binding) {
            drop_in_place(binding);
        }
    };
}

impl_binding!(
    waterui_binding_str,
    waterui_str,
    waterui_read_binding_str,
    waterui_write_binding_str,
    waterui_subscribe_binding_str,
    waterui_unsubscribe_binding_str,
    waterui_drop_binding_str
);

impl_binding!(
    waterui_binding_int,
    i32,
    waterui_read_binding_int,
    waterui_write_binding_int,
    waterui_subscribe_binding_int,
    waterui_unsubscribe_binding_int,
    waterui_drop_binding_int
);

impl_binding!(
    waterui_binding_bool,
    bool,
    waterui_read_binding_bool,
    waterui_write_binding_bool,
    waterui_subscribe_binding_bool,
    waterui_unsubscribe_binding_bool,
    waterui_drop_binding_bool
);
