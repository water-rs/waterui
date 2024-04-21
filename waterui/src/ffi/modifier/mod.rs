use crate::modifier;

use super::AnyView;

mod padding;

#[repr(C)]
pub struct WithValue<T> {
    value: T,
    content: AnyView,
}

impl<T> From<modifier::WithValue<T>> for WithValue<T> {
    fn from(value: modifier::WithValue<T>) -> Self {
        Self {
            value: value._value,
            content: value._content.into(),
        }
    }
}
