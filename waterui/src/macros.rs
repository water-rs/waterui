macro_rules! raw_view {
    ($ty:ty) => {
        impl crate::View for $ty {
            fn body(self, _env: crate::Environment) -> impl crate::view::View {
                panic!("You cannot call `body` for a raw view, may you need to handle this view `{}` manually",core::any::type_name::<Self>());
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

macro_rules! impl_view {
    ($ty:ty,$ffi_ty:ty,$force_as:ident,$id:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(view: $crate::ffi::AnyView) -> $ffi_ty {
            let view: $crate::AnyView = view.into();
            (*view.downcast_unchecked::<$ty>()).into()
        }

        #[no_mangle]
        unsafe extern "C" fn $id() -> $crate::ffi::TypeId {
            core::any::TypeId::of::<$ty>().into()
        }
    };
}

macro_rules! impl_modifier_with_value {
    ($value:ty,$force_as:ident,$id:ident) => {
        impl_view!(
            $crate::modifier::WithValue<$value>,
            $crate::ffi::WithValue<$value>,
            $force_as,
            $id
        );
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
