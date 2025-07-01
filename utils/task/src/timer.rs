//! Timer functionality for async operations.
//!
//! This module provides utilities for creating timed futures that resolve after a specified duration.
//! It's useful for implementing delays, timeouts, and other time-based operations in async code.

use core::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicBool, Ordering},
    task::{Context, Poll},
    time::Duration,
};

use alloc::sync::Arc;

use crate::exec_after;

/// A future that completes after a specified duration has elapsed.
///
/// A timer that can be awaited as a `Future` to introduce delays in async code.
///
/// This struct implements the `Future` trait and can be awaited in async contexts
/// to pause execution for the given duration.
#[derive(Debug)]
pub struct Timer {
    /// The duration to wait. This is taken (set to None) after the timer is started.
    duration: Option<Duration>,
    /// Atomic flag to track whether the timer has completed.
    /// This is shared between the future and the callback that will be executed after the duration.
    finished: Arc<AtomicBool>,
}

impl Timer {
    /// Creates a new `Timer` that will complete after the specified duration.
    ///
    /// # Arguments
    ///
    /// * `duration` - The amount of time to wait before the timer completes.
    ///
    /// # Returns
    ///
    /// A new `Timer` instance that can be awaited.
    ///
    /// # Example
    ///
    /// ```
    /// use std::time::Duration;
    ///
    /// async fn example() {
    ///     // Wait for 1 second
    ///     Timer::after(Duration::from_secs(1)).await;
    ///     println!("One second has passed!");
    /// }
    /// ```
    #[must_use]
    pub fn after(duration: Duration) -> Self {
        Self {
            duration: Some(duration),
            finished: Arc::default(),
        }
    }

    /// Creates a new `Timer` that will complete after the specified number of seconds.
    ///
    /// This is a convenience method that wraps `Timer::after` with `Duration::from_secs`.
    ///
    /// # Arguments
    ///
    /// * `secs` - The number of seconds to wait before the timer completes.
    ///
    /// # Returns
    ///
    /// A new `Timer` instance that can be awaited.
    ///
    /// # Example
    ///
    /// ```
    /// async fn example() {
    ///     // Wait for 5 seconds
    ///     Timer::after_secs(5).await;
    ///     println!("Five seconds have passed!");
    /// }
    /// ```
    #[must_use]
    pub fn after_secs(secs: u64) -> Self {
        Self::after(Duration::from_secs(secs))
    }
}

impl Future for Timer {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // If the timer has already finished, return Ready
        if self.finished.load(Ordering::Acquire) {
            return Poll::Ready(());
        }

        // If this is the first poll, set up the timer
        if let Some(duration) = self.duration.take() {
            let waker = cx.waker().clone();
            let finished = self.finished.clone();

            // Schedule the callback to run after the specified duration
            exec_after(duration, move || {
                // Mark the timer as finished
                finished.store(true, Ordering::Release);
                // Wake the task that's waiting on this timer
                waker.wake();
            });
        }

        // The timer hasn't completed yet
        Poll::Pending
    }
}

/// Suspends the current async task for the specified number of seconds.
///
/// This is a convenience wrapper around `Timer::after`.
///
/// # Arguments
///
/// * `secs` - The number of seconds to sleep.
///
/// # Example
///
/// ```
/// async fn example() {
///     println!("Going to sleep");
///     sleep(2).await;
///     println!("Woke up after 2 seconds");
/// }
/// ```
pub async fn sleep(secs: u64) {
    Timer::after(Duration::from_secs(secs)).await;
}
