pub use waterui_core::view::*;
use waterui_core::{components::Text, env::use_env, AnyView, Environment};

use alloc::boxed::Box;
use waterui_reactive::{
    compute::{ComputeResult, ToComputed},
    watcher::WatcherGuard,
    Binding, Compute, Computed,
};

use crate::{
    color::{BackgroundColor, Color, ForegroundColor},
    component::{focu::Focused, navigation::NavigationView, Metadata},
    layout::{Edge, Frame},
    utils::{Id, Mapping},
};

pub trait ViewExt: View + Sized {
    fn metadata<T>(self, metadata: T) -> Metadata<T>;
    fn with<T: 'static>(self, value: T) -> impl View;
    fn title(self, title: impl Into<Text>) -> NavigationView;
    fn anyview(self) -> AnyView;
    fn padding(self) -> Metadata<Edge>;
    fn focused<T: 'static + Eq + Clone>(
        self,
        value: Binding<Option<T>>,
        equals: T,
    ) -> Metadata<Focused>;
    fn background(self, color: impl ToComputed<Color>) -> Metadata<BackgroundColor>;
    fn foreground(self, color: impl ToComputed<Color>) -> Metadata<ForegroundColor>;
    fn frame(self, frame: impl ToComputed<Frame>) -> Metadata<Computed<Frame>>;
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
    fn view(&self, env: Environment) -> impl View;
}

impl<F, V> ViewBuilder for F
where
    F: Fn(Environment) -> V + 'static,
    V: View,
{
    fn view(&self, env: Environment) -> impl View {
        (self)(env)
    }
}
pub struct AnyViewBuilder(Box<dyn Fn(Environment) -> AnyView>);

impl_debug!(AnyViewBuilder);

impl AnyViewBuilder {
    pub fn new(builder: impl ViewBuilder + 'static) -> Self {
        Self(Box::new(move |env| builder.view(env).anyview()))
    }
}

impl ViewBuilder for AnyViewBuilder {
    fn view(&self, env: Environment) -> impl View {
        (self.0)(env)
    }
}

impl<V: View> ViewExt for V {
    fn metadata<T>(self, metadata: T) -> Metadata<T> {
        Metadata::new(self, metadata)
    }

    fn with<T: 'static>(self, value: T) -> impl View {
        use_env(move |env| self.metadata(env.with(value)))
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
    fn frame(self, frame: impl ToComputed<Frame>) -> Metadata<Computed<Frame>> {
        Metadata::new(self, frame.to_computed())
    }

    fn background(self, color: impl ToComputed<Color>) -> Metadata<BackgroundColor> {
        Metadata::new(self, BackgroundColor::new(color))
    }
    fn foreground(self, color: impl ToComputed<Color>) -> Metadata<ForegroundColor> {
        Metadata::new(self, ForegroundColor::new(color))
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

impl<T, V> Compute for TaggedView<T, V>
where
    Self: ComputeResult,
{
    type Output = Self;
    fn compute(&self) -> Self::Output {
        self.clone()
    }
    fn watch(
        &self,
        _watcher: impl Into<waterui_reactive::watcher::Watcher<Self::Output>>,
    ) -> waterui_reactive::watcher::WatcherGuard {
        WatcherGuard::new(|| {})
    }
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
