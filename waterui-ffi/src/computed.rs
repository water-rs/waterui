use crate::{array::waterui_str, closure::waterui_fn, IntoFFI, IntoRust};
use ::waterui_str::Str;
use waterui::ComputeExt;
use waterui_reactive::{watcher::WatcherGuard, Compute, Computed};

ffi_type!(
    waterui_computed_str,
    Computed<Str>,
    waterui_drop_computed_str
);
ffi_type!(
    waterui_computed_int,
    Computed<i32>,
    waterui_drop_computed_int
);

ffi_type!(
    waterui_computed_bool,
    Computed<bool>,
    waterui_drop_computed_bool
);
ffi_type!(
    waterui_watcher_guard,
    WatcherGuard,
    waterui_drop_watcher_guard
);

// WARNING: Computed<T> must be called on the Rust thread!!!
macro_rules! impl_computed {
    ($computed:ty,$ffi:ty,$read:ident,$watch:ident) => {
        #[no_mangle]
        pub unsafe extern "C" fn $read(computed: *const $computed) -> $ffi {
            (*computed).compute().into_ffi()
        }

        #[no_mangle]
        pub unsafe extern "C" fn $watch(
            computed: *const $computed,
            watcher: waterui_fn<$ffi>,
        ) -> *mut waterui_watcher_guard {
            let guard =
                (*computed).watch(move |v: <$ffi as IntoRust>::Rust| watcher.call(v.into_ffi()));
            guard.into_ffi()
        }
    };
}

impl_computed!(
    waterui_computed_str,
    waterui_str,
    waterui_read_computed_str,
    waterui_watch_computed_str
);

impl_computed!(
    waterui_computed_int,
    i32,
    waterui_read_computed_int,
    waterui_watch_computed_int
);

impl_computed!(
    waterui_computed_bool,
    bool,
    waterui_read_computed_bool,
    waterui_watch_computed_bool
);
