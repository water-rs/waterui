use core::future::Future;

use waterui_view::{Environment, View};

use super::Modifier;

pub(crate) struct Task<Fut> {
    fut: Fut,
}

impl<Fut> Task<Fut> {
    pub fn new(fut: Fut) -> Self {
        Self { fut }
    }
}

impl<Fut> Modifier for Task<Fut>
where
    Fut: Future + 'static,
    Fut::Output: 'static,
{
    fn modify(self, env: Environment, view: impl View) -> impl View {
        env.task(self.fut).detach();
        view
    }
}
