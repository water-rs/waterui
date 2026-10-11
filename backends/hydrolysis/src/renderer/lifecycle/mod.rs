// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

pub mod lazy;

pub use lazy::*;

pub struct DeferredLifeCycleHook {
    pub(crate) env: Environment,
    pub(crate) hook: LifeCycleHook,
}

impl DeferredLifeCycleHook {
    pub(crate) const fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    pub(crate) fn call(self) {
        self.hook.handle(&self.env);
    }
}
