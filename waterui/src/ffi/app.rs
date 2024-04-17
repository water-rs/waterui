use super::{AnyView, Environment};

#[repr(C)]
pub struct App {
    content: AnyView,
    env: Environment,
}

impl From<crate::App> for App {
    fn from(value: crate::App) -> Self {
        Self {
            content: value._content.into(),
            env: value._env.into(),
        }
    }
}
