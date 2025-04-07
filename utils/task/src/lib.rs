//! # WaterUI Task
//!
//! A lightweight, no-std compatible framework for spawning and managing asynchronous tasks.
//! This crate provides a simple yet powerful API for task execution with platform-specific
//! optimizations.
//!
//! ## Core Features
//!
//! - **No-std compatible**: Works in environments without the standard library
//! - **Thread-safe task execution**: `Task<T>` handles for sending tasks between threads
//! - **Thread-local tasks**: `LocalTask<T>` for tasks that must remain on the same thread
//! - **Priority-based scheduling**: Control execution priority of your tasks
//! - **Main thread execution**: API for ensuring code runs on the main/UI thread
//! - **Timing utilities**: Delay execution with timers and sleep functions
//! - **Platform optimizations**: Uses platform-specific scheduling where available
//!
//! ## Task Types
//!
//! ### `Task<T>`
//!
//! A handle to a spawned asynchronous task that can be shared between threads:
//!
//! ```rust
//! # use waterui_task::Task;
//! # async fn perform_work() {}
//! # async fn background_work() {}
//! # async fn update_ui() {}
//! # use waterui_task::Priority;
//! // Create a task with default priority
//! let task = Task::new(async { perform_work() });
//!
//! // Create a task with background priority
//! let bg_task = Task::with_priority(async { background_work() }, Priority::Background);
//!
//! // Ensure a task runs on the main thread
//! let main_task = Task::on_main(async { update_ui() });
//! ```
//!
//! ### `LocalTask<T>`
//!
//! Similar to `Task<T>` but for futures that are not `Send` and must run on the same thread:
//!
//! ```rust
//! # use waterui_task::LocalTask;
//! # async fn use_thread_local_data() {}
//! // Execute a non-Send future
//! let local_task = LocalTask::new(async { use_thread_local_data() });
//! ```
//!
//! ## Main Thread Safety
//!
//! The `MainValue<T>` wrapper provides safe handling of values that must be accessed on the main thread:
//!
//! ```rust
//! # use waterui_task::MainValue;
//! # fn create_ui_element() -> u32 { 42 }
//! // Create a value that will be accessed only on the main thread
//! let ui_element = MainValue::new(create_ui_element());
//!
//! # async fn example(ui_element: MainValue<u32>) {
//! // All operations occur safely on the main thread
//! ui_element.handle(|elem| elem + 1).await;
//! # }
//! ```
//!
//! ## Timing Utilities
//!
//! ```rust
//! # use std::time::Duration;
//! # use waterui_task::timer::{Timer, sleep};
//! # async fn example() {
//! // Wait for a specific duration
//! Timer::after(Duration::from_secs(1)).await;
//!
//! // Sleep for a given number of seconds
//! sleep(5).await;
//! # }
//! ```
//!
//! ## Platform-Specific Implementations
//!
//! The crate provides optimized implementations for different platforms:
//!
//! - **Apple platforms**: Uses Grand Central Dispatch (GCD) for efficient background execution
//!
//! ## Usage Example
//!
//! ```rust
//! # use waterui_task::{task, timer::sleep};
//! async fn example() {
//!     // Spawn a background task
//!     let task = task(async {
//!         // Do some work
//!         println!("Working in the background");
//!         42
//!     });
//!
//!     // Wait for some time
//!     sleep(1).await;
//!
//!     // Get the result
//!     let result = task.await;
//!     println!("Task completed with result: {}", result);
//! }
//! ```
//!
//! This crate is part of the WaterUI framework but can be used independently in any async Rust
//! application that needs lightweight task management capabilities.

#![no_std]
extern crate alloc;

mod apple;
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
