use crate::{
    array::waterui_str, closure::waterui_fn, computed::waterui_watcher_guard, IntoFFI, IntoRust,
};
use ::waterui_str::Str;
use waterui::ComputeExt;
use waterui_reactive::{Binding, Compute};
// WARNING: Binding<T> must be called on the Rust thread!!!

ffi_type!(waterui_binding_str, Binding<Str>, waterui_drop_binding_str);
ffi_type!(waterui_binding_int, Binding<i32>, waterui_drop_binding_int);
ffi_type!(
    waterui_binding_bool,
    Binding<bool>,
    waterui_drop_binding_bool
);

macro_rules! impl_binding {
    ($binding:ty,$ffi:ty,$read:ident,$set:ident,$watch:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(binding: *const $binding) -> $ffi {
            (*binding).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $set(binding: *const $binding, value: $ffi) {
            (*binding).set(value.into_rust());
        }

        #[no_mangle]
        pub unsafe extern "C" fn $watch(
            binding: *const $binding,
            watcher: waterui_fn<$ffi>,
        ) -> *mut waterui_watcher_guard {
            let guard =
                (*binding).watch(move |v: <$ffi as IntoRust>::Rust| watcher.call(v.into_ffi()));
            guard.into_ffi()
        }
    };
}

impl_binding!(
    waterui_binding_str,
    waterui_str,
    waterui_read_binding_str,
    waterui_set_binding_str,
    waterui_watch_binding_str
);

impl_binding!(
    waterui_binding_int,
    i32,
    waterui_read_binding_int,
    waterui_set_binding_int,
    waterui_watch_binding_int
);

impl_binding!(
    waterui_binding_bool,
    bool,
    waterui_read_binding_bool,
    waterui_set_binding_bool,
    waterui_watch_binding_bool
);
