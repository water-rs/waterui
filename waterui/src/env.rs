use core::{
    any::{Any, TypeId},
    ops::Deref,
};

use alloc::{collections::BTreeMap, rc::Rc};

#[cfg(feature = "async")]
pub(crate) type SharedExecutor = Rc<smol::LocalExecutor<'static>>;

#[derive(Clone)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
    #[cfg(feature = "async")]
    executor: SharedExecutor,
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

impl Environment {
    #[cfg(feature = "async")]
    pub fn new() -> Self {
        let executor = smol::LocalExecutor::new();
        Self {
            map: BTreeMap::new(),
            executor: Rc::new(executor),
        }
    }

    #[cfg(not(feature = "async"))]
    pub const fn new() -> Self {
        Self {
            map: BTreeMap::new(),
        }
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>()).map(|rc| unsafe {
            let any = rc.deref();
            &*(any as *const dyn Any as *const T)
        })
    }

    #[cfg(feature = "async")]
    pub fn task<Fut>(&self, fut: Fut) -> smol::Task<Fut::Output>
    where
        Fut: core::future::Future + 'static,
        Fut::Output: 'static,
    {
        self.executor.spawn(fut)
    }

    #[cfg(feature = "async")]
    pub(crate) fn executor(&self) -> Rc<smol::LocalExecutor<'static>> {
        self.executor.clone()
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Rc::new(value));
    }
}
