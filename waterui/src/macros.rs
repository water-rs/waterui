macro_rules! raw_view {
    ($ty:ty) => {
        impl crate::View for $ty {
            fn body(self, _env: crate::Environment) -> impl crate::view::View {
                panic!("You cannot call `view` for a raw view");
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
