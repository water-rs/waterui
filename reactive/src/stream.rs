use alloc::rc::Rc;
use core::{cell::RefCell, ops::AddAssign};
use waterui_task::{LocalTask, StreamExt, Task};

use crate::{
    Compute,
    
    watcher::{WatcherGuard, WatcherManager},
};

struct StreamInner<S, T, B> {
    stream: Option<S>,
    buffer: T,
    behavior: B,
    watchers: WatcherManager<T>,
}

impl<S, T, B> StreamInner<S, T, B> {
    pub fn try_lanuch(&mut self) {
        if let Some(mut stream) = { self.stream.take() } {
            let this = self.inner.clone();
            let watchers = self.watchers.clone();
            LocalTask::on_main(async move {
                while let Some(item) = stream.next().await {
                    this.borrow_mut().buffer = item.clone();
                    watchers.notify(item);
                }
            });
        }
    }
    pub fn push(&self) {}
}

trait StreamBehavior<T> {
    fn buffer(old: &mut T, new: T);
}

struct Replace;
struct Append;

impl<T> StreamBehavior<T> for Replace {
    fn buffer(old: &mut T, new: T) {
        *old = new;
    }
}

impl<T: AddAssign> StreamBehavior<T> for Append {
    fn buffer(old: &mut T, new: T) {
        old.add_assign(new);
    }
}

pub struct Stream<S, T>(Rc<RefCell<StreamInner<S, T, Replace>>>);

impl<S, T> Clone for Stream<S, T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<S, T> Compute for Stream<S, T>
where
    S: waterui_task::Stream<Item = T> + 'static,
    T + Default,
{
    type Output = S::Item;
    fn compute(&self) -> Self::Output {
        self.inner.borrow().buffer.clone()
    }
    fn watch(
        &self,
        watcher: crate::watcher::Watcher<Self::Output>,
    ) -> crate::watcher::WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}
