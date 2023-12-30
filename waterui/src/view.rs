use crate::{
    component::{AnyView, Text},
    env::Environment,
    layout::{Alignment, Frame, Size},
    modifier::{Modifier, ViewModifier},
    Reactive,
};

/// View represents a part of the user interface.
///
/// You can create your custom view by implement this trait. You just need to implement fit.
pub trait View: Send + Sync {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    /// # Panic
    /// - If this view is a [native implement view](crate::component)  but you call it, it must panic.
    fn body(self, _env: Environment) -> impl View;
}

pub trait IntoView: Sized {
    type Output: View + 'static;
    fn into_view(self) -> Self::Output;
    fn into_anyview(self) -> AnyView {
        AnyView::new(self.into_view())
    }
}

impl<V: View + 'static> IntoView for V {
    type Output = V;
    fn into_view(self) -> Self::Output {
        self
    }
}

pub type ViewBuilder = Box<dyn Fn() -> AnyView>;

impl IntoView for &str {
    type Output = Text;
    fn into_view(self) -> Self::Output {
        let value = self.to_string();
        Text::new(value)
    }
}

impl IntoView for String {
    type Output = Text;
    fn into_view(self) -> Self::Output {
        Text::new(self)
    }
}

impl IntoView for Reactive<String> {
    type Output = Text;
    fn into_view(self) -> Self::Output {
        Text::new(self)
    }
}

impl IntoView for Reactive<&str> {
    type Output = Text;
    fn into_view(self) -> Self::Output {
        Text::new(self)
    }
}

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
        impl <$($ty:IntoView,)*>IntoViews for ($($ty),*){
            fn into_views(self) -> Vec<AnyView> {
                let ($($ty),*)=self;
                vec![$($ty.into_anyview()),*]
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

raw_view!(Reactive<AnyView>);

pub trait ViewExt: View {
    fn modifier<T: ViewModifier>(self, modifier: T) -> Modifier<T>;
    fn width(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;
    fn height(self, size: impl Into<Size>) -> Modifier<Frame>
    where
        Self: Sized;

    fn leading(self) -> Modifier<Frame>;
    fn anyview(self) -> AnyView;
}

impl<V: View + 'static> ViewExt for V {
    fn modifier<T: ViewModifier>(self, modifier: T) -> Modifier<T> {
        Modifier::new(self.anyview(), modifier)
    }

    fn width(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.anyview(), Frame::default().width(size))
    }

    fn height(self, size: impl Into<Size>) -> Modifier<Frame> {
        Modifier::new(self.anyview(), Frame::default().height(size))
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
