use alloc::{borrow::Cow, string::String};
use waterui_reactive::{compute::IntoComputed, ComputeExt, Computed};
use waterui_str::Str;

use crate::View;

#[derive(Debug, Clone, Default)]
pub struct Label {
    pub content: Computed<Str>,
}

macro_rules! impl_label {
    ($($ty:ty),*) => {
        $(
            impl View for $ty {
                fn body(self, _env: &crate::Environment) -> impl View {
                    IntoComputed::<Str>::into_computed(self)
                }
            }

            impl View for Computed<$ty> {
                fn body(self, _env: &crate::Environment) -> impl View {
                    self.map(|value| Str::from(value)).computed()
                }
            }
        )*
    };
}

impl_label!(&'static str, String, Cow<'static, str>);

impl View for Str {
    fn body(self, _env: &crate::Environment) -> impl View {
        self.computed()
    }
}

raw_view!(Computed<Str>);
