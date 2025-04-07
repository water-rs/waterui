pub use waterui_core::view::*;
use waterui_core::{
    AnyView, Color, Environment,
    env::With,
    handler::{Handler, HandlerFn, IntoHandler},
};

use alloc::boxed::Box;
use waterui_navigation::NavigationView;
use waterui_reactive::{Binding, Computed, compute::IntoComputed};

use crate::background::{Background, ForegroundColor};
use crate::component::{Metadata, Text, badge::Badge, focu::Focused};
use waterui_core::id::TaggedView;

use waterui_layout::{Edge, Frame};

pub trait ConfigViewExt: ConfigurableView + Sized {
    fn modifier(self, modifier: impl Into<Modifier<Self>>) -> impl View {
        modifier.into().modify(Environment::new(), self.config())
    }
}

impl<V: ConfigurableView> ConfigViewExt for V {}

pub trait ViewBuilder: 'static {
    fn view(&self, env: &Environment) -> impl View;
}

impl<F, V> ViewBuilder for F
where
    F: Fn(Environment) -> V + 'static,
    V: View,
{
    fn view(&self, env: &Environment) -> impl View {
        (self)(env.clone())
    }
}

impl ViewBuilder for () {
    fn view(&self, _env: &Environment) -> impl View {}
}

impl<H, P, V> ViewBuilder for IntoHandler<H, P, V>
where
    H: HandlerFn<P, V>,
    P: 'static,
    V: View,
{
    fn view(&self, env: &Environment) -> impl View {
        self.handle(env)
    }
}
pub struct AnyViewBuilder(Box<dyn Fn(Environment) -> AnyView>);

impl_debug!(AnyViewBuilder);

impl AnyViewBuilder {
    pub fn new(builder: impl ViewBuilder + 'static) -> Self {
        Self(Box::new(move |env| builder.view(&env).anyview()))
    }
}

impl ViewBuilder for AnyViewBuilder {
    fn view(&self, env: &Environment) -> impl View {
        (self.0)(env.clone())
    }
}

pub trait ViewExt: View + Sized {
    fn metadata<T>(self, metadata: T) -> Metadata<T> {
        Metadata::new(self, metadata)
    }

    fn with<T: 'static>(self, value: T) -> With<Self, T> {
        With::new(self, value)
    }

    fn title(self, title: impl Into<Text>) -> NavigationView {
        NavigationView::new(title, self)
    }

    fn focused<T: 'static + Eq + Clone>(
        self,
        value: Binding<Option<T>>,
        equals: T,
    ) -> Metadata<Focused> {
        Metadata::new(self, Focused::new(value, equals))
    }

    fn anyview(self) -> AnyView {
        AnyView::new(self)
    }

    fn padding(self) -> Metadata<Edge> {
        Metadata::new(self, Edge::default())
    }
    fn frame(self, frame: impl IntoComputed<Frame>) -> Metadata<Computed<Frame>> {
        Metadata::new(self, frame.into_computed())
    }

    fn background(self, background: impl Into<Background>) -> Metadata<Background> {
        Metadata::new(self, background.into())
    }
    fn foreground(self, color: impl IntoComputed<Color>) -> Metadata<ForegroundColor> {
        Metadata::new(self, ForegroundColor::new(color))
    }
    fn badge(self, value: impl IntoComputed<i32>) -> Badge {
        Badge::new(value, self)
    }
    fn tag<T>(self, tag: T) -> TaggedView<T, Self> {
        TaggedView::new(tag, self)
    }
}

impl<V: View + Sized> ViewExt for V {}
