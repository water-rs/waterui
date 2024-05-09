mod padding;
pub use padding::Padding;

use crate::{Environment, View};

pub trait Modifier: 'static {
    fn modify(self, env: &Environment, view: impl View) -> impl View;
}

pub trait ModifierExt: Modifier + Sized {
    fn then<M>(self, modifier: M) -> And<Self, M>;
}

impl<M1: Modifier> ModifierExt for M1 {
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

impl<M1: Modifier, M2: Modifier> Modifier for And<M1, M2> {
    fn modify(self, env: &Environment, view: impl View) -> impl View {
        self.m2.modify(env, self.m1.modify(env, view))
    }
}
