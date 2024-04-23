mod padding;
pub use padding::Padding;
mod task;
pub(crate) use task::Task;
pub mod with;
pub use with::WithValue;

use waterui_view::{Environment, View};

pub trait Modifer {
    fn modify(self, env: Environment, view: impl View + 'static) -> impl View + 'static;
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
    fn modify(self, env: Environment, view: impl View + 'static) -> impl View + 'static {
        self.m2.modify(env.clone(), self.m1.modify(env, view))
    }
}
