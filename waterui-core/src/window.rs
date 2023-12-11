use crate::{
    ffi::{
        utils::ViewObject, waterui_close_window, waterui_create_window, waterui_window_closeable,
    },
    view::View,
};

#[derive(Debug, Clone)]
pub struct Window {
    id: usize,
}

impl Window {
    pub fn new(title: impl Into<String>, content: impl View + 'static) -> Self {
        let title = title.into();
        let content: Box<dyn View> = Box::new(content);
        let widget = ViewObject::from(content);
        unsafe { Self::from_raw(waterui_create_window(title.into(), widget)) }
    }

    pub fn id(&self) -> usize {
        self.id
    }

    pub fn disable_close(&self) {
        unsafe { waterui_window_closeable(self.id, false) }
    }

    unsafe fn from_raw(id: usize) -> Self {
        Self { id }
    }

    pub fn close(self) {
        unsafe { waterui_close_window(self.id) }
    }
}
