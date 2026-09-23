//! Cooperative single-threaded executor for the embedded reactive runtime.
//!
//! `WaterUI`'s reactivity (text i18n resolution, `Binding`/`Computed`
//! watchers) spawns local async tasks. On embedded targets the
//! `native-executor` polyfill backend runs on a dedicated thread and asserts
//! that `spawn_local` is called from it — incompatible with dew's
//! single-threaded `Rc`-based rendering, which spawns from the render thread.
//!
//! This module installs a cooperative [`async_executor::LocalExecutor`] as the
//! `executor-core` local executor instead: `spawn_local` detaches the task
//! into the executor (no thread assertion), and [`tick`] drives all ready
//! tasks on the render thread between frames. It is the embedded counterpart
//! of the host event loop driving the executor.

use std::cell::RefCell;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use async_executor::LocalExecutor;
use executor_core::Task;

/// `executor-core`'s task error type (its `Error` alias is private, so the
/// concrete type is spelled out here to match the `Task` trait signature).
type TaskError = Box<dyn core::any::Any + Send>;

thread_local! {
    /// The cooperative executor for this (render) thread, driven by [`tick`].
    static RENDER_EXECUTOR: RefCell<Option<Rc<LocalExecutor<'static>>>> =
        const { RefCell::new(None) };
}

/// Handle returned from `spawn_local`.
///
/// dew spawns reactive watchers and never awaits their handles (the watch
/// guard owns the subscription), so this handle only needs to satisfy the
/// `Task` bound; it never resolves and ticking drives the detached task.
struct DetachedTask<T>(PhantomData<fn() -> T>);

impl<T> core::future::Future for DetachedTask<T> {
    type Output = T;
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<T> {
        Poll::Pending
    }
}

impl<T> Task<T> for DetachedTask<T> {
    fn poll_result(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<T, TaskError>> {
        Poll::Pending
    }
}

/// A cloneable handle to the render thread's cooperative executor.
#[derive(Clone)]
struct RenderExecutor {
    inner: Rc<LocalExecutor<'static>>,
}

impl executor_core::LocalExecutor for RenderExecutor {
    type Task<T: 'static> = DetachedTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: core::future::Future + 'static,
    {
        self.inner.spawn(fut).detach();
        DetachedTask(PhantomData)
    }
}

/// Installs the cooperative executor as the `executor-core` local executor for
/// this thread.
///
/// Idempotent: a second call (or a previously-set executor) is left in place.
pub fn install() {
    let exec = Rc::new(LocalExecutor::new());
    RENDER_EXECUTOR.with(|cell| {
        if cell.borrow().is_none() {
            *cell.borrow_mut() = Some(Rc::clone(&exec));
        }
    });
    let _ = executor_core::try_init_local_executor(RenderExecutor { inner: exec });
}

/// Drives every ready task on the render thread's cooperative executor,
/// returning the number of tasks polled. Call once per frame after rendering
/// so reactive watchers make progress.
pub fn tick() -> usize {
    RENDER_EXECUTOR.with(|cell| {
        let borrow = cell.borrow();
        let Some(exec) = borrow.as_ref() else {
            return 0;
        };
        let mut polled = 0;
        while exec.try_tick() {
            polled += 1;
        }
        polled
    })
}
