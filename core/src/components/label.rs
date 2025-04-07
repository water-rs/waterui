use alloc::{borrow::Cow, string::String};
use waterui_str::Str;

use crate::View;

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

impl_label!(&'static str, String, Cow<'static, str>);

raw_view!(Str);
