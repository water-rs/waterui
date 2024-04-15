macro_rules! raw_view {
    ($ty:ty) => {
        impl crate::View for $ty {
            fn body(self, _env: crate::Environment) -> impl crate::view::View {
                panic!("You cannot call `view` for a raw view");
            }
        }
    };
}

macro_rules! impl_debug {
    ($ty:ty) => {
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
    ($ty:ident,$force_as:ident,$id:ident) => {
        #[no_mangle]
        unsafe extern "C" fn $force_as(view: $crate::ffi::AnyView) -> $ty {
            let view: $crate::component::AnyView = view.into();
            (*view.downcast_unchecked::<$crate::component::$ty>()).into()
        }

        #[no_mangle]
        unsafe extern "C" fn $id() -> $crate::ffi::TypeId {
            std::any::TypeId::of::<$crate::component::$ty>().into()
        }
    };
}

macro_rules! impl_computed {
    ($read:ident,$subscribe:ident,$unsubscribe:ident,$drop:ident,$computed_ty:ident,$ty:ty,$output_ty:ty) => {
        ffi_opaque!(Computed<$ty>, $computed_ty, 2);

        #[no_mangle]
        unsafe extern "C" fn $read(computed: $computed_ty) -> $output_ty {
            let computed = ManuallyDrop::new(Computed::from(computed));
            computed.compute().into()
        }

        #[no_mangle]
        unsafe extern "C" fn $subscribe(computed: $computed_ty, subscriber: Subscriber) -> usize {
            let computed = ManuallyDrop::new(Computed::from(computed));
            computed.register_subscriber(Box::new(move || subscriber.call()))
        }

        #[no_mangle]
        unsafe extern "C" fn $unsubscribe(computed: $computed_ty, id: usize) {
            let computed = ManuallyDrop::new(Computed::from(computed));
            computed.cancel_subscriber(id);
        }

        #[no_mangle]
        unsafe extern "C" fn $drop(computed: $computed_ty) {
            let _ = Computed::from(computed);
        }
    };
}

macro_rules! ffi_opaque {
    ($from:ty,$to:ident,$word:expr) => {
        #[repr(C)]
        pub struct $to {
            inner: [usize; $word],
            _marker: std::marker::PhantomData<(*const (), std::marker::PhantomPinned)>,
        }

        #[allow(clippy::missing_transmute_annotations)]
        impl From<$from> for $to {
            fn from(value: $from) -> Self {
                unsafe {
                    Self {
                        inner: std::mem::transmute(value),
                        _marker: std::marker::PhantomData,
                    }
                }
            }
        }

        impl From<$to> for $from {
            fn from(value: $to) -> Self {
                unsafe { std::mem::transmute(value.inner) }
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

        impl From<Vec<$from>> for $name {
            fn from(value: Vec<$from>) -> Self {
                let len = value.len();
                let head = Box::into_raw(value.into_boxed_slice()) as *mut $to;

                Self { head, len }
            }
        }

        impl From<$name> for Vec<$from> {
            fn from(value: $name) -> Self {
                unsafe {
                    Box::from_raw(
                        std::ptr::slice_from_raw_parts_mut(value.head, value.len) as *mut [$from]
                    )
                    .into_vec()
                }
            }
        }
    };
}
