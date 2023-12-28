use std::{
    any::{Any, TypeId},
    collections::HashMap,
    sync::Arc,
};

pub struct Environment {
    map: Arc<EnvironmentBuilder>,
}

impl Clone for Environment {
    fn clone(&self) -> Self {
        Self {
            map: self.map.clone(),
        }
    }
}

pub struct EnvironmentBuilder {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl EnvironmentBuilder {
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .map(|any| unsafe { &*(any as *const dyn Any as *const T) })
    }

    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())
            .map(|any| unsafe { &mut *(any as *mut dyn Any as *mut T) })
    }

    pub fn insert<T: Send + Sync + 'static>(&mut self, value: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(value))
            .map(|any| unsafe {
                let any: *mut dyn Any = Box::into_raw(any);
                let boxed: Box<T> = Box::from_raw(any as *mut T);
                *boxed
            })
    }
}

impl Environment {
    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get()
    }
    /// Constructs an `Environment` from a raw pointer
    /// # Safety
    /// The raw pointer must have been previously returned by a call to Environment::into_raw
    pub unsafe fn from_raw(ptr: *const EnvironmentBuilder) -> Self {
        Self {
            map: Arc::from_raw(ptr),
        }
    }

    /// Consumes the Environment, returning the wrapped pointer.
    /// To avoid a memory leak the pointer must be converted back to an Environment using Environment::from_raw.
    pub fn into_raw(self) -> *const EnvironmentBuilder {
        Arc::into_raw(self.map)
    }
}
