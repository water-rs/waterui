macro_rules! native_implement {
    ($ty:ty) => {
        impl crate::View for $ty {
            fn view(&self) -> crate::view::BoxView {
                panic!("[Native implement]");
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
