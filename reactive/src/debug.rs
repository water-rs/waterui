use core::any::type_name;
use std::rc::Rc;

use crate::{
    Compute,
    watcher::{Watcher, WatcherGuard},
};

#[derive(Debug, Clone)]
pub struct Debug<C> {
    source: C,
    inner: Rc<DebugInner>,
}

#[derive(Debug)]
struct DebugInner {
    #[allow(unused)]
    guard: WatcherGuard,
    config: Config,
}

impl<C> Debug<C>
where
    C: Compute,
    C::Output: core::fmt::Debug,
{
    pub fn with_config(source: C, config: Config) -> Self {
        let name = type_name::<C>();
        let guard = if config.change {
            source.watch(move |value, metadata: crate::watcher::Metadata| {
                if metadata.is_empty() {
                    log::info!("`{name}` changed to {value:?}")
                } else {
                    log::info!("`{name}` changed to {value:?} with metadata {metadata:?}")
                }
            })
        } else {
            WatcherGuard::new(|| {})
        };

        Self {
            source,
            inner: Rc::new(DebugInner { guard, config }),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    compute: bool,
    watch: bool,
    remove_watcher: bool,
    change: bool,
}

impl<C> Compute for Debug<C>
where
    C: Compute,
    C::Output: core::fmt::Debug,
{
    type Output = C::Output;
    fn compute(&self) -> Self::Output {
        let name = type_name::<C>();
        let value = self.source.compute();
        if self.inner.config.compute {
            log::debug!("`{name}` computed value {value:?}");
        }
        value
    }
    fn watch(&self, watcher: impl Watcher<C::Output>) -> crate::watcher::WatcherGuard {
        let mut guard = self.source.watch(watcher);
        if self.inner.config.watch {
            log::debug!("Added watcher");
        }
        if self.inner.config.remove_watcher {
            guard = guard.on_drop(|| {
                log::debug!("Removed watcher");
            })
        }
        guard
    }
}
