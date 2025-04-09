use crate::{
    Compute, ComputeExt,
    watcher::{Metadata, Watcher, WatcherGuard},
};
use waterui_ffi::{IntoFFI, IntoRust, OpaqueType, impl_opaque_drop};
use waterui_str::Str;
impl OpaqueType for WatcherGuard {}

macro_rules! impl_computed {
    ($ty:ty,$ffi_ty:ty,$read:ident,$watch:ident,$drop:ident) => {
        impl OpaqueType for $crate::Computed<$ty> {}
        impl_opaque_drop!($crate::Computed<$ty>, $drop);

        #[unsafe(no_mangle)]
        /// Reads the current value from a computed
        ///
        /// # Safety
        ///
        /// The computed pointer must be valid and point to a properly initialized computed object.
        pub unsafe extern "C" fn $read(computed: *const $crate::Computed<$ty>) -> $ffi_ty {
            unsafe { (*computed).compute().into_ffi() }
        }

        /// Watches for changes in a computed
        ///
        /// # Safety
        ///
        /// The computed pointer must be valid and point to a properly initialized computed object.
        /// The watcher must be a valid callback function.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $watch(
            computed: *const $crate::Computed<$ty>,
            watcher: WuiWatcher<$ffi_ty>,
        ) -> *mut WatcherGuard {
            use $crate::ffi::IntoFFI;
            unsafe {
                let guard = (*computed).watch(Watcher::new(move |v: $ty, metadata| {
                    watcher.call(v.into_ffi(), metadata)
                }));
                guard.into_ffi()
            }
        }
    };
}

macro_rules! impl_binding {
    ($ty:ty,$ffi_ty:ty,$read:ident,$set:ident,$watch:ident,$drop:ident) => {
        impl OpaqueType for $crate::Binding<$ty> {}
        impl_opaque_drop!($crate::Binding<$ty>, $drop);

        #[unsafe(no_mangle)]
        /// Reads the current value from a binding
        ///
        /// # Safety
        ///
        /// The binding pointer must be valid and point to a properly initialized binding object.
        pub unsafe extern "C" fn $read(binding: *const $crate::Binding<$ty>) -> $ffi_ty {
            unsafe { (*binding).compute().into_ffi() }
        }

        /// Sets a new value to a binding
        ///
        /// # Safety
        ///
        /// The binding pointer must be valid and point to a properly initialized binding object.
        /// The value must be a valid instance of the FFI type.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $set(binding: *mut $crate::Binding<$ty>, value: $ffi_ty) {
            unsafe {
                (*binding).set(value.into_rust());
            }
        }

        #[unsafe(no_mangle)]
        /// Watches for changes in a binding
        ///
        /// # Safety
        ///
        /// The binding pointer must be valid and point to a properly initialized binding object.
        /// The watcher must be a valid callback function.
        pub unsafe extern "C" fn $watch(
            binding: *const $crate::Binding<$ty>,
            watcher: WuiWatcher<$ffi_ty>,
        ) -> *mut WatcherGuard {
            unsafe {
                let guard = (*binding).watch(Watcher::new(move |v: $ty, metadata| {
                    watcher.call(v.into_ffi(), metadata)
                }));
                guard.into_ffi()
            }
        }
    };
}

impl_computed!(
    Str,
    Str,
    waterui_read_computed_str,
    waterui_watch_computed_str,
    waterui_drop_computed_str
);

impl_computed!(
    i32,
    i32,
    waterui_read_computed_int,
    waterui_watch_computed_int,
    waterui_drop_computed_int
);

impl_computed!(
    bool,
    bool,
    waterui_read_computed_bool,
    waterui_watch_computed_bool,
    waterui_drop_computed_bool
);

impl_computed!(
    f64,
    f64,
    waterui_read_computed_double,
    waterui_watch_computed_double,
    waterui_drop_computed_double
);

#[repr(C)]
pub struct WuiWatcher<T> {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), T, *const Metadata),
    drop: unsafe extern "C" fn(*mut ()),
}

impl<T: 'static> WuiWatcher<T> {
    /// Creates a new watcher with the given data, call function, and drop function.
    ///
    /// # Safety
    ///
    /// The caller must ensure that:
    /// - `data` points to valid data that can be safely accessed throughout the lifetime of the watcher.
    /// - `call` is a valid function that can safely operate on the provided `data` and `T` value.
    /// - `drop` is a valid function that can safely free the resources associated with `data`.
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*const (), T, *const Metadata),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self { data, call, drop }
    }
    pub fn call(&self, value: T, metadata: Metadata) {
        unsafe { (self.call)(self.data, value, (&metadata) as *const Metadata) }
    }
}

impl<T> Drop for WuiWatcher<T> {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}

impl_binding!(
    Str,
    Str,
    waterui_read_binding_str,
    waterui_set_binding_str,
    waterui_watch_binding_str,
    waterui_drop_binding_str
);

impl_binding!(
    f64,
    f64,
    waterui_read_binding_double,
    waterui_set_binding_double,
    waterui_watch_binding_double,
    waterui_drop_binding_double
);

impl_binding!(
    i32,
    i32,
    waterui_read_binding_int,
    waterui_set_binding_int,
    waterui_watch_binding_int,
    waterui_drop_binding_int
);

impl_binding!(
    bool,
    bool,
    waterui_read_binding_bool,
    waterui_set_binding_bool,
    waterui_watch_binding_bool,
    waterui_drop_binding_bool
);
