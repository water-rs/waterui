use std::{
    any::{type_name, TypeId},
    fmt::Debug,
    ops::Deref,
};

use crate::binding::BoxSubscriber;
pub trait View: Reactive {
    fn view(&self) -> BoxView;

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
        impl <$($ty:View+'static,)*>IntoViews for ($($ty),*){
            fn into_views(self) -> Vec<BoxView> {
                let ($($ty),*)=self;
                vec![$(Box::new($ty)),*]
            }
        }
    };
}

tuples!(impl_tuple_views);

native_implement!(());

pub trait Reactive {
    fn subscribe(&self, _subscriber: fn() -> BoxSubscriber) {}
    fn is_reactive(&self) -> bool {
        false
    }
}

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

impl<V: Reactive> Reactive for &V {
    fn is_reactive(&self) -> bool {
        (*self).is_reactive()
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        (*self).subscribe(subscriber)
    }
}

impl<V: Reactive + ?Sized> Reactive for Box<V> {
    fn is_reactive(&self) -> bool {
        self.deref().is_reactive()
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        self.deref().subscribe(subscriber)
    }
}

impl<V: View + ?Sized + 'static> View for Box<V> {
    fn view(&self) -> Box<dyn View> {
        self.deref().view()
    }
}

impl Debug for dyn View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn View")
    }
}
