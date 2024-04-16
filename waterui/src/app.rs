extern crate std;

use crate::{component::AnyView, Environment, View, ViewExt};

pub struct App {
    content: AnyView,
    env: Environment,
}

impl App {
    pub fn new(cntent: impl View + 'static) -> Self {
        Self {
            content: cntent.anyview(),
            env: Environment::new(),
        }
    }

    pub fn env<T: 'static>(mut self, value: T) -> Self {
        self.env.insert(value);
        self
    }

    #[cfg(feature = "async")]
    pub fn run(self, runtime: impl FnOnce(AnyView, Environment)) {
        let executor = self.env.executor();
        runtime(self.content, self.env);

        smol::block_on(async move {
            loop {
                executor.tick().await;
            }
        })
    }

    #[cfg(not(feature = "async"))]
    pub fn run(self, runtime: impl FnOnce(AnyView, Environment)) {
        runtime(self.content, self.env);
    }
}
