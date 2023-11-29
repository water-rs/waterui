use std::marker::PhantomData;

use crate::{
    ffi::FFIWindowManager,
    view::{BoxView, View},
    ViewExt,
};

#[derive(Debug, Clone)]
pub struct Window<Manager: WindowManager = GlobalManager> {
    id: usize,
    _marker: PhantomData<Manager>,
}

pub trait WindowManager: Send + Sync {
    fn create(view: BoxView) -> usize;

    fn close(id: usize);
}

type GlobalManager = FFIWindowManager;

impl<Manager: WindowManager> Window<Manager> {
    pub fn new(view: impl View + 'static) -> Self {
        Self::from_raw(Manager::create(view.into_boxed()))
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn from_raw(id: usize) -> Self {
        Self {
            id,
            _marker: PhantomData,
        }
    }

    pub fn close(self) {
        Manager::close(self.id)
    }
}
