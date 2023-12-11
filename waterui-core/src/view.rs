use crate::layout::{Alignment, Frame};
use crate::{layout::Size, modifier::ViewModifier};

use crate::component::TapGesture;
use crate::modifier::Modifier;
use std::{
    any::{type_name, TypeId},
    fmt::Debug,
    ops::Deref,
};

use crate::binding::SubscriberBuilderObject;

/// View represents a part of the user interface.
///
/// You can create your custom view by implement this trait. You just need to implement fit.
pub trait View: Reactive {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    /// # Panic
    /// - If this view is a [native implement view](crate::component)  but you call it, it must panic.
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

raw_view!(());

pub trait Reactive {
    fn subscribe(&self, _builder: SubscriberBuilderObject) {}
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

    fn subscribe(&self, subscriber: SubscriberBuilderObject) {
        (*self).subscribe(subscriber)
    }
}

impl<V: Reactive + ?Sized> Reactive for Box<V> {
    fn is_reactive(&self) -> bool {
        self.deref().is_reactive()
    }

    fn subscribe(&self, subscriber: SubscriberBuilderObject) {
        self.deref().subscribe(subscriber)
    }
}

impl<V: View + ?Sized + 'static> View for Box<V> {
    fn view(&self) -> Box<dyn View> {
        self.deref().view()
    }

    fn name(&self) -> &'static str {
        self.deref().name()
    }
}

impl Debug for dyn View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn View")
    }
}

pub trait ViewExt: View {
    fn modifier<T: ViewModifier>(self, modifier: T) -> Modifier<T>;
    fn on_tap(self, event: impl Fn() + 'static) -> TapGesture;
    fn width(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn height(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;

    fn leading(self) -> Modifier<Frame>;

    fn boxed(self) -> BoxView;
}

impl<V: View + 'static> ViewExt for V {
    fn modifier<T: ViewModifier>(self, modifier: T) -> Modifier<T> {
        Modifier::new(self.boxed(), modifier)
    }

    fn on_tap(self, event: impl Fn() + 'static) -> TapGesture {
        TapGesture::new(Box::new(self), Box::new(event))
    }

    fn width(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.boxed(), Frame::default().width(size))
    }

    fn height(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.boxed(), Frame::default().height(size))
    }

    fn leading(self) -> Modifier<Frame> {
        Modifier::new(self.boxed(), Frame::default().alignment(Alignment::Leading))
    }

    fn boxed(self) -> BoxView {
        Box::new(self)
    }
}
