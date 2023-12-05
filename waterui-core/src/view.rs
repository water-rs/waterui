use std::{
    any::{type_name, TypeId},
    fmt::Debug,
    ops::Deref,
};

use crate::binding::BoxSubscriber;
pub trait View: Reactive {
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

pub trait ViewBuilder<T = ()> {
    type Output: View + 'static;
    fn build(&self, context: T) -> Self::Output;
}

macro_rules! impl_view_builder {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        impl<F,V:View+'static,$($ty,)*> ViewBuilder<($($ty,)*)> for F
        where
            F: Fn($($ty,)*) -> V,
        {
            type Output=V;
            #[allow(unused_variables)]
            fn build(&self, context: ($($ty,)*)) -> V {
                let ($($ty,)*)=context;
                (self)($($ty,)*)
            }
        }

    };
}

tuples!(impl_view_builder);

impl<F, V: View + 'static> ViewBuilder<()> for F
where
    F: Fn() -> V,
{
    type Output = V;
    fn build(&self, _context: ()) -> V {
        (self)()
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

impl View for () {
    fn view(self) -> crate::view::BoxView {
        panic!("[Native implement]");
    }
}

impl Reactive for () {}

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

pub type BoxView = Box<dyn View>;

impl View for BoxView {
    fn view(self) -> Box<dyn View> {
        self
    }
}

impl<V: Reactive> Reactive for &V {
    fn is_reactive(&self) -> bool {
        (*self).is_reactive()
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        (*self).subscribe(subscriber)
    }
}

impl<V: Reactive> Reactive for Box<V> {
    fn is_reactive(&self) -> bool {
        self.deref().is_reactive()
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        self.deref().subscribe(subscriber)
    }
}

impl<V: View + 'static> View for Box<V> {
    fn view(self) -> Box<dyn View> {
        Box::new(self)
    }
}

impl Reactive for BoxView {
    fn is_reactive(&self) -> bool {
        self.deref().is_reactive()
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        self.deref().subscribe(subscriber)
    }
}

impl Debug for dyn View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn View")
    }
}
