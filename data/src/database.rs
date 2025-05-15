use core::future::Future;

use alloc::vec::Vec;
use waterui_reactive::watcher::WatcherGuard;

use crate::Id;

pub trait Database: Clone + Default {
    fn open(name: &[u8]) -> Self;
    fn get(&self, index: usize) -> impl Future<Output = Option<Id>>;
    fn len(&self) -> impl Future<Output = usize>;
    fn is_empty(&self) -> impl Future<Output = bool> {
        async { self.len().await == 0 }
    }
    fn by_id(&self, id: Id) -> impl Future<Output = Option<&[u8]>>;
    fn generate_id(&self) -> Id;
    fn insert(&self, key: &[u8], value: Vec<u8>);
    fn remove(&self, id: Id);

    fn add_watcher(&self, key: &[u8], watcher: impl Fn(&[u8]));
    fn on_change(&self, watcher: impl Fn()) -> WatcherGuard;
}

#[derive(Debug, Clone, Default)]
pub struct DefaultDatabase {}

impl Database for DefaultDatabase {
    fn open(name: &[u8]) -> Self {
        todo!()
    }

    async fn get(&self, index: usize) -> Option<Id> {
        todo!()
    }

    async fn len(&self) -> usize {
        todo!()
    }

    async fn by_id(&self, id: Id) -> Option<&[u8]> {
        todo!()
    }

    fn generate_id(&self) -> Id {
        todo!()
    }

    fn insert(&self, key: &[u8], value: Vec<u8>) {
        todo!()
    }

    fn remove(&self, id: Id) {
        todo!()
    }

    fn add_watcher(&self, key: &[u8], watcher: impl Fn(&[u8])) {
        todo!()
    }

    fn on_change(&self, watcher: impl Fn()) -> WatcherGuard {
        todo!()
    }
}
