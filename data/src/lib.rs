#![no_std]
extern crate alloc;
pub mod database;
use alloc::{rc::Rc, vec::Vec};
use core::{any::type_name, cell::RefCell, future::Future, marker::PhantomData};
use serde::{Serialize, de::DeserializeOwned};
use waterui::{compute::ComputeResult, id::Identifable};
use waterui_reactive::{
    Compute,
    binding::CustomBinding,
    collection::Collection,
    watcher::{WatcherGuard, WatcherManager},
};

use crate::database::{Database, DefaultDatabase};
pub struct Data<T: Schema> {
    database: DefaultDatabase,
    buf: RefCell<Vec<DataElement<T>>>,
    _marker: PhantomData<T>,
}

pub trait Schema: Serialize + DeserializeOwned + ComputeResult {}

pub type Id = u64;

impl<T: Schema> Collection for Data<T> {
    type Item = DataElement<T>;

    fn get(&self, index: usize) -> Option<Self::Item> {
        let buf = self.buf.borrow();
        if ((buf.len() as isize) - index as isize) < 10 {
            self.pull_data();
        }
        buf.get(index).cloned()
    }

    fn remove(&self, index: usize) {
        let buf = self.buf.borrow_mut();
        if let Some(index) = buf.get(index) {
            self.database.remove(index.id);
        }
    }

    fn len(&self) -> usize {
        self.buf.borrow().len()
    }

    fn watch(&self, watcher: waterui_reactive::watcher::Watcher<()>) -> WatcherGuard {
        self.database.on_change(move || {
            watcher.notify(());
        })
    }
}

impl<T: Schema> Data<T> {
    pub async fn load() -> Self {
        let name = type_name::<T>();
        let database = DefaultDatabase::open(name.as_bytes()).await;
        Self {
            database,
            buf: RefCell::default(),
            _marker: PhantomData,
        }
    }

    pub fn by_id() {}

    pub fn append(&self, value: T) {
        let ele = DataElement::create(value);
        //sself.engine.insert(ele.id, ele.);
    }

    pub fn delete(&self, id: Id) {}

    fn pull_data(&self) {
        todo!()
    }

    pub fn filter(self, filter: impl Filter) -> Self {
        todo!()
    }

    pub fn sort(&self) {}
}

pub trait Filter {
    fn filter() {}
}

pub struct Equals {}

type SharedValue<T> = Rc<RefCell<T>>;

#[derive(Debug, Clone)]
pub struct DataElement<T> {
    id: Id,
    database: DefaultDatabase,
    value: SharedValue<T>,
    watchers: WatcherManager<T>,
}

impl<T> Identifable for DataElement<T> {
    type Id = Id;
    fn id(&self) -> Self::Id {
        self.id
    }
}

impl<T: Schema> DataElement<T> {
    async fn new(data: &Data<T>, id: Id) -> Self {
        let database = data.database.clone();
        let watchers = WatcherManager::default();
        let key = id.to_be_bytes();
        let value: T = decode(database.by_id(id).await.unwrap());
        let shared = Rc::new(RefCell::new(value));
        {
            let watchers = watchers.clone();
            let shared = shared.clone();
            database.watch(&key, move |data| {
                let value: T = decode(data);
                *shared.borrow_mut() = value.clone();
                watchers.notify(value);
            });
        }

        Self {
            database,
            id,
            value: shared,
            watchers,
        }
    }

    fn create(data: &Data<T>, value: T) -> Self {
        let database = &data.database;
        let id = database.generate_id();
        database.insert(id, encode(value));
        todo!()
    }
}

impl<T: Schema> Compute for DataElement<T> {
    type Output = T;

    fn compute(&self) -> Self::Output {
        self.value.borrow().clone()
    }

    fn watch(&self, watcher: waterui_reactive::watcher::Watcher<Self::Output>) -> WatcherGuard {
        WatcherGuard::from_id(&self.watchers, self.watchers.register(watcher))
    }
}

impl<T: Schema> CustomBinding for DataElement<T> {
    fn set(&self, value: T) {
        let data = encode(&value);
        self.database.insert(&self.id.to_be_bytes(), data);
    }
}

fn decode<T: DeserializeOwned>(data: impl AsRef<[u8]>) -> T {
    rmp_serde::from_slice(data.as_ref()).unwrap()
}

fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    rmp_serde::to_vec_named(value).unwrap()
}
