use std::{
    any::{type_name, TypeId},
    fmt::Debug,
};

use crate::{component::Text, Reactive};

/// View represents a part of the user interface.
///
/// You can create your custom view by implement this trait. You just need to implement fit.
pub trait View {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    /// # Panic
    /// - If this view is a [native implement view](crate::component)  but you call it, it must panic.
    fn view(self) -> BoxView;

    fn name(&self) -> &'static str {
        type_name::<Self>()
    }

    #[doc(hidden)]
    fn type_id(&self, _sealed: sealed::Sealed) -> TypeId
    where
        Self: 'static,
    {
        TypeId::of::<Self>()
    }
}

pub trait IntoView {
    type Output: View + 'static;
    fn into_view(self) -> Self::Output;

    fn into_boxed_view(self) -> BoxView
    where
        Self: Sized,
    {
        Box::new(self.into_view())
    }
}

impl<V: View + 'static> IntoView for V {
    type Output = V;
    fn into_view(self) -> Self::Output {
        self
    }
}

impl IntoView for &str {
    type Output = Text;
    fn into_view(self) -> Self::Output {
        Text::new(self)
    }
}

impl IntoView for String {
    type Output = Text;
    fn into_view(self) -> Self::Output {
        Text::new(self)
    }
}

pub trait IntoViews {
    fn into_views(self) -> Vec<BoxView>;
}

impl IntoViews for Vec<BoxView> {
    fn into_views(self) -> Vec<BoxView> {
        self
    }
}

macro_rules! impl_tuple_views {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:IntoView,)*>IntoViews for ($($ty),*){
            fn into_views(self) -> Vec<BoxView> {
                let ($($ty),*)=self;
                vec![$($ty.into_boxed_view()),*]
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

mod sealed {
    pub struct Sealed;
}

impl dyn View {
    pub fn inner_type_id(&self) -> TypeId {
        self.type_id(sealed::Sealed)
    }

    pub fn is<T: View + 'static>(&self) -> bool {
        self.inner_type_id() == TypeId::of::<T>()
    }

    pub fn downcast_ref<T: View + 'static>(&self) -> Option<&T> {
        if self.is::<T>() {
            unsafe { Some(&*(self as *const dyn View as *const T)) }
        } else {
            None
        }
    }

    pub fn downcast<T: View + 'static>(self: Box<Self>) -> Result<Box<T>, Box<dyn View>> {
        if self.is::<T>() {
            unsafe {
                let raw: *mut dyn View = Box::into_raw(self);
                Ok(Box::from_raw(raw as *mut T))
            }
        } else {
            Err(self)
        }
    }
}

pub type BoxView = Box<dyn View + 'static>;
raw_view!(Reactive<BoxView>);

raw_view!(BoxView);

impl Debug for dyn View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn View")
    }
}
