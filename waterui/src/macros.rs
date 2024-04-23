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
        impl $ty<$crate::component::Text> {
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

macro_rules! ffi_with_modifier {
    ($ty:ty,$ffi:ty,$force_as:ident,$id:ident) => {
        impl $crate::modifier::Modifier for $ty {
            fn modify(
                self,
                _env: $crate::Environment,
                view: impl $crate::View + 'static,
            ) -> impl $crate::View + 'static {
                $crate::modifier::with::WithValue::new(view, self)
            }
        }

        waterui_ffi::ffi_view!(
            $crate::modifier::with::WithValue<$ty>,
            $crate::modifier::with::ffi::WithValue<$ffi>,
            $force_as,
            $id
        );
    };

    ($ty:ty,$force_as:ident,$id:ident) => {
        ffi_with_modifier!($ty, $ty, $force_as, $id);
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
