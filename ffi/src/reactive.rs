use core::ops::Deref;

use crate::array::WuiArray;
use crate::components::form::WuiPickerItem;
use crate::components::media::{WuiLivePhotoSource, WuiVideo};
use crate::{IntoFFI, OpaqueType, WuiAnyView, WuiStr};
use alloc::boxed::Box;
use alloc::vec::Vec;
use waterui::reactive::watcher::BoxWatcherGuard;
use waterui::reactive::watcher::Metadata;
use waterui::{AnyView, Str};
use waterui_core::id::Id;
use waterui_form::picker::PickerItem;
use waterui_media::Video;
use waterui_media::live::LivePhotoSource;
opaque!(WuiWatcherMetadata, Metadata, watcher_metadata);

opaque!(WuiWatcherGuard, BoxWatcherGuard);

pub struct WuiComputed<T>(pub(crate) waterui::Computed<T>);
impl<T> IntoFFI for waterui::Computed<T> {
    type FFI = *mut WuiComputed<T>;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiComputed(self)))
    }
}

impl<T> IntoFFI for waterui::Binding<T> {
    type FFI = *mut WuiBinding<T>;

    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiBinding(self)))
    }
}

impl<T> OpaqueType for WuiComputed<T> {}

impl<T> Deref for WuiComputed<T> {
    type Target = waterui::Computed<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> Deref for WuiBinding<T> {
    type Target = waterui::Binding<T>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub struct WuiBinding<T: 'static>(pub(crate) waterui::Binding<T>);

impl<T> OpaqueType for WuiBinding<T> {}

#[macro_export]
macro_rules! ffi_binding {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        paste::paste!{
            /// Reads the current value from a binding
            /// # Safety
            /// The binding pointer must be valid and point to a properly initialized binding object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_read_binding_ $ident >](binding: *const $crate::reactive::WuiBinding<$ty>) -> $ffi {
                unsafe { (*binding).get().into_ffi() }
            }
            /// Sets the value of a binding
            /// # Safety
            /// The binding pointer must be valid and point to a properly initialized binding object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_set_binding_ $ident >](binding: *mut $crate::reactive::WuiBinding<$ty>, value: $ffi) {
                unsafe {
                    (*binding).set($crate::IntoRust::into_rust(value));
                }
            }
            /// Watches for changes in a binding
            /// # Safety
            /// The binding pointer must be valid and point to a properly initialized binding object.
            /// The watcher must be a valid callback function.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_watch_binding_ $ident >](
                binding: *const waterui_core::Binding<$ty>,
                watcher: $crate::reactive::WuiWatcher<$ffi>,
            ) -> *mut $crate::reactive::WuiWatcherGuard {
                unsafe {
                    use waterui::Signal;
                    let guard = (*binding).watch(move |ctx| {
                        let metadata = ctx.metadata().clone();
                        let value = ctx.into_value();
                        watcher.call(value, metadata);
                    });
                    guard.into_ffi()
                }

            }
            /// Drops a binding
            /// # Safety
            /// The caller must ensure that `binding` is a valid pointer obtained from the corresponding FFI function.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_drop_binding_ $ident >](binding: *mut $crate::reactive::WuiBinding<$ty>) {
                unsafe {
                    drop(alloc::boxed::Box::from_raw(binding));
                }
            }
        }
    };

    ($ty:ty,$ffi:ty) =>{
        paste::paste!{
            $crate::ffi_binding!($ty,$ffi,[<$ty:snake>]);
        }
    }
}

#[macro_export]
macro_rules! ffi_computed {
    ($ty:ty,$ffi:ty, $ident:tt) => {

        paste::paste!{
            #[unsafe(no_mangle)]
        /// Reads the current value from a computed
        ///
        /// # Safety
        ///
        /// The computed pointer must be valid and point to a properly initialized computed object.
        pub unsafe extern "C" fn [< waterui_read_computed_ $ident >](computed: *const $crate::reactive::WuiComputed<$ty>) -> $ffi {
            use waterui::Signal;
            unsafe{
                $crate::IntoFFI::into_ffi((&(*computed)).get())
            }
        }

        /// Watches for changes in a computed
        ///
        /// # Safety
        ///
        /// The computed pointer must be valid and point to a properly initialized computed object.
        /// The watcher must be a valid callback function.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn [< waterui_watch_computed_ $ident >](
            computed: *const waterui_core::Computed<$ty>,
            watcher: $crate::reactive::WuiWatcher<$ffi>,
        ) -> *mut $crate::reactive::WuiWatcherGuard {
            use waterui::Signal;
            unsafe {
                let guard = (*computed).watch(move |ctx| {
                    let metadata = ctx.metadata().clone();
                    let value = ctx.into_value();
                    watcher.call(value, metadata);
                });
                $crate::IntoFFI::into_ffi(guard)
            }
        }

        /// Drops a computed
        /// # Safety
        /// The caller must ensure that `computed` is a valid pointer obtained from the corresponding FFI function.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn [< waterui_drop_computed_ $ident >](computed: *mut $crate::reactive::WuiComputed<$ty>) {
            unsafe {
                drop(alloc::boxed::Box::from_raw(computed));
            }
        }


    }

    };

    ($ty:ty,$ffi:ty) =>{
        paste::paste!{
            $crate::ffi_computed!($ty,$ffi,[<$ty:snake>]);
        }
    }
}

#[macro_export]
macro_rules! ffi_reactive {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        paste::paste! {
            $crate::ffi_binding!($ty,$ffi, $ident);
            $crate::ffi_computed!($ty,$ffi, $ident);
        }
    };

    ($ty:ty,$ffi:ty) => {
        paste::paste! {
            $crate::ffi_reactive!($ty,$ffi,[<$ty:snake>]);
        }
    };
}

ffi_reactive!(Str, WuiStr);

ffi_reactive!(AnyView, *mut WuiAnyView);

ffi_reactive!(i32, i32);

ffi_reactive!(bool, bool);

ffi_reactive!(f32, f32);

ffi_reactive!(f64, f64);

// Computed<Vec<PickerItem<Id>>>,
ffi_computed!(Vec<PickerItem<Id>>, WuiArray<WuiPickerItem>, picker_items);

ffi_computed!(Video, WuiVideo);

ffi_computed!(LivePhotoSource, WuiLivePhotoSource);

#[repr(C)]
pub struct WuiWatcher<T> {
    data: *mut (),
    call: unsafe extern "C" fn(*const (), T, *mut WuiWatcherMetadata),
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
        call: unsafe extern "C" fn(*const (), T, *mut WuiWatcherMetadata),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        Self { data, call, drop }
    }
    pub fn call(&self, value: impl IntoFFI<FFI = T>, metadata: Metadata) {
        unsafe { (self.call)(self.data, value.into_ffi(), metadata.into_ffi()) }
    }
}

impl<T> Drop for WuiWatcher<T> {
    fn drop(&mut self) {
        unsafe { (self.drop)(self.data) }
    }
}
