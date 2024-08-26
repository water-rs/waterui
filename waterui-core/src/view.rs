use crate::{components::text::text, AnyView, Environment};

use alloc::{boxed::Box, vec::Vec};
use waterui_reactive::Computed;
use waterui_str::Str;

/// View represents a part of the user interface.
///
/// You can create your custom view by implement this trait. You just need to implement fit.
pub trait View: 'static {
    /// Build this view and return the content.
    ///
    /// WARNING: This method should not be called directly by user.
    /// # Panic
    /// - If this view is a [native implement view](crate::component)  but you call it, it must panic.
    fn body(self, _env: Environment) -> impl View;
}

impl<V: View, E: View> View for Result<V, E> {
    fn body(self, _env: Environment) -> impl View {
        match self {
            Ok(view) => AnyView::new(view),
            Err(view) => AnyView::new(view),
        }
    }
}

impl<V: View> View for Option<V> {
    fn body(self, _env: Environment) -> impl View {
        match self {
            Some(view) => AnyView::new(view),
            None => AnyView::new(()),
        }
    }
}

pub trait TupleViews {
    fn into_views(self) -> Vec<AnyView>;
}

pub trait ConfigurableView: View {
    type Config: 'static;
    fn config(self) -> Self::Config;
}

pub struct Modifier<V: ConfigurableView>(Box<dyn Fn(Environment, V::Config) -> AnyView>);

impl<V, V2, F> From<F> for Modifier<V>
where
    V: ConfigurableView,
    V2: View,
    F: Fn(Environment, V::Config) -> V2 + 'static,
{
    fn from(value: F) -> Self {
        Self(Box::new(move |mut env, config| {
            env.remove::<Self>();
            AnyView::new(WithEnv::new(value(env.clone(), config), env))
        }))
    }
}

pub struct WithEnv {
    pub view: AnyView,
    pub env: Environment,
}

raw_view!(WithEnv);

impl WithEnv {
    pub fn new(view: impl View, env: Environment) -> Self {
        Self {
            view: AnyView::new(view),
            env,
        }
    }
}

impl<V: ConfigurableView> Modifier<V> {
    pub fn new<V2, F>(f: F) -> Self
    where
        V: ConfigurableView,
        V2: View,
        F: Fn(Environment, V::Config) -> V2 + 'static,
    {
        Self::from(f)
    }
    pub fn modify(&self, env: Environment, config: V::Config) -> AnyView {
        (self.0)(env, config)
    }
}

macro_rules! impl_tuple_views {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        #[allow(unused_parens)]
        impl <$($ty:View,)*>TupleViews for ($($ty),*){
            fn into_views(self) -> Vec<AnyView> {
                let ($($ty),*)=self;
                alloc::vec![$(AnyView::new($ty)),*]
            }
        }
    };
}

tuples!(impl_tuple_views);

raw_view!(());

impl View for ! {
    fn body(self, _env: Environment) -> impl View {}
}

impl<T> View for Computed<T>
where
    Str: From<T>,
{
    fn body(self, _env: Environment) -> impl View {
        text(self)
    }
}

raw_view!(Computed<AnyView>);
