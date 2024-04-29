use core::{
    any::{Any, TypeId},
    future::Future,
};

use alloc::{boxed::Box, collections::BTreeMap, rc::Rc};

pub struct EnvironmentInner {
    #[cfg(feature = "async")]
    executor: Executor,
    default_view: DefaultView,
}

#[derive(Clone)]
pub struct Environment {
    map: BTreeMap<TypeId, Rc<dyn Any>>,
    inner: Rc<EnvironmentInner>,
}

pub struct DefaultView {
    error: Box<dyn Fn(BoxedStdError) -> AnyView>,
    loading: Box<dyn Fn() -> AnyView>,
}

impl Default for DefaultView {
    fn default() -> Self {
        Self {
            error: Box::new(|_| AnyView::new(())),
            loading: Box::new(|| AnyView::new(())),
        }
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "async")]
mod executor {

    pub struct Executor {
        inner: smol::LocalExecutor<'static>,
    }

    impl Default for Executor {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Executor {
        pub fn new() -> Self {
            Self {
                inner: smol::LocalExecutor::new(),
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
}

#[cfg(feature = "async")]
pub use executor::Executor;

use crate::{error::BoxedStdError, AnyView};

impl Environment {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),

            inner: Rc::new(EnvironmentInner {
                #[cfg(feature = "async")]
                executor: Executor::new(),
                default_view: DefaultView::default(),
            }),
        }
    }

    pub fn insert<T: 'static>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Rc::new(value));
    }

    pub fn get<T: 'static>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())
            .and_then(|v| v.downcast_ref::<T>())
    }
    pub fn default_error_view(&self, error: BoxedStdError) -> AnyView {
        (self.inner.default_view.error)(error)
    }

    pub fn default_loading_view(&self) -> AnyView {
        (self.inner.default_view.loading)()
    }

    #[cfg(feature = "async")]
    pub fn task<Fut>(&self, fut: Fut) -> async_task::Task<Fut::Output>
    where
        Fut: Future + 'static,
        Fut::Output: 'static,
    {
        self.inner.executor.spawn(fut)
    }

    #[cfg(feature = "async")]
    pub fn executor(&self) -> &Executor {
        &self.inner.executor
    }
}
