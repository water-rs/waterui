use core::{
    any::Any,
    num::NonZeroUsize,
    ops::{Deref, DerefMut},
};

use crate::{AnyView, Environment};

use alloc::{boxed::Box, rc::Rc, vec::Vec};
use waterui_reactive::{Computed, Reactive};

/// View represents a part of the user interface.
///
/// You can create your custom view by implement this trait. You just need to implement fit.
pub trait View {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    /// # Panic
    /// - If this view is a [native implement view](crate::component)  but you call it, it must panic.
    fn body(self, _env: Environment) -> impl View;
}

impl<V: View + 'static, E: View + 'static> View for Result<V, E> {
    fn body(self, _env: Environment) -> impl View {
        match self {
            Ok(view) => AnyView::new(view),
            Err(view) => AnyView::new(view),
        }
    }
}

impl<V: View + 'static> View for Option<V> {
    fn body(self, _env: Environment) -> impl View {
        match self {
            Some(view) => AnyView::new(view),
            None => AnyView::new(()),
        }
    }
}

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;
pub type SharedViewBuilder = Rc<dyn Fn() -> AnyView>;

pub trait ReloadableView {
    fn reload(&self) -> impl View;
}

pub trait ReloadableViews: Reactive {
    type Item;
    fn id(&mut self, index: usize) -> usize; // return id
    fn pull(&mut self, index: usize) -> Self::Item;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

pub type BoxViews<T> = Box<dyn Views<Item = T>>;
pub trait Views: Reactive {
    type Item;
    fn id(&mut self, index: usize) -> usize; // return id
    fn pull(&mut self, index: usize) -> Option<Self::Item>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<T> Views for Box<dyn Views<Item = T>> {
    type Item = T;
    fn id(&mut self, index: usize) -> usize {
        self.deref_mut().id(index)
    }

    fn pull(&mut self, index: usize) -> Option<Self::Item> {
        self.deref_mut().pull(index)
    }

    fn len(&self) -> usize {
        self.deref().len()
    }

    fn is_empty(&self) -> bool {
        self.deref().is_empty()
    }
}

impl<T> Reactive for Box<dyn Views<Item = T>> {
    fn register_subscriber(
        &self,
        _subscriber: waterui_reactive::subscriber::BoxSubscriber,
    ) -> Option<NonZeroUsize> {
        None
    }

    fn cancel_subscriber(&self, _id: NonZeroUsize) {}

    fn notify(&self) {}
}

impl<T: ReloadableViews> Views for T {
    type Item = <T as ReloadableViews>::Item;
    fn id(&mut self, index: usize) -> usize {
        <T as ReloadableViews>::id(self, index)
    }
    fn pull(&mut self, index: usize) -> Option<Self::Item> {
        Some(<T as ReloadableViews>::pull(self, index))
    }
    fn len(&self) -> usize {
        <T as ReloadableViews>::len(self)
    }

    fn is_empty(&self) -> bool {
        <T as ReloadableViews>::is_empty(self)
    }
}

pub struct ConstantViews<T>(Vec<Option<T>>);

impl<T: View> ConstantViews<T> {
    pub fn new(views: impl IntoIterator<Item = T>) -> Self {
        Self(views.into_iter().map(|v| Some(v)).collect())
    }
}

impl<T> Reactive for ConstantViews<T> {
    fn register_subscriber(
        &self,
        _subscriber: waterui_reactive::subscriber::BoxSubscriber,
    ) -> Option<NonZeroUsize> {
        None
    }
    fn cancel_subscriber(&self, _id: NonZeroUsize) {}
    fn notify(&self) {}
}

impl<V> Views for ConstantViews<V> {
    type Item = V;
    fn id(&mut self, index: usize) -> usize {
        index
    }
    fn pull(&mut self, index: usize) -> Option<Self::Item> {
        self.0[index].take()
    }
    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<V: View> From<Vec<V>> for ConstantViews<V> {
    fn from(value: Vec<V>) -> Self {
        Self::new(value)
    }
}

macro_rules! impl_tuple_views {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:View+'static,)*>From<($($ty),*)> for ConstantViews<AnyView>{
            fn from(value:($($ty),*)) -> ConstantViews<AnyView> {
                let ($($ty),*)=value;
                ConstantViews::new(alloc::vec![$(AnyView::new($ty)),*])
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

raw_view!(Computed<AnyView>);

pub fn downcast<V: 'static>(view: impl View + 'static) -> Option<V> {
    let any = &mut Some(view) as &mut dyn Any;
    let any = any.downcast_mut::<Option<V>>();
    any.map(|v| v.take().unwrap())
}
