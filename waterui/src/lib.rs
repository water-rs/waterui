pub mod view;
pub use view::{BoxView, View, ViewExt};
pub mod widget;
pub use waterui_core::{
    attributed_string::AttributedString, ffi, layout, modifier::Modifier, Reactive,
};
pub use waterui_derive::view;

mod task {
    use std::{future::Future, sync::Arc};

    use pin_project_lite::pin_project;
    use smol::{spawn, LocalExecutor};

    pub fn task<Fut>(future: Fut)
    where
        Fut: std::future::Future + Send + 'static,
        Fut::Output: Send + 'static,
    {
        spawn(future).detach()
    }
}

pub use task::task;
