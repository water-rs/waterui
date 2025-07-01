use alloc::rc::Rc;
use core::cell::RefCell;
use waterui_task::{LocalTask, StreamExt};

use crate::{
    Compute,
    watcher::{Metadata, Watcher, WatcherGuard, WatcherManager},
};

impl<S> Stream<S>
where
    S: waterui_task::Stream + Unpin + 'static,
    S::Item: Clone,
{
    /// Creates a new Stream with a default initial value.
    pub fn new(stream: S) -> Self
    where
        S::Item: Default,
    {
        Self::with_inital_value(stream, S::Item::default())
    }

    /// Creates a new Stream with an initial value.
    #[must_use]
    pub fn with_inital_value(stream: S, inital: S::Item) -> Self {
        Self {
            stream: Rc::new(RefCell::new(Some(stream))),
            buffer: Rc::new(RefCell::new(inital)),
            watchers: WatcherManager::default(),
        }
    }

    /// Launches the stream and starts listening for items.
    pub fn try_lanuch(&self) {
        if let Some(mut stream) = { self.stream.take() } {
            let buffer = self.buffer.clone();
            let watchers = self.watchers.clone();
            LocalTask::on_main(async move {
                while let Some(item) = stream.next().await {
                    *buffer.borrow_mut() = item.clone();
                    watchers.notify(move || item.clone(), &Metadata::new());
                }
            });
        }
    }
    #[must_use]
    /// Checks if the stream has been launched.
    pub fn is_lanuched(&self) -> bool {
        self.stream.borrow().is_some()
    }
}

type Buffer<T> = Rc<RefCell<T>>;

/// A reactive stream wrapper that manages a buffer and notifies watchers on updates.
#[derive(Debug)]
pub struct Stream<S>
where
    S: waterui_task::Stream + Unpin + 'static,
    S::Item: Clone,
{
    #[allow(clippy::struct_field_names)]
    stream: Buffer<Option<S>>,
    buffer: Buffer<S::Item>,
    watchers: WatcherManager<S::Item>,
}

impl<S> Clone for Stream<S>
where
    S: waterui_task::Stream + Unpin + 'static,
    S::Item: Clone,
{
    fn clone(&self) -> Self {
        Self {
            stream: self.stream.clone(),
            buffer: self.buffer.clone(),
            watchers: self.watchers.clone(),
        }
    }
}

impl<S> Compute for Stream<S>
where
    S: waterui_task::Stream + Unpin + 'static,
    S::Item: Clone,
{
    type Output = S::Item;
    fn compute(&self) -> Self::Output {
        self.try_lanuch();
        self.buffer.borrow().clone()
    }
    fn add_watcher(&self, watcher: impl Watcher<S::Item>) -> crate::watcher::WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}

/// Creates a new reactive stream from a waterui task stream.
pub fn stream<S>(s: S) -> Stream<S>
where
    S: waterui_task::Stream + Unpin + 'static,
    S::Item: Clone + Default,
{
    Stream::new(s)
}
