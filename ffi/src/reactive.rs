use crate::array::WuiArray;
use crate::components::form::WuiPickerItem;
use crate::components::media::{WuiLivePhotoSource, WuiVideo};
use crate::{IntoFFI, WuiAnyView, WuiId, WuiStr, impl_opaque_drop};
use alloc::vec::Vec;
use waterui::reactive::watcher::BoxWatcherGuard;
use waterui::{AnyView, Str};
use waterui::{Computed, reactive::watcher::Metadata};
use waterui_core::id::Id;
use waterui_form::picker::PickerItem;
use waterui_media::Video;
use waterui_media::live::LivePhotoSource;
ffi_type!(WuiWatcherMetadata, Metadata, waterui_drop_watcher_metadata);

ffi_type!(
    WuiWatcherGuard,
    BoxWatcherGuard,
    waterui_drop_box_watcher_guard
);

#[macro_export]
macro_rules! impl_computed {
    ($ty:ty,$ffi_ty:ty,$read:ident,$watch:ident,$drop:ident) => {
        impl $crate::OpaqueType for Computed<$ty> {}
        impl_opaque_drop!(Computed<$ty>, $drop);

        #[unsafe(no_mangle)]
        /// Reads the current value from a computed
        ///
        /// # Safety
        ///
        /// The computed pointer must be valid and point to a properly initialized computed object.
        pub unsafe extern "C" fn $read(computed: *const Computed<$ty>) -> $ffi_ty {
            use waterui::Signal;
            unsafe { (*computed).get().into_ffi() }
        }

        /// Watches for changes in a computed
        ///
        /// # Safety
        ///
        /// The computed pointer must be valid and point to a properly initialized computed object.
        /// The watcher must be a valid callback function.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $watch(
            computed: *const Computed<$ty>,
            watcher: $crate::reactive::WuiWatcher<$ffi_ty>,
        ) -> *mut $crate::reactive::WuiWatcherGuard {
            use waterui::Signal;
            use $crate::IntoFFI;
            unsafe {
                let guard =
                    (*computed).watch(move |ctx| watcher.call(ctx.value.into_ffi(), ctx.metadata));
                guard.into_ffi()
            }
        }
    };
}

#[macro_export]
macro_rules! impl_binding {
    ($ty:ty,$ffi_ty:ty,$read:ident,$set:ident,$watch:ident,$drop:ident) => {
        impl $crate::OpaqueType for waterui::Binding<$ty> {}
        impl_opaque_drop!(waterui::Binding<$ty>, $drop);

        #[unsafe(no_mangle)]
        /// Reads the current value from a binding
        ///
        /// # Safety
        ///
        /// The binding pointer must be valid and point to a properly initialized binding object.
        pub unsafe extern "C" fn $read(binding: *const waterui::Binding<$ty>) -> $ffi_ty {
            unsafe { $crate::IntoFFI::into_ffi((*binding).get()) }
        }

        /// Sets a new value to a binding
        ///
        /// # Safety
        ///
        /// The binding pointer must be valid and point to a properly initialized binding object.
        /// The value must be a valid instance of the FFI type.
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $set(binding: *mut waterui::Binding<$ty>, value: $ffi_ty) {
            unsafe {
                (*binding).set($crate::IntoRust::into_rust(value));
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
            binding: *const waterui::Binding<$ty>,
            watcher: $crate::reactive::WuiWatcher<$ffi_ty>,
        ) -> *mut $crate::reactive::WuiWatcherGuard {
            unsafe {
                use waterui::Signal;
                let guard = (*binding).watch(move |ctx| {
                    watcher.call($crate::IntoFFI::into_ffi(ctx.value), ctx.metadata)
                });
                guard.into_ffi()
            }
        }
    };
}

impl_computed!(
    Str,
    WuiStr,
    waterui_read_computed_str,
    waterui_watch_computed_str,
    waterui_drop_computed_str
);

impl_computed!(
    AnyView,
    *mut WuiAnyView,
    waterui_read_computed_any_view,
    waterui_watch_computed_any_view,
    waterui_drop_computed_any_view
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
    f32,
    f32,
    waterui_read_computed_float,
    waterui_watch_computed_float,
    waterui_drop_computed_float
);

impl_computed!(
    f64,
    f64,
    waterui_read_computed_double,
    waterui_watch_computed_double,
    waterui_drop_computed_double
);

// Computed<Vec<PickerItem<Id>>>,
impl_computed!(
    Vec<PickerItem<Id>>,
    WuiArray<WuiPickerItem>,
    waterui_read_computed_picker_items,
    waterui_watch_computed_picker_items,
    waterui_drop_computed_picker_items
);

impl_computed!(
    Video,
    WuiVideo,
    waterui_read_computed_video,
    waterui_watch_computed_video,
    waterui_drop_computed_video
);

impl_computed!(
    LivePhotoSource,
    WuiLivePhotoSource,
    waterui_read_computed_live_photo_source,
    waterui_watch_computed_live_photo_source,
    waterui_drop_computed_live_photo_sources
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
    WuiStr,
    waterui_read_binding_str,
    waterui_set_binding_str,
    waterui_watch_binding_str,
    waterui_drop_binding_str
);

impl_binding!(
    f32,
    f32,
    waterui_read_binding_float,
    waterui_set_binding_float,
    waterui_watch_binding_float,
    waterui_drop_binding_float
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

// Add Id binding support
impl_binding!(
    waterui_core::id::Id,
    WuiId,
    waterui_read_binding_id,
    waterui_set_binding_id,
    waterui_watch_binding_id,
    waterui_drop_binding_id
);
