use super::*;

#[derive(Default)]
pub(crate) struct LifecycleState {
    pub(crate) current_frame_retain: Vec<Retain>,
    pub(crate) previous_frame_retain: Vec<Retain>,
}

pub(crate) struct DeferredLifeCycleHook {
    pub(crate) env: Environment,
    pub(crate) hook: LifeCycleHook,
}

impl LifecycleState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.previous_frame_retain.clear();
        self.current_frame_retain.clear();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.previous_frame_retain = core::mem::take(&mut self.current_frame_retain);
    }
}

impl DeferredLifeCycleHook {
    pub(crate) fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    pub(crate) fn call(self) {
        self.hook.handle(&self.env);
    }
}
