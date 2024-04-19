use core::any::Any;

use crate::{AnyView, Environment};

use alloc::{boxed::Box, rc::Rc, vec::Vec};
use waterui_reactive::Computed;

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

pub trait IntoViews {
    fn into_views(self) -> Vec<AnyView>;
}

impl IntoViews for Vec<AnyView> {
    fn into_views(self) -> Vec<AnyView> {
        self
    }
}

macro_rules! impl_tuple_views {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:View+'static,)*>IntoViews for ($($ty),*){
            fn into_views(self) -> Vec<AnyView> {
                let ($($ty),*)=self;
                alloc::vec![$(AnyView::new($ty)),*]
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
