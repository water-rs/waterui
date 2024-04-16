use crate::{
    component::AnyView,
    env::Environment,
    layout::{Alignment, Frame, Size},
    modifier::{Display, Modifier, ViewModifier},
    Compute, ComputeExt, Computed,
};

use alloc::{boxed::Box, vec, vec::Vec};

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

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;

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
                vec![$($ty.anyview()),*]
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

raw_view!(Computed<AnyView>);

pub trait ViewExt: View {
    fn modifier<T: ViewModifier>(self, modifier: impl Compute<Output = T>) -> Modifier<T>;
    fn width(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn height(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn show(self, condition: impl Compute<Output = bool> + Clone) -> Modifier<Display>;
    fn leading(self) -> Modifier<Frame>;
    fn anyview(self) -> AnyView;
}

impl<V: View + 'static> ViewExt for V {
    fn modifier<T: ViewModifier>(self, modifier: impl Compute<Output = T>) -> Modifier<T> {
        Modifier::new(self.anyview(), modifier)
    }

    fn width(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.anyview(), Frame::default().width(size))
    }

    fn height(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.anyview(), Frame::default().height(size))
    }

    fn show(self, condition: impl Compute<Output = bool> + Clone) -> Modifier<Display> {
        self.modifier(condition.transform(Display::new))
    }

    fn leading(self) -> Modifier<Frame> {
        Modifier::new(
            self.anyview(),
            Frame::default().alignment(Alignment::Leading),
        )
    }

    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }
}
