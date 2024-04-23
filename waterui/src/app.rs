use core::future::Future;

use crate::{AnyView, Environment, View, ViewExt};

pub struct App {
    pub _content: AnyView,
    pub _env: Environment,
}

impl App {
    pub fn new(content: impl View + 'static) -> Self {
        Self {
            _content: content.anyview(),
            _env: Environment::new(),
        }
    }

    pub fn run<F, Fut>(self, start: F)
    where
        F: FnOnce(Self) -> Fut,
        Fut: Future<Output = ()> + 'static,
    {
        let env = self._env.clone();
        env.task(start(self)).detach();
        smol::block_on(env.executor().run());
    }
}
