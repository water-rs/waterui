use crate::view::ViewExt;
use waterui_view::{AnyView, Environment, View};
pub struct WithValue<T> {
    pub _content: AnyView,
    pub _value: T,
}

impl<T> WithValue<T> {
    pub fn new(content: impl View + 'static, value: T) -> Self {
        Self {
            _content: content.anyview(),
            _value: value,
        }
    }
}

impl<T> View for WithValue<T> {
    fn body(self, _env: Environment) -> impl View {
        panic!(
            "You cannot call `body` for a raw view, may you need to handle this view `{}` manually",
            core::any::type_name::<Self>()
        );
    }
}
