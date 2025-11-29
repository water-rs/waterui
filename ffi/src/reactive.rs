use core::ops::Deref;

use crate::array::WuiArray;
use crate::components::form::WuiPickerItem;
use crate::components::media::{WuiLivePhotoSource, WuiVideo};
use crate::{IntoFFI, IntoRust, OpaqueType, WuiAnyView, WuiStr};
use alloc::boxed::Box;
use alloc::rc::Rc;
use alloc::vec::Vec;
use nami::watcher::{Context, Watcher, WatcherGuard};
use nami::{Computed, Signal, watcher};
use waterui::reactive::watcher::BoxWatcherGuard;
use waterui::reactive::watcher::Metadata;
use waterui::{AnyView, Str};
use waterui_core::id::Id;
use waterui_form::picker::PickerItem;
use waterui_media::Video;
use waterui_media::live::LivePhotoSource;
opaque!(WuiWatcherMetadata, Metadata, watcher_metadata);

opaque!(WuiWatcherGuard, BoxWatcherGuard);

#[repr(transparent)]
pub struct WuiComputed<T>(pub(crate) waterui::Computed<T>);

impl<T> WuiComputed<T>
where
    T: IntoFFI,
    T::FFI: IntoRust<Rust = T>,
{
    pub unsafe fn new(
        data: *mut (),
        get: unsafe extern "C" fn(*const ()) -> T::FFI,
        watch: unsafe extern "C" fn(*const (), *mut WuiWatcher<T>) -> *mut WuiWatcherGuard,
        drop: unsafe extern "C" fn(*mut ()),
    ) -> WuiComputed<T>
    where
        T: IntoFFI + 'static,
    {
        unsafe { WuiComputed(Computed::new(FFIComputed::new(data, get, watch, drop))) }
    }
}

struct FFIComputed<T: IntoFFI> {
    data: *mut (), // we use reference counting in the watcher to manage lifetime
    get: unsafe extern "C" fn(*const ()) -> T::FFI,
    watch: unsafe extern "C" fn(*const (), *mut WuiWatcher<T>) -> *mut WuiWatcherGuard,
    drop: unsafe extern "C" fn(*mut ()),
}

impl<T: IntoFFI> Signal for FFIComputed<T>
where
    T::FFI: IntoRust<Rust = T>,
{
    type Output = T;
    type Guard = BoxWatcherGuard;
    fn get(&self) -> Self::Output {
        unsafe {
            // self.data is Rc<*mut ()>*, dereference to get the original native pointer
            let rc = Rc::from_raw(self.data as *const *mut ());
            let native_data = *rc;
            let _ = Rc::into_raw(rc); // don't drop the Rc
            let ffi_value = (self.get)(native_data as *const ());
            ffi_value.into_rust()
        }
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let watcher: Watcher<Self::Output> = Rc::new(watcher);
        let watcher = watcher.into_ffi();

        unsafe {
            // self.data is Rc<*mut ()>*, dereference to get the original native pointer
            let rc = Rc::from_raw(self.data as *const *mut ());
            let native_data = *rc;
            let _ = Rc::into_raw(rc); // don't drop the Rc
            let guard_ptr = (self.watch)(native_data as *const (), watcher);
            (*Box::from_raw(guard_ptr)).0
        }
    }
}

impl<T: IntoFFI> FFIComputed<T> {
    pub unsafe fn new(
        data: *mut (),
        get: unsafe extern "C" fn(*const ()) -> T::FFI,
        watch: unsafe extern "C" fn(*const (), *mut WuiWatcher<T>) -> *mut WuiWatcherGuard,
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        // Wrap the native data pointer in an Rc for reference counting during clones
        let data = Rc::into_raw(Rc::new(data)) as *mut ();
        Self {
            data,
            get,
            watch,
            drop,
        }
    }
}

impl<T: IntoFFI> Clone for FFIComputed<T> {
    fn clone(&self) -> Self {
        unsafe {
            let rc = Rc::from_raw(self.data as *const *mut ());
            let cloned_data = Rc::into_raw(rc.clone()) as *mut ();
            let _ = Rc::into_raw(rc); // prevent dropping the original Rc
            Self {
                data: cloned_data,
                get: self.get,
                watch: self.watch,
                drop: self.drop,
            }
        }
    }
}

impl<T: IntoFFI> Drop for FFIComputed<T> {
    fn drop(&mut self) {
        unsafe {
            let rc = Rc::from_raw(self.data as *const *mut ());
            // Only call native drop when this is the last reference
            if Rc::strong_count(&rc) == 1 {
                let native_data = *rc;
                (self.drop)(native_data);
            }
            // rc drops here, decrementing count
        }
    }
}

impl<T: 'static> IntoFFI for waterui::Computed<T> {
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

impl<T: 'static> OpaqueType for WuiComputed<T> {}

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

#[repr(transparent)]
pub struct WuiBinding<T: 'static>(pub(crate) waterui::Binding<T>);

impl<T> OpaqueType for WuiBinding<T> {}

/// Generates computed FFI support for read-only reactive types.
///
/// Generates:
/// - `waterui_read_computed_{ident}` - read current value
/// - `waterui_watch_computed_{ident}` - subscribe to changes
/// - `waterui_drop_computed_{ident}` - cleanup
/// - `waterui_clone_computed_{ident}` - clone signal
/// - `waterui_new_watcher_{ident}` - create watcher
///
/// For types that also need native-controlled signal constructors,
/// additionally invoke `ffi_computed_ctor!`.
#[macro_export]
macro_rules! ffi_computed {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        paste::paste!{
            /// Reads the current value from a computed
            /// # Safety
            /// The computed pointer must be valid and point to a properly initialized computed object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_read_computed_ $ident >](computed: *const $crate::reactive::WuiComputed<$ty>) -> $ffi {
                use waterui::Signal;
                unsafe { $crate::IntoFFI::into_ffi((&(*computed)).get()) }
            }

            /// Watches for changes in a computed
            /// # Safety
            /// The computed pointer must be valid and point to a properly initialized computed object.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_watch_computed_ $ident >](
                computed: *const $crate::reactive::WuiComputed<$ty>,
                watcher: *mut $crate::reactive::WuiWatcher<$ty>,
            ) -> *mut $crate::reactive::WuiWatcherGuard {
                use waterui::Signal;
                unsafe {
                    let guard = (&*computed).watch(move |ctx| {
                        let metadata = ctx.metadata().clone();
                        let value = ctx.into_value();
                        (*watcher).call(value, metadata);
                    });
                    $crate::IntoFFI::into_ffi(guard)
                }
            }

            /// Drops a computed
            /// # Safety
            /// The caller must ensure that `computed` is a valid pointer.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_drop_computed_ $ident >](computed: *mut $crate::reactive::WuiComputed<$ty>) {
                unsafe { drop(alloc::boxed::Box::from_raw(computed)); }
            }

            /// Clones a computed
            /// # Safety
            /// The caller must ensure that `computed` is a valid pointer.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_clone_computed_ $ident >](computed: *const $crate::reactive::WuiComputed<$ty>) -> *mut $crate::reactive::WuiComputed<$ty> {
                unsafe {
                    let cloned = (*computed).clone();
                    $crate::IntoFFI::into_ffi(cloned)
                }
            }

            /// Creates a watcher from native callbacks.
            /// # Safety
            /// All function pointers must be valid.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_new_watcher_ $ident >](
                data: *mut (),
                call: unsafe extern "C" fn(*mut (), $ffi, *mut $crate::reactive::WuiWatcherMetadata),
                drop: unsafe extern "C" fn(*mut ()),
            ) -> *mut $crate::reactive::WuiWatcher<$ty>
            where
                $ty: $crate::IntoFFI + 'static,
            {
                use alloc::boxed::Box;
                let call: unsafe extern "C" fn(
                    *mut (),
                    <$ty as $crate::IntoFFI>::FFI,
                    *mut $crate::reactive::WuiWatcherMetadata,
                ) = unsafe { core::mem::transmute(call) };
                let watcher = unsafe { $crate::reactive::WuiWatcher::new(data, call, drop) };
                Box::into_raw(Box::new(watcher))
            }
        }
    };

    ($ty:ty,$ffi:ty) => {
        paste::paste! {
            $crate::ffi_computed!($ty, $ffi, [<$ty:snake>]);
        }
    }
}

/// Generates the native-controlled computed constructor.
///
/// Generates `waterui_new_computed_{ident}` for creating signals from native callbacks.
/// Requires the FFI type to implement `IntoRust`.
#[macro_export]
macro_rules! ffi_computed_ctor {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        paste::paste!{
            /// Creates a computed signal from native callbacks.
            /// # Safety
            /// All function pointers must be valid and follow the expected calling conventions.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [< waterui_new_computed_ $ident >](
                data: *mut (),
                get: unsafe extern "C" fn(*const ()) -> $ffi,
                watch: unsafe extern "C" fn(*const (), *mut $crate::reactive::WuiWatcher<$ty>) -> *mut $crate::reactive::WuiWatcherGuard,
                drop: unsafe extern "C" fn(*mut ()),
            ) -> *mut $crate::reactive::WuiComputed<$ty>
            where
                $ty: $crate::IntoFFI + 'static,
                <$ty as $crate::IntoFFI>::FFI: $crate::IntoRust<Rust = $ty>,
            {
                let get: unsafe extern "C" fn(*const ()) -> <$ty as $crate::IntoFFI>::FFI =
                    unsafe { core::mem::transmute(get) };
                let computed = unsafe { $crate::reactive::WuiComputed::new(data, get, watch, drop) };
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new(computed))
            }
        }
    };

    ($ty:ty,$ffi:ty) => {
        paste::paste! {
            $crate::ffi_computed_ctor!($ty, $ffi, [<$ty:snake>]);
        }
    }
}

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
                binding: *const $crate::reactive::WuiBinding<$ty>,
                watcher: *mut $crate::reactive::WuiWatcher<$ty>,
            ) -> *mut $crate::reactive::WuiWatcherGuard {
                use waterui::Signal;
                use core::cell::Cell;
                use alloc::rc::Rc;

                // Filter out synchronous callbacks during setup to prevent re-entrancy deadlocks
                let is_setting_up = Rc::new(Cell::new(true));
                let is_setting_up_clone = is_setting_up.clone();

                unsafe {
                    let guard = (*binding).watch(move |ctx| {
                        if is_setting_up_clone.get() {
                            return; // Skip synchronous callback during setup
                        }
                        let metadata = ctx.metadata().clone();
                        let value = ctx.into_value();
                        (*watcher).call(value, metadata);
                    });
                    is_setting_up.set(false);
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

/// Generates both binding and computed FFI support.
///
/// Use this for types that need two-way reactive binding support.
#[macro_export]
macro_rules! ffi_reactive {
    ($ty:ty,$ffi:ty, $ident:tt) => {
        $crate::ffi_binding!($ty, $ffi, $ident);
        $crate::ffi_computed!($ty, $ffi, $ident);
    };

    ($ty:ty,$ffi:ty) => {
        paste::paste! {
            $crate::ffi_reactive!($ty, $ffi, [<$ty:snake>]);
        }
    };
}

ffi_reactive!(Str, WuiStr);

ffi_reactive!(AnyView, *mut WuiAnyView);

ffi_reactive!(i32, i32);

ffi_reactive!(bool, bool);

ffi_reactive!(f32, f32);

ffi_reactive!(f64, f64);

ffi_computed!(Vec<PickerItem<Id>>, WuiArray<WuiPickerItem>, picker_items);

ffi_computed!(Video, WuiVideo);

ffi_computed!(LivePhotoSource, WuiLivePhotoSource);

pub struct WuiWatcher<T: IntoFFI>(watcher::Watcher<T>);

impl<T: IntoFFI> WuiWatcher<T> {
    /// Creates a new FFI watcher using C-style function pointers.
    ///
    /// # Safety
    /// The caller must ensure that the provided function pointers are valid and adhere to the expected signatures
    pub unsafe fn new(
        data: *mut (),
        call: unsafe extern "C" fn(*mut (), T::FFI, *mut WuiWatcherMetadata),
        drop: unsafe extern "C" fn(*mut ()),
    ) -> Self {
        struct Cleaner {
            data: *mut (),
            drop: unsafe extern "C" fn(*mut ()),
        }

        impl Drop for Cleaner {
            fn drop(&mut self) {
                unsafe { (self.drop)(self.data) }
            }
        }
        let cleaner = Cleaner { data, drop };
        WuiWatcher(Rc::new(move |ctx| {
            let _ = &cleaner; // Closure captures cleaner to ensure it lives as long as the watcher.
            let metadata = ctx.metadata().clone();
            let value = ctx.into_value();
            unsafe {
                call(data, value.into_ffi(), metadata.into_ffi());
            }
        }))
    }

    pub fn call(&self, value: T, metadata: Metadata) {
        (self.0)(Context::new(value, metadata));
    }
}

impl<T: IntoFFI> IntoFFI for Watcher<T> {
    type FFI = *mut WuiWatcher<T>;
    fn into_ffi(self) -> Self::FFI {
        Box::into_raw(Box::new(WuiWatcher(self)))
    }
}

/// Creates a new watcher guard from raw data and a drop function.
///
/// # Safety
/// The caller must ensure that the provided data pointer and drop function are valid.
#[unsafe(no_mangle)]
pub extern "C" fn waterui_new_watcher_guard(
    data: *mut (),
    drop: unsafe extern "C" fn(*mut ()),
) -> *mut WuiWatcherGuard {
    struct Cleaner {
        data: *mut (),
        drop: unsafe extern "C" fn(*mut ()),
    }

    impl Drop for Cleaner {
        fn drop(&mut self) {
            unsafe { (self.drop)(self.data) }
        }
    }

    let cleaner = Cleaner { data, drop };
    impl WatcherGuard for Cleaner {}
    Box::into_raw(Box::new(WuiWatcherGuard(Box::new(cleaner))))
}
