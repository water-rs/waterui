use std::{
    any::{type_name, TypeId},
    collections::HashMap,
    fmt::{Debug, Display},
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::binding::BoxSubscriber;

#[derive(Debug, Clone)]
pub enum Alignment {
    Default,
    Leading,
    Center,
    Trailing,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum Size {
    Default,
    Px(u16),
    Percent(f64),
    Maximum(usize),
    Minimum(usize),
}

impl Default for Size {
    fn default() -> Self {
        Self::Default
    }
}

impl_from!(Size, u16, Px);

pub trait View: 'static + Reactive {
    fn view(&mut self) -> Box<dyn View>;

    fn frame(&self) -> crate::view::Frame;
    fn set_frame(&mut self, frame: crate::view::Frame);

    fn name(&self) -> &'static str {
        type_name::<Self>()
    }

    #[doc(hidden)]
    fn type_id(&self, _sealed: sealed::Sealed) -> TypeId {
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
        impl <$($ty:View,)*>IntoViews for ($($ty,)*){
            fn into_views(self) -> Vec<BoxView> {
                let ($($ty,)*)=self;;
                vec![$(Box::new($ty)),*]
            }
        }
    };
}

tuples!(impl_tuple_views);

impl View for () {
    fn view(&mut self) -> crate::view::BoxView {
        panic!("[Native implement]");
    }

    fn frame(&self) -> crate::view::Frame {
        Frame::default()
    }

    fn set_frame(&mut self, _frame: crate::view::Frame) {}
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

#[derive(Debug, Default, Clone)]
#[repr(C)]
pub struct Frame {
    pub width: Size,
    pub height: Size,
    pub margin: Edge,
}

#[derive(Debug, Default, Clone, PartialEq, PartialOrd)]
#[repr(C)]
pub struct Edge {
    pub top: Size,
    pub right: Size,
    pub bottom: Size,
    pub left: Size,
}

impl Edge {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn vertical(size: impl Into<Size>) -> Self {
        let size = size.into();
        Self::new().left(size.clone()).right(size)
    }

    pub fn horizontal(size: impl Into<Size>) -> Self {
        let size = size.into();

        Self::new().top(size.clone()).bottom(size)
    }

    pub fn round(size: impl Into<Size>) -> Self {
        let size = size.into();

        Self::new()
            .top(size.clone())
            .left(size.clone())
            .right(size.clone())
            .bottom(size)
    }

    pub fn top(mut self, size: impl Into<Size>) -> Self {
        self.top = size.into();
        self
    }

    pub fn left(mut self, size: impl Into<Size>) -> Self {
        self.left = size.into();
        self
    }
    pub fn right(mut self, size: impl Into<Size>) -> Self {
        self.right = size.into();
        self
    }
    pub fn bottom(mut self, size: impl Into<Size>) -> Self {
        self.bottom = size.into();
        self
    }
}

impl dyn View {
    pub fn inner_type_id(&self) -> TypeId {
        self.type_id(sealed::Sealed)
    }

    pub fn is<T: View>(&self) -> bool {
        self.inner_type_id() == TypeId::of::<T>()
    }

    pub fn downcast_ref<T: View>(&self) -> Option<&T> {
        if self.is::<T>() {
            unsafe { Some(&*(self as *const dyn View as *const T)) }
        } else {
            None
        }
    }

    pub fn downcast<T: View>(self: Box<Self>) -> Result<Box<T>, Box<dyn View>> {
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
    fn view(&mut self) -> Box<dyn View> {
        self.deref_mut().view()
    }

    fn frame(&self) -> crate::view::Frame {
        self.deref().frame()
    }

    fn set_frame(&mut self, frame: crate::view::Frame) {
        self.deref_mut().set_frame(frame)
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

impl<V: View> View for Box<V> {
    fn view(&mut self) -> Box<dyn View> {
        self.deref_mut().view()
    }

    fn frame(&self) -> crate::view::Frame {
        self.deref().frame()
    }

    fn set_frame(&mut self, frame: crate::view::Frame) {
        self.deref_mut().set_frame(frame)
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

pub struct Renderer<T> {
    map: HashMap<TypeId, Box<dyn Hook<T>>>,
}

pub trait Hook<T> {
    fn call_hook(&self, state: &mut T, render: &Renderer<T>, view: BoxView);
}

pub struct IntoHook<F, T, V> {
    f: F,
    _marker: PhantomData<(T, V)>,
}

impl<F, T, V> IntoHook<F, T, V> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _marker: PhantomData,
        }
    }
}

impl<F, T, V> Hook<T> for IntoHook<F, T, V>
where
    F: Fn(&mut T, &Renderer<T>, V),
    V: View,
{
    fn call_hook(&self, state: &mut T, renderer: &Renderer<T>, view: BoxView) {
        (self.f)(state, renderer, *view.downcast::<V>().unwrap());
    }
}

impl<T: 'static> Renderer<T> {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    pub fn add<F, V: View>(&mut self, hook: F)
    where
        F: Fn(&mut T, &Renderer<T>, V) + 'static,
    {
        self.map
            .insert(TypeId::of::<V>(), Box::new(IntoHook::new(hook)));
    }

    pub fn call(&self, view: BoxView, state: &mut T) {
        if let Some(hook) = self.map.get(&view.inner_type_id()) {
            hook.call_hook(state, self, view);
        } else {
            match view.downcast::<BoxView>() {
                Ok(v) => self.call(*v, state),
                Err(mut boxed) => self.call(boxed.view(), state),
            }
        }
    }
}
