use std::cell::RefCell;
use std::ops::Deref;
use std::rc::Rc;
use std::time::Instant;

use executor_core::LocalExecutor;
use executor_core::async_executor::AsyncLocalExecutor;
use waterui_browser_wpe::{WpePage, WpeRuntime};
use waterui_webview::{BackendEvent, WebViewEvent};

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

    /// Pumps the page until `future` completes, panicking with `purpose` once
    /// `deadline` passes.
    pub fn block_on<T: 'static>(
        &self,
        future: impl Future<Output = T> + 'static,
        deadline: Instant,
        purpose: &str,
    ) -> T {
        let output = Rc::new(RefCell::new(None));
        let slot = Rc::clone(&output);
        executor_core::spawn_local(async move {
            slot.replace(Some(future.await));
        })
        .detach();
        loop {
            self.pump();
            if let Some(output) = output.borrow_mut().take() {
                return output;
            }
            assert!(
                Instant::now() < deadline,
                "WPE smoke timed out waiting for {purpose}"
            );
            std::thread::yield_now();
        }
    }

    /// Loads `url` and pumps the page until the engine reports it loaded,
    /// panicking on a load error or once `deadline` passes.
    pub fn load(&self, url: &str, deadline: Instant) {
        let loaded = Rc::new(RefCell::new(None::<Result<(), String>>));
        // The guard has to outlive the pump loop below: dropping it
        // unsubscribes, and the run would then wait for a `Loaded` it can no
        // longer observe until it times out.
        let _watcher = self.page.watch({
            let loaded = Rc::clone(&loaded);
            move |event| match event {
                BackendEvent::Event(WebViewEvent::Loaded) => {
                    loaded.replace(Some(Ok(())));
                }
                BackendEvent::Event(WebViewEvent::Error(error)) => {
                    loaded.replace(Some(Err(format!("{error:?}"))));
                }
                _ => {}
            }
        });
        self.page.load_uri(url);
        loop {
            self.pump();
            if let Some(outcome) = loaded.borrow_mut().take() {
                if let Err(error) = outcome {
                    panic!("WPE smoke navigation to {url} failed: {error}");
                }
                return;
            }
            assert!(
                Instant::now() < deadline,
                "WPE smoke timed out loading {url}"
            );
            std::thread::yield_now();
        }
    }
}

impl Deref for SmokePage {
    type Target = WpePage;

    fn deref(&self) -> &Self::Target {
        &self.page
    }
}
