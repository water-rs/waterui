use std::{any::TypeId, collections::HashMap};

use waterui_core::{AnyView, Environment, View};

type Handler<T> = Box<dyn FnMut(&mut T, AnyView)>;

pub struct ViewDispatcher<T> {
    state: T,
    map: HashMap<TypeId, Handler<T>>,
}

impl<T> ViewDispatcher<T> {
    pub fn new(state: T) -> Self {
        let mut map = Self {
            state,
            map: HashMap::new(),
        };

        map.insert(|_, _: ()| {});

        map
    }
    pub fn call(&mut self, view: AnyView, env: Environment) {
        let id = view.type_id();
        if let Some(handler) = self.map.get_mut(&id) {
            handler(&mut self.state, view);
        } else {
            self.call(AnyView::new(view.body(env.clone())), env)
        }
    }

    pub fn insert<V: View>(&mut self, mut handler: impl FnMut(&mut T, V) + 'static) {
        self.map.insert(
            TypeId::of::<V>(),
            Box::new(move |state, view| handler(state, *view.downcast().unwrap())),
        );
    }
}
