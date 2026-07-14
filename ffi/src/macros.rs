#[macro_export]
/// Declares types as FFI-safe by implementing `IntoFFI` and `IntoRust` traits.
///
/// This macro automatically implements the necessary traits to make the specified types
/// usable across the FFI boundary. It creates trivial implementations where the FFI
/// representation is the same as the Rust representation.
///
/// # Arguments
///
/// * `$ty` - One or more types to make FFI-safe
///
/// # Example
///
/// ```ignore
/// ffi_safe!(u32, i32, bool);
/// ```
macro_rules! ffi_safe {
    ($($ty:ty),*) => {
       $(
            impl $crate::IntoFFI for $ty {
                type FFI = $ty;
                fn into_ffi(self) -> Self::FFI {
                    self
                }
            }


            impl $crate::IntoRust for $ty{
                type Rust=$ty;
                unsafe fn into_rust(self) -> Self::Rust{
                    self
                }
            }
       )*
    };
}

/// Generates FFI functions for view types.
///
/// When `c-api` feature is enabled (default), generates C FFI functions.
/// When `android-jni` feature is enabled, generates JNI functions.
///
/// # Generated Functions (C-API)
/// - `waterui_<ident>_id()` - Returns the type ID as 128-bit value
/// - `waterui_force_as_<ident>()` - Downcasts AnyView to the view type
///
/// # Generated Functions (Android-JNI)
/// - `Java_dev_waterui_android_ffi_WatcherJni_<ident>Id()` - Returns TypeIdStruct
/// - `Java_dev_waterui_android_ffi_WatcherJni_forceAs<Ident>()` - Returns struct
///
/// Generates FFI functions for view types.
///
/// Uses `:lower_camel` for JNI names (colorPickerId) and `:camel` for forceAs (forceAsColorPicker).
#[macro_export]
macro_rules! ffi_view {
    ($view:ty, $ffi:ty, $ident:tt) => {
        $crate::ffi_view!($view, $ffi, $ident, all());
    };

    ($view:ty, $ffi:ty, $ident:tt, $jni:meta) => {
        pastey::paste! {
            // ========== C-API (for Apple/GTK backends) ==========
            /// # Safety
            ///
            /// `view` must be a valid, owning `WuiAnyView` handle whose erased value is a
            /// `Native<_>` of the expected view type; it is consumed by this call and must
            /// not be used afterwards.
            #[cfg(feature = "c-api")]
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_force_as_ $ident>](view: *mut $crate::WuiAnyView) -> $ffi {
                unsafe {
                    let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
                    let view = (*any.downcast_unchecked::<waterui_core::Native<$view>>());
                    $crate::IntoFFI::into_ffi(view)
                }
            }

            #[cfg(feature = "c-api")]
            #[unsafe(no_mangle)]
            pub extern "C" fn [<waterui_ $ident _id>]() -> $crate::WuiTypeId {
                $crate::WuiTypeId::of::<waterui_core::Native<$view>>()
            }

            // ========== Android JNI ==========
            // :lower_camel → buttonId, colorPickerId (first letter lowercase)
            // :camel → Button, ColorPicker (first letter uppercase, for forceAs prefix)
            #[cfg(all(feature = "android-jni", $jni))]
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_ $ident:lower_camel Id>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
            ) -> $crate::jni::jobject {
                let type_id = $crate::WuiTypeId::of::<waterui_core::Native<$view>>();
                $crate::jni::type_id_to_java(&mut env, type_id).into_raw()
            }

            /// # Safety
            ///
            /// `view_ptr` must be a valid, owning `WuiAnyView` handle (as a `jlong`) whose
            /// erased value is a `Native<_>` of the expected view type; it is consumed by
            /// this call and must not be used afterwards.
            #[cfg(all(feature = "android-jni", $jni))]
            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_forceAs $ident:camel>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                view_ptr: $crate::jni::jlong,
            ) -> $crate::jni::jobject {
                use $crate::jni::convert::jlong_to_ptr_mut;
                unsafe {
                    let view_ptr: *mut $crate::WuiAnyView = jlong_to_ptr_mut(view_ptr);
                    let any: waterui::AnyView = $crate::IntoRust::into_rust(view_ptr);
                    let view = (*any.downcast_unchecked::<waterui_core::Native<$view>>());
                    let ffi_struct: $ffi = $crate::IntoFFI::into_ffi(view);
                    $crate::jni::convert::struct_to_java(&mut env, &ffi_struct).into_raw()
                }
            }
        }
    };
}

/// Generates FFI functions for Metadata<T> types.
///
/// Unlike `ffi_view!`, this macro does NOT wrap in `Native<T>` because
/// `Metadata<T>` is stored directly in the view tree (not as a native view).
///
/// # Generated Functions (C-API)
/// - `waterui_metadata_<ident>_id()` - Returns the type ID as 128-bit value
/// - `waterui_force_as_metadata_<ident>()` - Downcasts AnyView to the metadata type
///
/// # Generated Functions (Android-JNI)
/// - `Java_dev_waterui_android_ffi_WatcherJni_metadata<Ident>Id()` - Returns TypeIdStruct
/// - `Java_dev_waterui_android_ffi_WatcherJni_forceAsMetadata<Ident>()` - Returns struct
#[macro_export]
macro_rules! ffi_metadata {
    ($ty:ty, $ffi:ty, $ident:tt) => {
        pastey::paste! {
            // ========== C-API (for Apple/GTK backends) ==========
            #[cfg(feature = "c-api")]
            /// Returns the type ID as a 128-bit value for O(1) comparison.
            /// Returns the view's TypeId (guaranteed unique within a single binary).
            #[unsafe(no_mangle)]
            pub extern "C" fn [<waterui_metadata_ $ident _id>]() -> $crate::WuiTypeId {
                // Metadata<T> is stored directly, not wrapped in Native<T>
                $crate::WuiTypeId::of::<waterui_core::Metadata<$ty>>()
            }

            #[cfg(feature = "c-api")]
            /// Force-casts an AnyView to this metadata type
            ///
            /// # Safety
            /// The caller must ensure that `view` is a valid pointer to an `AnyView`
            /// that contains a `Metadata<$ty>`.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_force_as_metadata_ $ident>](
                view: *mut $crate::WuiAnyView
            ) -> $ffi {
                unsafe {
                    let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
                    // Metadata<T> is stored directly, not wrapped in Native<T>
                    let metadata = *any.downcast_unchecked::<waterui_core::Metadata<$ty>>();
                    $crate::IntoFFI::into_ffi(metadata)
                }
            }

            // ========== Android JNI ==========
            #[cfg(feature = "android-jni")]
            /// JNI: Returns the type ID for this metadata type.
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_metadata $ident:camel Id>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
            ) -> $crate::jni::jobject {
                let type_id = $crate::WuiTypeId::of::<waterui_core::Metadata<$ty>>();
                $crate::jni::type_id_to_java(&mut env, type_id).into_raw()
            }

            #[cfg(feature = "android-jni")]
            /// JNI: Force-casts an AnyView pointer to this metadata type.
            ///
            /// # Safety
            /// The view pointer must be valid and contain the expected metadata type.
            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_forceAsMetadata $ident:camel>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                view_ptr: $crate::jni::jlong,
            ) -> $crate::jni::jobject {
                use $crate::jni::convert::jlong_to_ptr_mut;
                unsafe {
                    let view_ptr: *mut $crate::WuiAnyView = jlong_to_ptr_mut(view_ptr);
                    let any: waterui::AnyView = $crate::IntoRust::into_rust(view_ptr);
                    let metadata = *any.downcast_unchecked::<waterui_core::Metadata<$ty>>();
                    let ffi_struct: $ffi = $crate::IntoFFI::into_ffi(metadata);
                    $crate::jni::convert::struct_to_java(&mut env, &ffi_struct).into_raw()
                }
            }
        }
    };
}

/// Generates FFI functions for IgnorableMetadata<T> types.
///
/// Similar to `ffi_metadata!`, but for `IgnorableMetadata<T>` which can be
/// safely ignored by renderers that don't support the metadata type.
///
/// # Generated Functions (C-API)
/// - `waterui_ignorable_metadata_<ident>_id()` - Returns the type ID as 128-bit value
/// - `waterui_force_as_ignorable_metadata_<ident>()` - Downcasts AnyView to the metadata type
///
/// # Generated Functions (Android-JNI)
/// - `Java_dev_waterui_android_ffi_WatcherJni_ignorableMetadata<Ident>Id()` - Returns TypeIdStruct
/// - `Java_dev_waterui_android_ffi_WatcherJni_forceAsIgnorableMetadata<Ident>()` - Returns struct
#[macro_export]
macro_rules! ffi_ignorable_metadata {
    ($ty:ty, $ffi:ty, $ident:tt) => {
        pastey::paste! {
            // ========== C-API (for Apple/GTK backends) ==========
            #[cfg(feature = "c-api")]
            /// Returns the type ID as a 128-bit value for O(1) comparison.
            /// Returns the view's TypeId (guaranteed unique within a single binary).
            #[unsafe(no_mangle)]
            pub extern "C" fn [<waterui_ignorable_metadata_ $ident _id>]() -> $crate::WuiTypeId {
                // IgnorableMetadata<T> is stored directly, not wrapped in Native<T>
                $crate::WuiTypeId::of::<waterui_core::IgnorableMetadata<$ty>>()
            }

            #[cfg(feature = "c-api")]
            /// Force-casts an AnyView to this ignorable metadata type
            ///
            /// # Safety
            /// The caller must ensure that `view` is a valid pointer to an `AnyView`
            /// that contains an `IgnorableMetadata<$ty>`.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_force_as_ignorable_metadata_ $ident>](
                view: *mut $crate::WuiAnyView
            ) -> $ffi {
                unsafe {
                    let any: waterui::AnyView = $crate::IntoRust::into_rust(view);
                    // IgnorableMetadata<T> is stored directly, not wrapped in Native<T>
                    let metadata = *any.downcast_unchecked::<waterui_core::IgnorableMetadata<$ty>>();
                    $crate::IntoFFI::into_ffi(metadata)
                }
            }

            // ========== Android JNI ==========
            #[cfg(feature = "android-jni")]
            /// JNI: Returns the type ID for this ignorable metadata type.
            #[unsafe(no_mangle)]
            pub extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_ignorableMetadata $ident:camel Id>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
            ) -> $crate::jni::jobject {
                let type_id = $crate::WuiTypeId::of::<waterui_core::IgnorableMetadata<$ty>>();
                $crate::jni::type_id_to_java(&mut env, type_id).into_raw()
            }

            #[cfg(feature = "android-jni")]
            /// JNI: Force-casts an AnyView pointer to this ignorable metadata type.
            ///
            /// # Safety
            /// The view pointer must be valid and contain the expected ignorable metadata type.
            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_forceAsIgnorableMetadata $ident:camel>]<'local>(
                mut env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                view_ptr: $crate::jni::jlong,
            ) -> $crate::jni::jobject {
                use $crate::jni::convert::jlong_to_ptr_mut;
                unsafe {
                    let view_ptr: *mut $crate::WuiAnyView = jlong_to_ptr_mut(view_ptr);
                    let any: waterui::AnyView = $crate::IntoRust::into_rust(view_ptr);
                    let metadata = *any.downcast_unchecked::<waterui_core::IgnorableMetadata<$ty>>();
                    let ffi_struct: $ffi = $crate::IntoFFI::into_ffi(metadata);
                    $crate::jni::convert::struct_to_java(&mut env, &ffi_struct).into_raw()
                }
            }
        }
    };
}

/// Defines an opaque FFI type that is passed as a pointer across FFI boundaries.
///
/// This macro creates a wrapper struct and implements `IntoFFI`/`IntoRust` traits
/// for pointer-based transfer. It also generates a drop function for cleanup.
///
/// When `c-api` feature is enabled, generates `waterui_drop_<ident>()`.
/// When `android-jni` feature is enabled, generates JNI drop function.
#[macro_export]
macro_rules! opaque {
    ($name:ident,$ty:ty,$ident:tt,$jni_drop:meta) => {
        #[allow(nonstandard_style)]
        pub struct $name(pub(crate) $ty);

        $crate::impl_deref!($name, $ty);

        impl $crate::IntoFFI for $ty {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                alloc::boxed::Box::into_raw(alloc::boxed::Box::new($name(self)))
            }
        }

        impl $crate::IntoFFI for Option<$ty> {
            type FFI = *mut $name;
            fn into_ffi(self) -> Self::FFI {
                if let Some(value) = self {
                    value.into_ffi()
                } else {
                    core::ptr::null::<$name>() as *mut $name
                }
            }
        }

        impl $crate::IntoRust for *mut $name {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                unsafe { alloc::boxed::Box::from_raw(self).0 }
            }
        }

        pastey::paste! {
            // ========== C-API (for Apple/GTK backends) ==========
            #[cfg(feature = "c-api")]
            /// # Safety
            /// The caller must ensure that `value` is a valid pointer obtained from the corresponding FFI function.
            #[unsafe(no_mangle)]
            pub unsafe extern "C" fn [<waterui_drop_ $ident>](value: *mut $name) {
                unsafe {
                    let _ = $crate::IntoRust::into_rust(value);
                }
            }

            // ========== Android JNI ==========
            #[cfg(all(feature = "android-jni", $jni_drop))]
            /// JNI: Drops the opaque pointer, freeing its memory.
            ///
            /// # Safety
            /// The pointer must be valid and not have been dropped before.
            #[unsafe(no_mangle)]
            pub unsafe extern "system" fn [<Java_dev_waterui_android_ffi_WatcherJni_drop $ident:camel>]<'local>(
                _env: $crate::jni::JNIEnv<'local>,
                _class: $crate::jni::JClass<'local>,
                ptr: $crate::jni::jlong,
            ) {
                use $crate::jni::convert::jlong_to_ptr_mut;
                unsafe {
                    let ptr: *mut $name = jlong_to_ptr_mut(ptr);
                    let _ = $crate::IntoRust::into_rust(ptr);
                }
            }
        }
    };

    ($name:ident,$ty:ty,$ident:tt) => {
        $crate::opaque!($name, $ty, $ident, all());
    };

    ($name:ident,$ty:ty) => {
        pastey::paste! {
            $crate::opaque!($name,$ty,[<$ty:snake>]);
        }
    };
}

/// Derive `IntoFFI` trait for a struct or enum
/// # Example
/// ```ignore
/// into_ffi!{
///   ListConfig,
///   struct WuiList{
///      contents: *mut WuiAnyViews,
///   }
/// }
/// ```
macro_rules! into_ffi {
    ($ty:ty, $(#[$meta:meta])* pub struct $ffi:ident { $($field:ident : $ftype:ty),* $(,)? }) => {
        $(#[$meta])*
        #[repr(C)]
        pub struct $ffi {
            $(pub $field: $ftype),*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                let value = self;
                $ffi {
                    $($field: $crate::IntoFFI::into_ffi(value.$field)),*
                }
            }
        }
    };

    ($ty:ty, $(#[$meta:meta])* pub enum $ffi:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy)]
        #[repr(C)]
        pub enum $ffi {
            $($variant),*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                match self {
                    $(<$ty>::$variant => $ffi::$variant),*
                }
            }
        }

        impl $crate::IntoRust for $ffi {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                match self {
                    $( $ffi::$variant => <$ty>::$variant ),*
                }
            }
        }
    };

    // Non-exhaustive Rust enum. Unknown variants are unsupported by this ABI.
    ($ty:ty, non_exhaustive, $(#[$meta:meta])* pub enum $ffi:ident { $($variant:ident),* $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy)]
        #[repr(C)]
        pub enum $ffi {
            $($variant),*
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $ffi;
            fn into_ffi(self) -> Self::FFI {
                match self {
                    $(<$ty>::$variant => $ffi::$variant),*,
                    _ => panic!(concat!("unsupported ", stringify!($ty), " variant")),
                }
            }
        }

        impl $crate::IntoRust for $ffi {
            type Rust = $ty;
            unsafe fn into_rust(self) -> Self::Rust {
                match self {
                    $( $ffi::$variant => <$ty>::$variant ),*
                }
            }
        }
    };
}

#[macro_export]
macro_rules! impl_deref {
    ($ty:ty,$target:ty) => {
        impl core::ops::Deref for $ty {
            type Target = $target;
            fn deref(&self) -> &Self::Target {
                &self.0
            }
        }

        impl core::ops::DerefMut for $ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.0
            }
        }
    };
}
