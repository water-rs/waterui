//! This module provides implementations of the `View` trait for common string types,
//! allowing them to be used directly as views in the UI system.

use alloc::{borrow::Cow, string::String};
use waterui_str::Str;

use crate::View;

/// A macro that implements the `View` trait for multiple string types.
///
/// This macro takes a list of types and implements the `View` trait for each of them,
/// converting the type to a `Str` for rendering.
macro_rules! impl_label {
    ($($ty:ty),*) => {
        $(
            impl View for $ty {
                fn body(self, _env: &crate::Environment) -> impl View {
                    Str::from(self)
                }
            }

        )*
    };
}

// Implement `View` for common string types
impl_label!(&'static str, String, Cow<'static, str>);

// Define Str as a raw view
raw_view!(Str);

mod ffi {
    use waterui_str::Str;

    use crate::AnyView;

    pub struct Label(String);
    impl From<String> for Label {
        fn from(str: String) -> Self {
            Label(str)
        }
    }

    impl From<Label> for String {
        fn from(value: Label) -> Self {
            value.0
        }
    }

    impl Label {
        #[uniffi::constructor]
        pub fn new(text: AnyView) -> Self {
            Self(text.downcast::<Str>().unwrap().into_string())
        }
    }

    uniffi::custom_newtype!(Label, String);
}
