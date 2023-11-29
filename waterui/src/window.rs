use std::sync::{Mutex, RwLock};

use waterui_core::view::Frame;

use crate::{
    view::{BoxView, View},
    ViewExt,
};

#[derive(Debug, Clone)]
pub struct Window {
    id: usize,
}

pub trait WindowManager: Send + Sync {
    fn create(&mut self, view: BoxView) -> Window;
    fn frame(&self, window: &Window) -> Frame;
    fn set_frame(&self, frame: Frame);
    fn close(&mut self, window: Window);
}

pub struct UnimplementedWindowManager;

impl WindowManager for UnimplementedWindowManager {
    fn create(&mut self, _view: BoxView) -> Window {
        todo!()
    }

    fn set_frame(&self, _frame: Frame) {
        todo!()
    }

    fn frame(&self, _window: &Window) -> Frame {
        todo!()
    }

    fn close(&mut self, _window: Window) {
        todo!()
    }
}

pub static GLOBAL_MANAGER: RwLock<UnimplementedWindowManager> =
    RwLock::new(UnimplementedWindowManager);

impl Window {
    pub fn new(view: impl View + 'static) -> Self {
        GLOBAL_MANAGER.write().unwrap().create(view.into_boxed())
    }

    pub fn frame(&self) -> Frame {
        GLOBAL_MANAGER.read().unwrap().frame(self)
    }

    pub fn set_frame(&self, frame: Frame) {
        GLOBAL_MANAGER.write().unwrap().set_frame(frame)
    }

    pub fn close(self) {
        GLOBAL_MANAGER.write().unwrap().close(self);
    }
}
