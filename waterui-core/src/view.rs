use std::{
    any::{type_name, TypeId},
    collections::HashMap,
    fmt::Debug,
    marker::PhantomData,
    ops::Deref,
};

use serde::{Deserialize, Serialize};

use crate::binding::BoxSubscriber;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[repr(u8)]
pub enum Alignment {
    Default,
    Leading,
    Center,
    Trailing,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[repr(C)]
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

pub trait View: Reactive {
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

pub trait ViewBuilder<V: View, T> {
    fn build(&self, context: T) -> V;
}

macro_rules! impl_view_builder {
    ($($ty:ident),*) => {
        #[allow(non_snake_case)]
        #[allow(unused_variables)]
        impl<F,V:View+'static,$($ty,)*> ViewBuilder<V,($($ty,)*)> for F
        where
            F: Fn($($ty,)*) -> V,
        {
            #[allow(unused_variables)]
            fn build(&self, context: ($($ty,)*)) -> V {
                let ($($ty,)*)=context;
                (self)($($ty,)*)
            }
        }

    };
}

tuples!(impl_view_builder);

impl<F, V: View + 'static> ViewBuilder<V, ()> for F
where
    F: Fn() -> V,
{
    fn build(&self, _context: ()) -> V {
        (self)()
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

impl View for () {
    fn view(&self) -> crate::view::BoxView {
        panic!("[Native implement]");
    }
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

#[derive(Debug, Default, Clone, Deserialize, Serialize, PartialEq)]
#[repr(C)]

pub struct Frame {
    pub width: Size,
    pub height: Size,
    pub margin: Edge,
}

#[derive(Debug, Default, Clone, PartialEq, Deserialize, Serialize)]
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

pub type BoxView = Box<dyn View>;

impl View for BoxView {
    fn view(&self) -> Box<dyn View> {
        self.deref().view()
    }
}

impl<V: View> View for &V {
    fn view(&self) -> Box<dyn View> {
        (*self).view()
    }
}

impl<V: Reactive> Reactive for &V {
    fn is_reactive(&self) -> bool {
        (*self).is_reactive()
    }

    fn subscribe(&self, subscriber: fn() -> BoxSubscriber) {
        (*self).subscribe(subscriber)
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
    fn view(&self) -> Box<dyn View> {
        self.deref().view()
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

pub struct RendererBuilder<State, LocalState: Default, Message> {
    map: HashMap<TypeId, Box<dyn Hook<State, LocalState, Message>>>,
    global_hook_before: fn(&mut State, &mut LocalState, &Message),
    global_hook_after: fn(&mut State, &mut LocalState),

    _marker: PhantomData<LocalState>,
}

pub trait Hook<State, LocalState: Default, Message> {
    fn call_hook(
        &self,
        view: BoxView,
        state: &mut State,
        local_state: &mut LocalState,
        message: Message,

        renderer: &RendererBuilder<State, LocalState, Message>,
    );
}

pub struct IntoHook<F, V> {
    f: F,
    _marker: PhantomData<V>,
}

impl<F, V> IntoHook<F, V> {
    pub fn new(f: F) -> Self {
        Self {
            f,
            _marker: PhantomData,
        }
    }
}

impl<F, State, LocalState: Default, Message, V> Hook<State, LocalState, Message> for IntoHook<F, V>
where
    F: Fn(V, &mut State, &mut LocalState, Message, &RendererBuilder<State, LocalState, Message>),
    V: View + 'static,
{
    fn call_hook(
        &self,
        view: BoxView,
        state: &mut State,
        local_state: &mut LocalState,
        message: Message,
        renderer: &RendererBuilder<State, LocalState, Message>,
    ) {
        (self.f)(
            *view.downcast::<V>().unwrap(),
            state,
            local_state,
            message,
            renderer,
        );
    }
}

impl Default for RendererBuilder<(), (), ()> {
    fn default() -> Self {
        Self::new()
    }
}

impl<State: 'static, LocalState: Default, Message> RendererBuilder<State, LocalState, Message> {
    pub fn new() -> Self {
        let mut renderer = Self {
            map: HashMap::new(),
            global_hook_before: |_, _, _| {},
            global_hook_after: |_, _| {},

            _marker: PhantomData,
        };
        renderer.add(|_: (), _, _, _, _| {});
        renderer
    }

    pub fn global_hook_before(mut self, f: fn(&mut State, &mut LocalState, &Message)) -> Self {
        self.global_hook_before = f;
        self
    }

    pub fn global_hook_after(mut self, f: fn(&mut State, &mut LocalState)) -> Self {
        self.global_hook_after = f;
        self
    }

    pub fn add<F, V: View + 'static>(&mut self, hook: F)
    where
        F: Fn(
                V,
                &mut State,
                &mut LocalState,
                Message,
                &RendererBuilder<State, LocalState, Message>,
            ) + 'static,
    {
        self.map
            .insert(TypeId::of::<V>(), Box::new(IntoHook::new(hook)));
    }

    pub fn call_with_message(&self, view: BoxView, state: &mut State, message: Message) {
        let mut local_state = LocalState::default();
        (self.global_hook_before)(state, &mut local_state, &message);
        if let Some(hook) = self.map.get(&view.inner_type_id()) {
            hook.call_hook(view, state, &mut local_state, message, self);
        } else {
            match view.downcast::<BoxView>() {
                Ok(v) => self.call_with_message(*v, state, message),
                Err(boxed) => self.call_with_message(boxed.view(), state, message),
            }
        }
        (self.global_hook_after)(state, &mut local_state);
    }

    pub fn call(&self, view: BoxView, state: &mut State)
    where
        Message: Default,
    {
        self.call_with_message(view, state, Message::default())
    }
}
