#![doc = include_str!("../README.md")]
#![no_std]
extern crate alloc;

mod apple;
mod local_value;
pub use local_value::{LocalValue, OnceValue};
mod main_value;
pub use main_value::MainValue;
pub mod timer;

use core::mem::ManuallyDrop;
use core::time::Duration;
pub use futures_lite::*;

#[cfg(target_vendor = "apple")]
type DefaultExecutor = apple::ApplePlatformExecutor;

trait PlatformExecutor {
    fn exec_main(f: impl FnOnce() + Send + 'static);
    fn exec(f: impl FnOnce() + Send + 'static, priority: Priority);

    fn exec_after(delay: Duration, f: impl FnOnce() + Send + 'static);
}

use async_task::Runnable;

/// Execution priority levels for tasks.
///
/// Controls how the task scheduler prioritizes the execution of different tasks.
#[derive(Debug, Default, Clone, Copy)]
pub enum Priority {
    /// Standard priority level for most tasks.
    #[default]
    Default,
    /// Lower priority for tasks that should yield to more important operations.
    Background,
}

/// A handle to a spawned asynchronous task that can be shared between threads.
///
/// Represents a future that will complete with the output of the spawned task.
/// The task is automatically scheduled for execution when created.
pub struct Task<T> {
    inner: ManuallyDrop<async_task::Task<T>>,
}

impl<T: Send + 'static> Task<T> {
    /// Creates a new task with default priority.
    ///
    /// # Parameters
    /// * `future` - The future to execute in the task
    ///
    /// # Returns
    /// A new `Task` handle that can be used to await the result
    pub fn new<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = T> + Send + 'static,
    {
        Self::with_priority(future, Priority::default())
    }

    /// Creates a new task with the specified priority.
    ///
    /// # Parameters
    /// * `future` - The future to execute in the task
    /// * `priority` - The execution priority for the task
    ///
    /// # Returns
    /// A new `Task` handle that can be used to await the result
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

    /// Schedules a task to run on the main thread.
    ///
    /// # Parameters
    /// * `future` - The future to execute in the task on the main thread
    ///
    /// # Returns
    /// A new `Task` handle that can be used to await the result
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

    /// Cancels the task and waits for it to stop.
    ///
    /// # Returns
    /// A future that resolves when the task has been cancelled
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

/// A handle to a thread-local asynchronous task.
///
/// Similar to `Task` but for tasks that are not thread-safe and must run
/// on the same thread where they were created.
pub struct LocalTask<T> {
    inner: ManuallyDrop<async_task::Task<T>>,
}

impl<T: 'static> LocalTask<T> {
    /// Creates a new thread-local task with default priority.
    ///
    /// # Parameters
    /// * `future` - The future to execute in the local task
    ///
    /// # Returns
    /// A new `LocalTask` handle that can be used to await the result
    pub fn new<Fut>(future: Fut) -> Self
    where
        Fut: Future<Output = T> + 'static,
    {
        Self::with_priority(future, Priority::default())
    }

    /// Creates a new thread-local task with the specified priority.
    ///
    /// # Parameters
    /// * `future` - The future to execute in the local task
    /// * `priority` - The execution priority for the task
    ///
    /// # Returns
    /// A new `LocalTask` handle that can be used to await the result
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

    /// Schedules a thread-local task to run on the main thread.
    ///
    /// # Parameters
    /// * `future` - The future to execute in the task on the main thread
    ///
    /// # Returns
    /// A new `LocalTask` handle that can be used to await the result
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

    /// Cancels the local task and waits for it to stop.
    ///
    /// # Returns
    /// A future that resolves when the task has been cancelled
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

/// Convenience function to create a new task with default priority.
///
/// # Parameters
/// * `fut` - The future to execute in the task
///
/// # Returns
/// A new `Task` handle that can be used to await the result
pub fn task<Fut>(fut: Fut) -> Task<Fut::Output>
where
    Fut: Future + Send + 'static,
    Fut::Output: Send,
{
    Task::new(fut)
}

/// Schedules a function to be executed on the main thread.
///
/// # Parameters
/// * `f` - The function to execute on the main thread
fn exec_main(f: impl FnOnce() + Send + 'static) {
    DefaultExecutor::exec_main(f);
}

/// Schedules a function to be executed with the specified priority.
///
/// # Parameters
/// * `f` - The function to execute
/// * `priority` - The execution priority for the function
fn exec(f: impl FnOnce() + Send + 'static, priority: Priority) {
    DefaultExecutor::exec(f, priority);
}

/// Schedules a function to be executed after a specified delay.
///
/// # Parameters
/// * `delay` - The duration to wait before executing the function
/// * `f` - The function to execute after the delay
fn exec_after(delay: Duration, f: impl FnOnce() + Send + 'static) {
    DefaultExecutor::exec_after(delay, f);
}
