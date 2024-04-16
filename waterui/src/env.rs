use core::any::{Any, TypeId};

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};

pub struct Environment {
    inner: Rc<EnvironmentBuilder>,
}

impl Clone for Environment {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug, Default)]
pub struct EnvironmentBuilder {
    map: BTreeMap<TypeId, Box<dyn Any>>,
    #[cfg(feature = "async")]
    executor: async_executor::LocalExecutor<'static>,
}

impl EnvironmentBuilder {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn insert<T: 'static>(&mut self, value: T) -> Option<T> {
        self.map
            .insert(TypeId::of::<T>(), Box::new(value))
            .map(|any| unsafe {
                let any: *mut dyn Any = Box::into_raw(any);
                let boxed: Box<T> = Box::from_raw(any as *mut T);
                *boxed
            })
    }

    pub fn build(self) -> Environment {
        Environment::new(self)
    }
}

impl Environment {
    fn new(builder: EnvironmentBuilder) -> Self {
        Self {
            inner: Rc::new(builder),
        }
    }

    #[cfg(feature = "async")]
    pub fn task<Fut>(&self, fut: Fut) -> async_executor::Task<Fut::Output>
    where
        Fut: core::future::Future + 'static,
        Fut::Output: 'static,
    {
        self.inner.executor.spawn(fut)
    }

    pub fn builder() -> EnvironmentBuilder {
        EnvironmentBuilder::new()
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.inner.get()
    }
    /// Constructs an `Environment` from a raw pointer
    /// # Safety
    /// The raw pointer must have been previously returned by a call to Environment::into_raw
    pub unsafe fn from_raw(ptr: *const EnvironmentBuilder) -> Self {
        Self {
            inner: Rc::from_raw(ptr),
        }
    }

    /// Consumes the Environment, returning the wrapped pointer.
    /// To avoid a memory leak the pointer must be converted back to an Environment using Environment::from_raw.
    pub fn into_raw(self) -> *const EnvironmentBuilder {
        Rc::into_raw(self.inner)
    }
}
