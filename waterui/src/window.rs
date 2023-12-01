use crate::{
    ffi::{waterui_close_window, waterui_create_window, waterui_window_closeable},
    view::View,
    widget::Widget,
};

#[derive(Debug, Clone)]
pub struct Window {
    id: usize,
}

impl Window {
    pub fn new(title: impl Into<String>, view: impl View + 'static) -> Self {
        let title = title.into();
        let widget = Widget::from_view(view);
        unsafe { Self::from_raw(waterui_create_window(title.into(), widget.into())) }
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
