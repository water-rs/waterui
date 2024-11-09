#![no_std]
extern crate alloc;

mod apple;
mod main_value;
pub use main_value::MainValue;
pub mod timer;

use core::mem::ManuallyDrop;

#[cfg(target_vendor = "apple")]
#[doc(inline)]
pub use apple::*;
use async_task::Runnable;
pub use futures_lite::*;

#[derive(Debug, Default, Clone, Copy)]
pub enum Priority {
    #[default]
    Default,
    Background,
}

pub struct Task<T> {
    inner: ManuallyDrop<async_task::Task<T>>,
}

impl<T: Send + 'static> Task<T> {
    pub fn new<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = T> + Send + 'static,
    {
        Self::with_priority(future, Priority::default())
    }

    pub fn with_priority<Fut>(future: Fut, priority: Priority) -> Self
    where
        Fut: Future<Output = T> + Send + 'static,
    {
        let (runnable, task) = async_task::spawn(future, move |runnable: Runnable| {
            exec(
                move || {
                    runnable.run();
                },
                priority,
            );
        });

        exec(
            move || {
                runnable.run();
            },
            priority,
        );
        Self {
            inner: ManuallyDrop::new(task),
        }
    }

    pub fn on_main<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = T> + Send + 'static,
    {
        let (runnable, task) = async_task::spawn(future, move |runnable: Runnable| {
            exec_main(move || {
                runnable.run();
            });
        });

        exec_main(move || {
            runnable.run();
        });
        Self {
            inner: ManuallyDrop::new(task),
        }
    }

    pub async fn cancel(self) {
        ManuallyDrop::into_inner(self.inner).cancel().await;
    }
}

impl<T> Future for Task<T> {
    type Output = T;
    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        self.inner.poll(cx)
    }
}

pub struct LocalTask<T> {
    inner: ManuallyDrop<async_task::Task<T>>,
}

impl<T: 'static> LocalTask<T> {
    pub fn new<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = T> + 'static,
    {
        Self::with_priority(future, Priority::default())
    }

    pub fn with_priority<Fut>(future: Fut, priority: Priority) -> Self
    where
        Fut: Future<Output = T> + 'static,
    {
        let (runnable, task) = async_task::spawn_local(future, move |runnable: Runnable| {
            exec(
                move || {
                    runnable.run();
                },
                priority,
            );
        });

        exec(
            move || {
                runnable.run();
            },
            priority,
        );
        Self {
            inner: ManuallyDrop::new(task),
        }
    }

    pub fn on_main<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = T> + 'static,
    {
        let (runnable, task) = async_task::spawn_local(future, move |runnable: Runnable| {
            exec_main(move || {
                runnable.run();
            });
        });

        exec_main(move || {
            runnable.run();
        });
        Self {
            inner: ManuallyDrop::new(task),
        }
    }

    pub async fn cancel(self) {
        ManuallyDrop::into_inner(self.inner).cancel().await;
    }
}

impl<T> Future for LocalTask<T> {
    type Output = T;
    fn poll(
        mut self: core::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> core::task::Poll<Self::Output> {
        self.inner.poll(cx)
    }
}

pub fn task<Fut>(fut: Fut) -> Task<Fut::Output>
where
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    Task::new(fut)
}
