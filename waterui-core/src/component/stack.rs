use crate::view::IntoViews;

use crate::{
    view::{BoxView, ViewExt},
    View,
};

macro_rules! impl_frame {
    ($($ty:ident),*) => {
        $(
            pub struct $ty {
                pub(crate)contents: Vec<BoxView>,
            }

            impl<V: View + 'static> FromIterator<V> for $ty {
                fn from_iter<T: IntoIterator<Item = V>>(iter: T) -> Self {
                    let content: Vec<BoxView> = iter.into_iter().map(|v| v.boxed()).collect();
                    Self::new(content)
                }
            }

            impl $ty {
                pub fn new(views: impl IntoViews) -> Self {
                    let contents = views.into_views();
                    Self { contents }
                }
            }
        )*

    };
}

impl_frame!(VStack, HStack);

raw_view!(VStack);
raw_view!(HStack);
