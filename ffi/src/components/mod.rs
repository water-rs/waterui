pub mod layout;

pub mod native {
    use crate::IntoFFI;

    use waterui::component::Native;

    impl<T: IntoFFI> IntoFFI for Native<T> {
        type FFI = T::FFI;
        fn into_ffi(self) -> Self::FFI {
            IntoFFI::into_ffi(self.0)
        }
    }
}
