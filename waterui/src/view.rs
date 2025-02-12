pub use waterui_core::view::*;
use waterui_core::{
    env::With,
    handler::{Handler, HandlerFn, IntoHandler},
    AnyView, Environment,
};

use alloc::boxed::Box;
use waterui_reactive::{compute::IntoComputed, Binding, Computed};

use crate::{
    color::{BackgroundColor, Color, ForegroundColor},
    component::{badge::Badge, focu::Focused, navigation::NavigationView, text::Text, Metadata},
    layout::{Edge, Frame},
    utils::{Id, Mapping},
};

pub trait ViewExt: View + Sized {
    fn metadata<T>(self, metadata: T) -> Metadata<T>;
    fn with<T: 'static>(self, value: T) -> With<Self, T>;
    fn title(self, title: impl Into<Text>) -> NavigationView;
    fn anyview(self) -> AnyView;
    fn padding(self) -> Metadata<Edge>;
    fn focused<T: 'static + Eq + Clone>(
        self,
        value: Binding<Option<T>>,
        equals: T,
    ) -> Metadata<Focused>;
    fn background(self, color: impl IntoComputed<Color>) -> Metadata<BackgroundColor>;
    fn foreground(self, color: impl IntoComputed<Color>) -> Metadata<ForegroundColor>;
    fn frame(self, frame: impl IntoComputed<Frame>) -> Metadata<Computed<Frame>>;
    fn badge(self, value: impl IntoComputed<i32>) -> Badge;
    fn tag<T>(self, tag: T) -> TaggedView<T, Self>;
}

pub trait ConfigViewExt: ConfigurableView + Sized {
    fn modifier(self, modifier: impl Into<Modifier<Self>>) -> impl View;
}

impl<V: ConfigurableView> ConfigViewExt for V {
    fn modifier(self, modifier: impl Into<Modifier<Self>>) -> impl View {
        modifier.into().modify(Environment::new(), self.config())
    }
}

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

impl<V: View> ViewExt for V {
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

    fn background(self, color: impl IntoComputed<Color>) -> Metadata<BackgroundColor> {
        Metadata::new(self, BackgroundColor::new(color))
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

#[derive(Debug, Clone, PartialEq)]
pub struct TaggedView<T, V> {
    pub tag: T,
    pub content: V,
}

impl<T, V: View> TaggedView<T, V> {
    pub fn new(tag: T, content: V) -> Self {
        Self { tag, content }
    }

    pub fn map<F, T2>(self, f: F) -> TaggedView<T2, V>
    where
        F: Fn(T) -> T2,
    {
        TaggedView {
            tag: f(self.tag),
            content: self.content,
        }
    }

    pub fn mapping(self, mapping: &Mapping<T>) -> TaggedView<Id, V>
    where
        T: Ord + Clone,
    {
        self.map(move |v| mapping.register(v))
    }

    pub fn erase(self) -> TaggedView<T, AnyView> {
        TaggedView {
            tag: self.tag,
            content: self.content.anyview(),
        }
    }
}
