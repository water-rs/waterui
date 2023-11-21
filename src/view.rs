use std::{
    any::{type_name, TypeId},
    collections::HashMap,
    fmt::Debug,
    marker::PhantomData,
};

use crate::{component::TapGesture, Event};

pub trait View: 'static {
    fn view(&self) -> Box<dyn View>;
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }

    #[doc(hidden)]
    fn type_id(&self, _sealed: sealed::Sealed) -> TypeId {
        TypeId::of::<Self>()
    }
}

mod sealed {
    pub struct Sealed;
}

pub trait ViewExt {
    fn on_tap(self, event: impl Event) -> TapGesture;
}

impl<V: View> ViewExt for V {
    fn on_tap(self, event: impl Event) -> TapGesture {
        TapGesture::new(Box::new(self), Box::new(event))
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

pub struct AnyView(BoxView);

impl AnyView {
    pub fn into_inner(self) -> BoxView {
        self.0
    }
}

impl Debug for dyn View {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("dyn View")
    }
}

native_implement!(AnyView);

pub fn downcast_view<T: View>(mut view: BoxView) -> Result<T, BoxView> {
    match view.downcast::<T>() {
        Ok(v) => return Ok(*v),
        Err(boxed) => view = boxed,
    }

    match view.downcast::<AnyView>() {
        Ok(v) => downcast_view(v.into_inner()),
        Err(boxed) => Err(boxed),
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
            println!("this path2");

            self.call(view.view(), state);
        }
    }
}
