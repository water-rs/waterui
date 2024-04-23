#[macro_export]
macro_rules! ffi_opaque {
    ($name:ident,$ty:ty,$word:expr) => {
        #[repr(C)]
        pub struct $name {
            inner: [usize; $word],
            _marker: core::marker::PhantomData<(*const (), core::marker::PhantomPinned)>,
        }

        impl core::ops::Deref for $name {
            type Target = $ty;
            fn deref(&self) -> &Self::Target {
                unsafe { core::mem::transmute(&self.inner) }
            }
        }

        impl core::ops::DerefMut for $name {
            fn deref_mut(&mut self) -> &mut Self::Target {
                unsafe { core::mem::transmute(&mut self.inner) }
            }
        }

        impl Drop for $name {
            fn drop(&mut self) {
                let _: $ty = unsafe { core::mem::transmute(self.inner) };
            }
        }

        impl $crate::IntoFFI for $ty {
            type FFI = $name;

            fn into_ffi(self) -> Self::FFI {
                unsafe {
                    $name {
                        inner: core::mem::transmute::<$ty, [usize; $word]>(self),
                        _marker: core::marker::PhantomData,
                    }
                }
            }
        }

        impl $crate::IntoRust for $name {
            type Rust = $ty;
            fn into_rust(self) -> Self::Rust {
                unsafe { core::mem::transmute(self) }
            }
        }
    };

    ($name:ident,$ty:ty,$word:expr,$drop:ident) => {
        $crate::ffi_opaque!($name, $ty, $word);

        #[no_mangle]
        unsafe extern "C" fn $drop(value: $name) {
            let _ = value;
        }
    };
}

#[macro_export]
macro_rules! impl_array {
    ($name:ident,$ty:ty,$ffi:ty) => {
        #[repr(C)]
        pub struct $name {
            head: *mut $ffi,
            len: usize,
        }

        impl core::ops::Deref for $name {
            type Target = [$ty];
            fn deref(&self) -> &Self::Target {
                unsafe { &*(core::ptr::slice_from_raw_parts(self.head, self.len) as *const [$ty]) }
            }
        }

        impl $crate::IntoFFI for alloc::vec::Vec<$ty> {
            type FFI = $name;

            fn into_ffi(mut self) -> Self::FFI {
                let len = self.len();
                let head = self.as_mut_ptr() as *mut $ffi;
                core::mem::forget(self);

                $name { head, len }
            }
        }

        impl $crate::IntoRust for $name {
            type Rust = alloc::vec::Vec<$ty>;
            fn into_rust(self) -> Self::Rust {
                unsafe {
                    alloc::boxed::Box::from_raw(core::ptr::slice_from_raw_parts_mut(
                        self.head, self.len,
                    ) as *mut [$ty])
                    .into_vec()
                }
            }
        }
    };
}

#[macro_export]
macro_rules! ffi_view {
    ($name:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(view: $crate::AnyView) -> $ffi {
            let any: waterui_view::AnyView = $crate::IntoRust::into_rust(view);
            let view = (*any.downcast_unchecked::<$name>());
            $crate::IntoFFI::into_ffi(view)
        }

        #[no_mangle]
        unsafe extern "C" fn $id() -> $crate::TypeId {
            $crate::IntoFFI::into_ffi(core::any::TypeId::of::<$name>())
        }
    };
}

#[macro_export]
macro_rules! ffi_safe {
    ($($ty:ty),*) => {
       $(
            impl IntoFFI for $ty {
                type FFI = $ty;

                fn into_ffi(self) -> Self::FFI {
                    self
                }
            }


            impl IntoRust for $ty{
                type Rust=$ty;
                fn into_rust(self) -> Self::Rust{
                    self
                }
            }
       )*
    };
}

#[macro_export]
macro_rules! ffi_clone {
    ($f:ident,$ffi:ty) => {
        #[no_mangle]
        unsafe extern "C" fn $f(pointer: *const $ffi) -> $ffi {
            $crate::IntoFFI::into_ffi(Clone::clone(core::ops::Deref::deref(&*pointer)))
        }
    };
}
