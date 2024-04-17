macro_rules! raw_view {
    ($ty:ty) => {
        impl crate::View for $ty {
            fn body(self, _env: crate::Environment) -> impl crate::view::View {
                panic!("You cannot call `body` for a raw view, may you need to handle this view `{}` manually",core::any::type_name::<$ty>());
            }
        }
    };
}

macro_rules! impl_debug {
    ($ty:ty) => {
        impl core::fmt::Debug for $ty {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(stringify!($ty))
            }
        }
    };
}

macro_rules! impl_from {
    ($enum_ty:ty,$ty:tt) => {
        impl From<$ty> for $enum_ty {
            fn from(value: $ty) -> Self {
                Self::$ty(value)
            }
        }
    };

    ($enum_ty:ty,$ty:ty,$variant_name:ident) => {
        impl From<$ty> for $enum_ty {
            fn from(value: $ty) -> Self {
                Self::$variant_name(value)
            }
        }
    };
}

macro_rules! impl_label {
    ($ty:ident) => {
        impl $ty<Text> {
            pub fn font(mut self, font: crate::component::text::Font) -> Self {
                self.label = self.label.font(font);
                self
            }

            pub fn size(mut self, size: f64) -> Self {
                self.label = self.label.size(size);
                self
            }
        }
    };
}

macro_rules! impl_builder {
    ($(#[$meta:meta])* $vis:vis struct $name:ident{$($field_vis:vis $field_name:ident:$field_type:ty),*}) => {
        $(#[$meta])*
        $vis struct $name{
            $($field_vis $field_name:$field_type),*
        }

        impl $name{
            $(
                pub fn $field_name(mut self,value:impl Into<$field_type>) -> Self{
                    self.$field_name=value.into();
                    self
                }
            )*
        }
    };
}

macro_rules! tuples {
    ($macro:ident) => {
        $macro!(T0);
        $macro!(T0, T1);
        $macro!(T0, T1, T2);
        $macro!(T0, T1, T2, T3);
        $macro!(T0, T1, T2, T3, T4);
        $macro!(T0, T1, T2, T3, T4, T5);
        $macro!(T0, T1, T2, T3, T4, T5, T6);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13);
        $macro!(T0, T1, T2, T3, T4, T5, T6, T7, T8, T9, T10, T11, T12, T13, T14);
    };
}

macro_rules! impl_view {
    ($ty:ty,$ffi_ty:ty,$force_as:ident,$id:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(view: $crate::ffi::AnyView) -> $ffi_ty {
            let view: $crate::component::AnyView = view.into();
            (*view.downcast_unchecked::<$ty>()).into()
        }

        #[no_mangle]
        unsafe extern "C" fn $id() -> $crate::ffi::TypeId {
            core::any::TypeId::of::<$ty>().into()
        }
    };
}

macro_rules! ffi_opaque {
    ($ty:ty,$ffi_ty:ident,$word:expr) => {
        #[repr(C)]
        pub struct $ffi_ty {
            inner: [usize; $word],
            _marker: core::marker::PhantomData<(*const (), core::marker::PhantomPinned)>,
        }

        impl core::ops::Deref for $ffi_ty {
            type Target = $ty;
            fn deref(&self) -> &Self::Target {
                unsafe { core::mem::transmute(&self.inner) }
            }
        }

        impl core::ops::DerefMut for $ffi_ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                unsafe { core::mem::transmute(&mut self.inner) }
            }
        }

        #[allow(clippy::missing_transmute_annotations)]
        impl From<$ty> for $ffi_ty {
            fn from(value: $ty) -> Self {
                unsafe {
                    Self {
                        inner: core::mem::transmute(value),
                        _marker: core::marker::PhantomData,
                    }
                }
            }
        }

        impl $ffi_ty {
            pub fn into_ty(self) -> $ty {
                unsafe { core::mem::transmute(self) }
            }
        }

        impl From<$ffi_ty> for $ty {
            fn from(value: $ffi_ty) -> Self {
                value.into_ty()
            }
        }

        impl Drop for $ffi_ty {
            fn drop(&mut self) {
                let _: $ty = unsafe { core::mem::transmute(self.inner) };
            }
        }
    };
}

macro_rules! impl_array {
    ($name:ident,$from:ty,$to:ty) => {
        #[repr(C)]
        pub struct $name {
            head: *mut $to,
            len: usize,
        }

        impl From<alloc::vec::Vec<$from>> for $name {
            fn from(value: alloc::vec::Vec<$from>) -> Self {
                let len = value.len();
                let value = value.into_boxed_slice();
                let head = value.as_ptr() as *mut $to;
                core::mem::forget(value);

                Self { head, len }
            }
        }

        impl From<$name> for alloc::vec::Vec<$from> {
            fn from(value: $name) -> Self {
                unsafe {
                    alloc::boxed::Box::from_raw(core::ptr::slice_from_raw_parts_mut(
                        value.head, value.len,
                    ) as *mut [$from])
                    .into_vec()
                }
            }
        }
    };
}
