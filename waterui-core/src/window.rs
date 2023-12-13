use crate::{
    ffi::{
        utils::ViewObject, waterui_close_window, waterui_create_window, waterui_window_closeable,
    },
    view::IntoView,
};

#[derive(Debug, Clone)]
pub struct Window {
    id: usize,
}

impl Window {
    pub fn new(title: impl Into<String>, content: impl IntoView) -> Self {
        let title = title.into();
        let widget = ViewObject::from(content.into_boxed_view());
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
