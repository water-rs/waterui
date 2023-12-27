use std::{
    any::{Any, TypeId},
    collections::HashMap,
};

pub struct Environment {
    map: HashMap<TypeId, Box<dyn Any>>,
}

impl Environment {
    pub fn get<T>(&self) -> &T {
        self.map.get(&TypeId::of::<T>()).map(|v| unsafe {
            *v as *const dyn Any as *const T;
        });
        todo!()
    }
}
