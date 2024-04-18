use core::{
    any::{Any, TypeId},
    ops::Deref,
};

use alloc::{collections::BTreeMap, rc::Rc};

#[derive(Clone)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
    #[cfg(feature = "async")]
    executor: Executor,
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "async")]
mod executor {
    use alloc::rc::Rc;

    pub struct Executor {
        inner: Rc<smol::LocalExecutor<'static>>,
    }

    impl Default for Executor {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Executor {
        pub fn new() -> Self {
            Self {
                inner: Rc::new(smol::LocalExecutor::new()),
            }
        }

        pub fn spawn<Fut>(&self, fut: Fut) -> async_task::Task<Fut::Output>
        where
            Fut: core::future::Future + 'static,
            Fut::Output: 'static,
        {
            self.inner.spawn(fut)
        }

        pub async fn run(&self) {
            loop {
                self.inner.tick().await;
            }
        }
    }

    impl Clone for Executor {
        fn clone(&self) -> Self {
            Self {
                inner: self.inner.clone(),
            }
        }
    }
}

#[cfg(feature = "async")]
pub use executor::Executor;

impl Environment {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            #[cfg(feature = "async")]
            executor: Executor::new(),
        }
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>()).map(|rc| unsafe {
            let any = rc.deref();
            &*(any as *const dyn Any as *const T)
        })
    }

    #[cfg(feature = "async")]
    pub fn task<Fut>(&self, fut: Fut) -> async_task::Task<Fut::Output>
    where
        Fut: core::future::Future + 'static,
        Fut::Output: 'static,
    {
        self.executor.spawn(fut)
    }

    #[cfg(feature = "async")]
    pub fn executor(&self) -> Executor {
        self.executor.clone()
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Rc::new(value));
    }
}
