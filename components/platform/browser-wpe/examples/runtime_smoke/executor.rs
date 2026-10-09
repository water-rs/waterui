use std::ops::Deref;
use std::rc::Rc;

use executor_core::LocalExecutor;
use executor_core::async_executor::AsyncLocalExecutor;
use waterui_browser_wpe::{WpePage, WpeRuntime};

#[derive(Clone, Debug)]
pub struct SmokeExecutor(Rc<AsyncLocalExecutor<'static>>);

impl LocalExecutor for SmokeExecutor {
    type Task<T: 'static> = <AsyncLocalExecutor<'static> as LocalExecutor>::Task<T>;

    fn spawn_local<Fut>(&self, future: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        self.0.spawn_local(future)
    }
}

impl SmokeExecutor {
    pub fn install() -> Self {
        let executor = Self(Rc::new(AsyncLocalExecutor::new()));
        executor_core::init_local_executor(executor.clone());
        executor
    }

    fn tick(&self) {
        self.0.try_tick();
    }
}

#[derive(Clone)]
pub struct SmokePage {
    page: WpePage,
    executor: SmokeExecutor,
}

impl SmokePage {
    pub fn new(runtime: WpeRuntime, executor: &SmokeExecutor) -> Self {
        Self {
            page: WpePage::new(runtime),
            executor: executor.clone(),
        }
    }

    pub fn pump(&self) {
        self.page.pump();
        self.executor.tick();
    }
}

impl Deref for SmokePage {
    type Target = WpePage;

    fn deref(&self) -> &Self::Target {
        &self.page
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn installed_executor_drives_spawn_local() {
        let executor = SmokeExecutor::install();
        let completed = Rc::new(Cell::new(false));
        let result = Rc::clone(&completed);
        executor_core::spawn_local(async move { result.set(true) }).detach();
        assert!(!completed.get());
        executor.tick();
        assert!(completed.get());
    }
}
