macro_rules! native_implement {
    ($ty:ty) => {
        impl crate::View for $ty {
            fn view(&self) -> crate::view::BoxView {
                panic!("[Native implement]");
            }
        }
    };
}

macro_rules! into_ref_impl {
    (($target_ty:ty,$($source_ty:ty),*)) => {
        $(
            impl IntoRef<AttributedString> for $source_ty {
                fn into_ref(self) -> Ref<AttributedString> {
                    Ref::new(AttributedString::new(self))
                }
            }
        )*
    };
}
