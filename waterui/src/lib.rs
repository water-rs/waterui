pub mod view;
pub use view::{BoxView, View, ViewExt};
pub mod widget;
pub use waterui_core::{
    attributed_string::AttributedString, binding, ffi, layout, modifier::Modifier, Binding,
};
pub use waterui_derive::view;

mod task {
    use std::future::Future;

    use pin_project_lite::pin_project;
    use smol::LocalExecutor;

    pin_project! {
        pub struct Task<T> {
            #[pin]
            inner: smol::Task<T>,
        }
    }

    impl<T> Future for Task<T> {
        type Output = T;

        fn poll(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Self::Output> {
            self.project().inner.poll(cx)
        }
    }

    thread_local! {
        static EXECUTOR:LocalExecutor<'static>=LocalExecutor::new();
    }

    pub fn task<Fut>(future: Fut) -> Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
        Fut::Output: 'static,
    {
        Task {
            inner: EXECUTOR.with(move |executor| executor.spawn(future)),
        }
    }
}

pub use task::{task, Task};
