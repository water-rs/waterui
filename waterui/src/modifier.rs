use core::future::Future;

use waterui_view::{Environment, View};

pub trait Modifer {
    fn modify(self, env: Environment, view: impl View) -> impl View;
}

pub trait ModiferExt: Modifer + Sized {
    fn then<M>(self, modifier: M) -> And<Self, M>;
}

impl<M1: Modifer> ModiferExt for M1 {
    fn then<M2>(self, m2: M2) -> And<Self, M2> {
        And::new(self, m2)
    }
}

pub struct And<M1, M2> {
    m1: M1,
    m2: M2,
}

impl<M1, M2> And<M1, M2> {
    pub fn new(m1: M1, m2: M2) -> Self {
        Self { m1, m2 }
    }
}

impl<M1: Modifer, M2: Modifer> Modifer for And<M1, M2> {
    fn modify(self, env: Environment, view: impl View) -> impl View {
        self.m2.modify(env.clone(), self.m1.modify(env, view))
    }
}

pub(crate) struct Task<Fut> {
    fut: Fut,
}

impl<Fut> Task<Fut> {
    pub fn new(fut: Fut) -> Self {
        Self { fut }
    }
}

impl<Fut> Modifer for Task<Fut>
where
    Fut: Future + 'static,
    Fut::Output: 'static,
{
    fn modify(self, env: Environment, view: impl View) -> impl View {
        env.task(self.fut).detach();
        view
    }
}
